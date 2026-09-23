//! Open-loop latency recording and the discrete-event admission simulator
//! (ADR 9000 / P1).
//!
//! # Two recorders, two populations
//!
//! Coordinated-omission correction (`hdrhistogram::record_correct`) exists for
//! *closed-loop* measurement: when the measuring loop itself stalls, the requests
//! it should have issued during the stall never happen, and back-filling them
//! keeps the tail honest. The open-loop simulator below has no such gap — every
//! arrival in the schedule is offered at its scheduled time whether or not the
//! previous one has finished. Applying the correction there does not recover
//! missing requests; it invents samples, weighted towards the requests that
//! waited longest, and the reported quantile stops describing the population of
//! requests that were actually admitted.
//!
//! So the simulator records **one raw sample per started request**, and the
//! corrected recorder is a distinct mode for callers that really do have
//! omission. The two are never mixed in one histogram.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::sync::Barrier;
use std::thread;
use std::time::{Duration, Instant};

use hdrhistogram::Histogram;
use taskmesh_contract::ClassPolicy;
use taskmesh_engine::{AdmissionDecision, ClaimOutcome, PermitId, ReleaseOutcome};

use crate::workload::{fixture, root_spec, Fixture, ValidatedArrivals, WorkloadError};

/// How a [`LatencyRecorder`] treats each sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecorderMode {
    /// One histogram sample per `record` call. For open-loop populations where
    /// every request was actually offered.
    Raw,
    /// Coordinated-omission correction against a fixed send cadence. For
    /// closed-loop measurement where a stalled measuring loop *omitted*
    /// requests it should have sent.
    OmissionCorrected { expected_interval_ns: u64 },
}

/// Records latencies under an explicit [`RecorderMode`].
pub struct LatencyRecorder {
    hist: Histogram<u64>,
    mode: RecorderMode,
    /// `record` calls, regardless of how many histogram entries they produced.
    recorded_calls: u64,
}

impl LatencyRecorder {
    fn with_mode(mode: RecorderMode) -> Self {
        Self {
            // 3 significant figures, auto-resizing so large tails still record.
            hist: Histogram::new(3).expect("histogram"),
            mode,
            recorded_calls: 0,
        }
    }

    /// A raw recorder: exactly one histogram sample per recorded latency.
    pub fn raw() -> Self {
        Self::with_mode(RecorderMode::Raw)
    }

    /// A coordinated-omission-corrected recorder for closed-loop measurement.
    pub fn corrected(expected_interval_ns: u64) -> Self {
        Self::with_mode(RecorderMode::OmissionCorrected {
            expected_interval_ns: expected_interval_ns.max(1),
        })
    }

    pub fn mode(&self) -> RecorderMode {
        self.mode
    }

    /// Record a measured latency (ns). The histogram is auto-resizing across
    /// the complete `u64` value domain, so every call contributes a sample.
    pub fn record(&mut self, latency_ns: u64) {
        self.recorded_calls += 1;
        match self.mode {
            RecorderMode::Raw => self.hist.record(latency_ns),
            RecorderMode::OmissionCorrected {
                expected_interval_ns,
            } => self.hist.record_correct(latency_ns, expected_interval_ns),
        }
        .expect("auto-resizing u64 histogram accepts every u64 latency");
    }

    /// Compatibility metric. Auto-resizing makes refusal unreachable for the
    /// `u64` latency domain, so this is always zero.
    pub fn dropped(&self) -> u64 {
        0
    }

    /// Number of latencies handed to `record`. In `Raw` mode this equals
    /// `len()` minus `dropped()`; in corrected mode `len()` may be larger,
    /// and the difference is the synthetic population.
    pub fn recorded_calls(&self) -> u64 {
        self.recorded_calls
    }

    /// Histogram entries that were synthesized by omission correction. Always
    /// zero in `Raw` mode.
    pub fn synthetic_samples(&self) -> u64 {
        self.hist.len().saturating_sub(self.recorded_calls)
    }

    pub fn quantile(&self, q: f64) -> u64 {
        self.hist.value_at_quantile(q)
    }

    pub fn p50(&self) -> u64 {
        self.quantile(0.50)
    }
    pub fn p99(&self) -> u64 {
        self.quantile(0.99)
    }
    pub fn p999(&self) -> u64 {
        self.quantile(0.999)
    }
    /// Histogram entries, including any synthesized by correction.
    pub fn len(&self) -> u64 {
        self.hist.len()
    }
    pub fn is_empty(&self) -> bool {
        self.hist.is_empty()
    }
}

/// Outcome of an open-loop simulation over a governor.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SimResult {
    pub offered: usize,
    pub admitted_immediately: usize,
    pub queued_then_promoted: usize,
    pub rejected: usize,
    pub completed: usize,
    /// High-water mark of the queue across the whole run (bounded-queue check).
    pub max_queue_observed: usize,
    /// Tickets still queued (never promotable) when the simulation drained — they
    /// did not complete and did not reject. Surfaced explicitly so conservation
    /// (`offered = completed + rejected + leftover_queued`) always holds and no
    /// request silently vanishes.
    pub leftover_queued: usize,
}

impl SimResult {
    pub fn started(&self) -> usize {
        self.admitted_immediately + self.queued_then_promoted
    }

    /// Conservation identity that must hold for every run.
    pub fn is_conserved(&self) -> bool {
        self.offered == self.completed + self.rejected + self.leftover_queued
            && self.completed == self.started()
    }
}

/// A scheduled completion in the discrete-event simulation.
type Completion = (u64, PermitId); // (completion_time_ns, permit)

const NANOS_PER_MILLI: u64 = 1_000_000;

fn virtual_millis(nanos: u64) -> u64 {
    nanos / NANOS_PER_MILLI
}

/// Open-loop discrete-event simulation of `arrivals` against `fx`'s governor
/// (ADR 9000 / P1 + P5). Each request holds a permit for `service_ns` of virtual
/// time; queued requests are promoted as inflight work completes. The recorded
/// latency is the *governance-induced admission wait* (start − arrival), not the
/// service time — that isolates the control-plane's contribution.
///
/// Latencies are recorded **raw**: one sample per started request (see the
/// module docs). Time is virtual and deterministic; the governor's
/// `ManualClock` is advanced to each event so retry/lease timestamps stay
/// consistent. All time arithmetic is checked.
pub fn simulate(
    fx: &Fixture,
    arrivals: &ValidatedArrivals,
    service_ns: u64,
) -> Result<(LatencyRecorder, SimResult), WorkloadError> {
    let g = &*fx.governor;
    let mut latency = LatencyRecorder::raw();
    let mut heap: BinaryHeap<Reverse<Completion>> = BinaryHeap::new();
    let mut pending: BTreeMap<u64, u64> = BTreeMap::new(); // ticket -> arrival_ns
    let mut res = SimResult {
        offered: arrivals.len(),
        ..Default::default()
    };

    let completion_at = |arrival_ns: u64| -> Result<u64, WorkloadError> {
        arrival_ns
            .checked_add(service_ns)
            .ok_or(WorkloadError::CompletionTimeOverflow {
                arrival_ns,
                service_ns,
            })
    };

    for (idx, (arrival, &t)) in arrivals
        .as_slice()
        .iter()
        .zip(arrivals.send_times_ns())
        .enumerate()
    {
        // Drain every completion that is due no later than this arrival, freeing
        // capacity and promoting queued work as we go.
        while let Some(&Reverse((ct, _))) = heap.peek() {
            if ct > t {
                break;
            }
            let Reverse((ct, permit)) = heap.pop().expect("peeked");
            fx.clock.set(virtual_millis(ct));
            assert_eq!(g.release(permit), ReleaseOutcome::Released);
            res.completed += 1;
            claim_promoted(
                fx,
                ct,
                &completion_at,
                &mut pending,
                &mut heap,
                &mut latency,
                &mut res,
            )?;
        }

        fx.clock.set(virtual_millis(t));
        let op = format!("op-{idx}");
        let spec = root_spec(&arrival.class, &op);
        match g.admit(&spec) {
            AdmissionDecision::Admitted { permit_id } => {
                res.admitted_immediately += 1;
                latency.record(0);
                heap.push(Reverse((completion_at(t)?, permit_id)));
            }
            AdmissionDecision::Queued { ticket } => {
                pending.insert(ticket, t);
                res.max_queue_observed = res.max_queue_observed.max(pending.len());
            }
            AdmissionDecision::Rejected(_) => {
                res.rejected += 1;
            }
        }
    }

    // Drain the tail: complete all remaining inflight work, promoting as we go.
    while let Some(Reverse((ct, permit))) = heap.pop() {
        fx.clock.set(virtual_millis(ct));
        assert_eq!(g.release(permit), ReleaseOutcome::Released);
        res.completed += 1;
        claim_promoted(
            fx,
            ct,
            &completion_at,
            &mut pending,
            &mut heap,
            &mut latency,
            &mut res,
        )?;
    }

    // Anything still queued was never promotable (e.g. a binding global budget):
    // account it explicitly so conservation holds and nothing vanishes silently.
    res.leftover_queued = pending.len();

    Ok((latency, res))
}

/// After a release at `now_ns`, claim every ticket the governor just promoted,
/// record its admission wait, and schedule its completion.
fn claim_promoted(
    fx: &Fixture,
    now_ns: u64,
    completion_at: &dyn Fn(u64) -> Result<u64, WorkloadError>,
    pending: &mut BTreeMap<u64, u64>,
    heap: &mut BinaryHeap<Reverse<Completion>>,
    latency: &mut LatencyRecorder,
    res: &mut SimResult,
) -> Result<(), WorkloadError> {
    loop {
        let mut promoted = None;
        for (&ticket, &arrival_ns) in pending.iter() {
            match fx.governor.claim(ticket) {
                ClaimOutcome::Ready(permit) => {
                    promoted = Some((ticket, arrival_ns, permit));
                    break;
                }
                ClaimOutcome::Pending => {}
                other => panic!("simulation ticket ended without service: {other:?}"),
            }
        }
        let Some((ticket, arrival_ns, permit)) = promoted else {
            break;
        };
        pending.remove(&ticket);
        res.queued_then_promoted += 1;
        latency.record(now_ns.saturating_sub(arrival_ns));
        heap.push(Reverse((completion_at(now_ns)?, permit)));
    }
    Ok(())
}

/// Process-global pool of per-worker key strings. Interned once and reused
/// across every contention run, so repeated sweeps (e.g. the USL probe) do not
/// leak a fresh string per call — total leak is bounded by the highest worker id
/// ever used.
fn worker_key(tid: usize) -> &'static str {
    use std::sync::Mutex;
    use std::sync::OnceLock;
    static POOL: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
    let pool = POOL.get_or_init(|| Mutex::new(Vec::new()));
    let mut pool = pool.lock().expect("key pool");
    let required_len = tid.checked_add(1).expect("worker id fits usize");
    let mut next = pool.len();
    pool.resize_with(required_len, || {
        let s: &'static str = Box::leak(format!("worker-{next}").into_boxed_str());
        next = next.checked_add(1).expect("worker key index fits usize");
        s
    });
    pool[tid]
}

/// One contention run: `threads` workers hammering one shared governor for
/// exactly `completed_ops` admit→release cycles in total.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentionRun {
    pub threads: usize,
    /// Cycles actually completed across all workers. Equal to the requested
    /// total by construction — the request is distributed exactly, remainder
    /// included.
    pub completed_ops: usize,
    /// Wall time of the op loop only. Thread creation and key preparation
    /// happen before a start barrier and are excluded.
    pub elapsed: Duration,
}

impl ContentionRun {
    pub fn throughput_per_sec(&self) -> f64 {
        let secs = self.elapsed.as_secs_f64();
        if secs <= 0.0 {
            return 0.0;
        }
        self.completed_ops as f64 / secs
    }
}

/// Split `total` ops across `threads` workers exactly: quotient each, remainder
/// distributed one per worker from the front. `Σ = total` always.
pub fn distribute_ops(total: usize, threads: usize) -> Vec<usize> {
    assert!(threads >= 1, "at least one worker");
    let base = total / threads;
    let extra = total % threads;
    (0..threads)
        .map(|tid| base + usize::from(tid < extra))
        .collect()
}

/// Run exactly `total_ops` admit→release cycles across `threads` workers on one
/// shared governor (ADR 9000 / P3 contention input for USL fitting).
///
/// The count is *exact*: a caller that asks for `iters` cycles gets a duration
/// for `iters` cycles, so a `Criterion::iter_custom` denominator matches the
/// work that was timed. Rounding `iters / threads` up or down silently changes
/// the per-op figure by up to `threads×` at small batch sizes.
///
/// Each thread reuses one interned `&'static str` key, so the per-op
/// `RequestKey` clone is allocation-free and the op loop measures pure lock +
/// grant contention. Workers rendezvous on a barrier before the clock starts,
/// so thread spawn latency is not inside the measured region.
pub fn contention_run(threads: usize, total_ops: usize) -> ContentionRun {
    assert!(threads >= 1, "at least one worker");
    // Unbounded class: cost 0 and budgets 0 (disabled checks) → admit always Ok.
    let fx = fixture(vec![("bench", ClassPolicy::new())], 0, 0);
    let g = fx.governor.clone();
    let shares = distribute_ops(total_ops, threads);
    assert_eq!(shares.len(), threads, "one exact share per worker");
    assert_eq!(
        shares.iter().sum::<usize>(),
        total_ops,
        "shares conserve ops"
    );

    // Derive the exact barrier cardinality without an open-ended worker-side
    // loop. A malformed count must fail immediately instead of leaving every
    // participant parked forever.
    let parties = threads
        .checked_add(1)
        .expect("contention participant count fits usize");
    let ready = Barrier::new(parties);
    let start_gate = Barrier::new(parties);
    let (elapsed, completed) = thread::scope(|scope| {
        let handles: Vec<_> = shares
            .iter()
            .enumerate()
            .map(|(tid, &share)| {
                let g = g.clone();
                let ready = &ready;
                let start_gate = &start_gate;
                scope.spawn(move || {
                    let op = worker_key(tid);
                    let spec = root_spec("bench", op);
                    ready.wait();
                    start_gate.wait();
                    let mut admitted = 0usize;
                    for _ in 0..share {
                        match g.admit(&spec) {
                            AdmissionDecision::Admitted { permit_id } => {
                                admitted += 1;
                                assert_eq!(g.release(permit_id), ReleaseOutcome::Released);
                            }
                            other => panic!("contention admit must succeed, got {other:?}"),
                        }
                    }
                    admitted
                })
            })
            .collect();
        // Phase one proves every worker is spawned. Phase two releases all
        // workers together after the measurement clock starts; no sequential
        // coordinator sends are part of the measured workload.
        ready.wait();
        let start = Instant::now();
        start_gate.wait();
        let completed: usize = handles
            .into_iter()
            .map(|h| h.join().expect("contention worker panicked"))
            .sum();
        (start.elapsed(), completed)
    });
    assert_eq!(
        completed, total_ops,
        "every requested op must run exactly once"
    );
    ContentionRun {
        threads,
        completed_ops: completed,
        elapsed,
    }
}

/// Aggregate admit→release throughput (ops/sec) for `threads` workers each
/// running `ops_per_thread` cycles. Convenience over [`contention_run`].
pub fn contention_throughput(threads: usize, ops_per_thread: usize) -> f64 {
    assert!(threads >= 1 && ops_per_thread >= 1);
    let total_ops = threads
        .checked_mul(ops_per_thread)
        .expect("requested contention operation count fits usize");
    contention_run(threads, total_ops).throughput_per_sec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correction_backfills_a_long_stall() {
        // expected one sample per 100ns; a single 10_000ns stall should inflate
        // the recorded count well beyond one via omission correction.
        let mut rec = LatencyRecorder::corrected(100);
        rec.record(10_000);
        assert!(
            rec.len() > 1,
            "omitted samples must be back-filled: {}",
            rec.len()
        );
        assert_eq!(rec.recorded_calls(), 1);
        assert!(rec.synthetic_samples() > 0);
        assert!(rec.p99() > 0);
    }

    #[test]
    fn raw_mode_records_exactly_one_sample_per_call() {
        let mut rec = LatencyRecorder::raw();
        for latency in [0u64, 9_999, 10_000, 1_000_000] {
            rec.record(latency);
        }
        assert_eq!(rec.len(), 4);
        assert_eq!(rec.recorded_calls(), 4);
        assert_eq!(rec.synthetic_samples(), 0);
        assert_eq!(rec.dropped(), 0);
    }

    #[test]
    fn recorder_accessors_observe_nonzero_quantiles_and_full_u64_domain() {
        let mut rec = LatencyRecorder::raw();
        assert!(rec.is_empty());
        for latency in [10u64, 20, 30] {
            rec.record(latency);
        }
        assert!(!rec.is_empty());
        assert!(rec.p50() >= 10);
        assert_eq!(rec.recorded_calls(), 3);

        rec.record(u64::MAX);
        assert_eq!(rec.recorded_calls(), 4);
        assert_eq!(rec.dropped(), 0);
        assert_eq!(rec.synthetic_samples(), 0);
    }

    #[test]
    fn conservation_rejects_each_independent_ledger_mismatch() {
        let valid = SimResult {
            offered: 3,
            admitted_immediately: 1,
            queued_then_promoted: 1,
            rejected: 1,
            completed: 2,
            max_queue_observed: 1,
            leftover_queued: 0,
        };
        assert!(valid.is_conserved());
        assert!(!SimResult {
            offered: 4,
            ..valid
        }
        .is_conserved());
        assert!(!SimResult {
            completed: 1,
            ..valid
        }
        .is_conserved());
        assert!(!SimResult {
            leftover_queued: 1,
            ..valid
        }
        .is_conserved());
        assert!(!SimResult {
            offered: 2,
            leftover_queued: 1,
            ..valid
        }
        .is_conserved());
    }

    #[test]
    fn throughput_handles_zero_time_and_has_exact_units() {
        let zero = ContentionRun {
            threads: 2,
            completed_ops: 4,
            elapsed: Duration::ZERO,
        };
        assert_eq!(zero.throughput_per_sec(), 0.0);
        let measured = ContentionRun {
            elapsed: Duration::from_secs(2),
            ..zero
        };
        assert_eq!(measured.throughput_per_sec(), 2.0);
    }

    #[test]
    fn virtual_clock_conversion_uses_floor_milliseconds() {
        assert_eq!(virtual_millis(0), 0);
        assert_eq!(virtual_millis(999_999), 0);
        assert_eq!(virtual_millis(1_000_000), 1);
        assert_eq!(virtual_millis(2_500_001), 2);
    }

    #[test]
    fn quantiles_are_ordered() {
        let mut rec = LatencyRecorder::corrected(1_000);
        for v in 1..=1000u64 {
            rec.record(v * 10);
        }
        assert!(rec.p50() <= rec.p99());
        assert!(rec.p99() <= rec.p999());
    }

    use crate::workload::{fixture, retrieval_policy, Arrival};

    /// A constant-rate open-loop schedule of `n` arrivals at `rate` req/sec.
    fn cadence(n: usize, rate: f64) -> ValidatedArrivals {
        let step = 1.0 / rate;
        ValidatedArrivals::new(
            (0..n)
                .map(|i| Arrival {
                    send_time_secs: i as f64 * step,
                    class: "retrieval".to_string(),
                })
                .collect(),
        )
        .expect("a cadence is a valid schedule")
    }

    // ---- TM16-029: the open-loop histogram is the admitted population ------

    #[test]
    fn open_loop_waits_are_recorded_exactly_once() {
        // Two arrivals 1ns apart, one inflight slot, 10µs service: the first
        // waits 0, the second waits 9_999ns. Two requests, two samples. The
        // corrected recorder produced ten here and reported a p50 of ~5µs that
        // described neither request.
        let fx = fixture(vec![("retrieval", retrieval_policy(1, 8))], 0, 0);
        let arrivals = ValidatedArrivals::new(vec![
            Arrival {
                send_time_secs: 0.0,
                class: "retrieval".into(),
            },
            Arrival {
                send_time_secs: 1.0e-9,
                class: "retrieval".into(),
            },
        ])
        .unwrap();
        let (lat, res) = simulate(&fx, &arrivals, 10_000).unwrap();
        assert_eq!(res.offered, 2);
        assert_eq!(res.completed, 2);
        assert_eq!(lat.mode(), RecorderMode::Raw);
        assert_eq!(lat.len(), 2, "one sample per admitted request");
        assert_eq!(lat.recorded_calls(), 2);
        assert_eq!(lat.synthetic_samples(), 0);
        assert_eq!(lat.dropped(), 0);
        // An independently recorded histogram agrees exactly.
        let mut independent = LatencyRecorder::raw();
        independent.record(0);
        independent.record(9_999);
        assert_eq!(lat.p50(), independent.p50());
        assert_eq!(lat.p99(), independent.p99());
    }

    #[test]
    fn completion_at_the_exact_arrival_boundary_frees_capacity_first() {
        let fx = fixture(vec![("retrieval", retrieval_policy(1, 8))], 0, 0);
        let arrivals = ValidatedArrivals::new(vec![
            Arrival {
                send_time_secs: 0.0,
                class: "retrieval".into(),
            },
            Arrival {
                send_time_secs: 10.0e-9,
                class: "retrieval".into(),
            },
        ])
        .expect("exact-boundary schedule");

        let (latency, result) = simulate(&fx, &arrivals, 10).expect("simulation");

        assert_eq!(result.admitted_immediately, 2);
        assert_eq!(result.queued_then_promoted, 0);
        assert_eq!(latency.p999(), 0);
        assert!(result.is_conserved());
    }

    #[test]
    fn histogram_population_equals_started_requests_under_overload() {
        let depth = 16;
        let fx = fixture(
            vec![("retrieval", retrieval_policy(1, depth))],
            1_000,
            1_000,
        );
        let arrivals = cadence(2_000, 100_000.0);
        let (lat, res) = simulate(&fx, &arrivals, 1_000_000).unwrap();
        assert!(res.rejected > 0, "overload rejects");
        assert_eq!(
            lat.len() as usize,
            res.started(),
            "the histogram is exactly the started population; rejected requests are counted apart"
        );
        assert_eq!(lat.synthetic_samples(), 0);
    }

    #[test]
    fn undersaturated_admits_immediately_and_completes_all() {
        // Capacity (32 inflight) >> offered concurrency: nothing should queue.
        let fx = fixture(vec![("retrieval", retrieval_policy(32, 128))], 1_000, 1_000);
        let arrivals = cadence(500, 1_000.0); // 1ms apart
        let service_ns = 100_000; // 0.1ms < interarrival → never backs up
        let (lat, res) = simulate(&fx, &arrivals, service_ns).unwrap();

        assert_eq!(res.offered, 500);
        assert_eq!(res.rejected, 0, "no rejects when undersaturated");
        assert_eq!(res.admitted_immediately, 500, "all admit without queueing");
        assert_eq!(res.max_queue_observed, 0);
        assert_eq!(res.leftover_queued, 0);
        assert!(res.is_conserved(), "{res:?}");
        assert_eq!(lat.dropped(), 0, "no latency samples dropped");
        assert_eq!(lat.len(), 500);
    }

    #[test]
    fn overload_stays_bounded_and_fails_closed() {
        // Tiny capacity, bursty arrivals, slow service → the queue must never
        // exceed max_queue_depth and excess offered load must be rejected.
        let depth = 16;
        let fx = fixture(
            vec![("retrieval", retrieval_policy(1, depth))],
            1_000,
            1_000,
        );
        let arrivals = cadence(2_000, 100_000.0); // 10µs apart: far above service rate
        let service_ns = 1_000_000; // 1ms service
        let (lat, res) = simulate(&fx, &arrivals, service_ns).unwrap();

        assert_eq!(res.offered, 2_000);
        assert!(res.rejected > 0, "overload must reject (fail-closed)");
        assert!(
            res.max_queue_observed <= depth as usize,
            "queue must stay bounded: {} > {depth}",
            res.max_queue_observed
        );
        assert!(res.is_conserved(), "conservation violated: {res:?}");
        assert_eq!(res.leftover_queued, 0, "single-class run must fully drain");
        assert_eq!(
            lat.dropped(),
            0,
            "no latency samples dropped under overload"
        );
    }

    #[test]
    fn completion_time_overflow_is_a_typed_error_not_a_wrap() {
        let fx = fixture(vec![("retrieval", retrieval_policy(1, 8))], 0, 0);
        // An arrival at t = 1s: `1e9 + u64::MAX` does not fit, and a wrapping
        // add would schedule the completion in the past.
        let arrivals = ValidatedArrivals::new(vec![Arrival {
            send_time_secs: 1.0,
            class: "retrieval".into(),
        }])
        .unwrap();
        let result = simulate(&fx, &arrivals, u64::MAX);
        assert!(
            matches!(result, Err(WorkloadError::CompletionTimeOverflow { .. })),
            "got {:?}",
            result.map(|(_, r)| r)
        );
    }

    // ---- TM16-036: the batch count is the denominator -----------------------

    #[test]
    fn ops_are_distributed_exactly() {
        for threads in [1usize, 2, 4, 8] {
            for total in [1usize, threads - 1, threads, threads + 1, 1_000, 1_001] {
                let shares = distribute_ops(total, threads);
                assert_eq!(shares.len(), threads);
                assert_eq!(
                    shares.iter().sum::<usize>(),
                    total,
                    "t={threads} total={total}"
                );
                let (min, max) = (
                    shares.iter().min().copied().unwrap(),
                    shares.iter().max().copied().unwrap(),
                );
                assert!(max - min <= 1, "balanced within one: {shares:?}");
            }
        }
    }

    #[test]
    fn a_contention_run_completes_exactly_the_requested_count() {
        for threads in [1usize, 2, 4, 8] {
            for total in [1usize, threads - 1, threads, threads + 1, 257] {
                if total == 0 {
                    continue;
                }
                let run = contention_run(threads, total);
                assert_eq!(run.completed_ops, total, "t={threads} total={total}");
                assert_eq!(run.threads, threads);
            }
        }
    }

    #[test]
    fn contention_throughput_math_and_convenience_smoke() {
        let known = ContentionRun {
            threads: 2,
            completed_ops: 4,
            elapsed: Duration::from_secs(2),
        };
        assert_eq!(known.throughput_per_sec(), 2.0);
        assert_eq!(
            ContentionRun {
                elapsed: Duration::ZERO,
                ..known
            }
            .throughput_per_sec(),
            0.0
        );

        // The real-thread convenience path only needs to prove that a positive
        // elapsed measurement reaches the rate calculation; no stress budget.
        let convenience = contention_throughput(2, 1);
        assert!(convenience.is_finite());
        assert!(convenience > 0.0, "{convenience}");
    }
}
