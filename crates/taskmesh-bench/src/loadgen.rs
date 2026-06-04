//! Open-loop, coordinated-omission-free latency recording (ADR 9000 / P1).
//!
//! When a measured latency exceeds the expected inter-arrival interval, the
//! requests that *should* have been sent during the stall are synthesized by
//! `hdrhistogram::record_correct` so the tail is not understated. Averages are
//! intentionally never exposed.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::thread;
use std::time::Instant;

use hdrhistogram::Histogram;
use taskmesh_contract::ClassPolicy;
use taskmesh_engine::{AdmissionDecision, PermitId};

use crate::workload::{fixture, root_spec, Arrival, Fixture};

/// Records latencies with coordinated-omission correction against a fixed
/// expected interval (the open-loop send cadence).
pub struct LatencyRecorder {
    hist: Histogram<u64>,
    expected_interval_ns: u64,
    /// Samples the histogram refused (would understate the tail if ignored).
    dropped: u64,
}

impl LatencyRecorder {
    pub fn new(expected_interval_ns: u64) -> Self {
        Self {
            // 3 significant figures, auto-resizing so large tails still record.
            hist: Histogram::new(3).expect("histogram"),
            expected_interval_ns: expected_interval_ns.max(1),
            dropped: 0,
        }
    }

    /// Record a measured latency (ns), back-filling omitted samples. A refused
    /// sample is counted, never silently dropped — `dropped()` must stay 0 for a
    /// trustworthy tail (asserted by callers/tests).
    pub fn record(&mut self, latency_ns: u64) {
        if self
            .hist
            .record_correct(latency_ns, self.expected_interval_ns)
            .is_err()
        {
            self.dropped += 1;
        }
    }

    /// Count of samples the histogram could not record. A non-zero value means
    /// the reported quantiles understate the true tail.
    pub fn dropped(&self) -> u64 {
        self.dropped
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

/// Open-loop discrete-event simulation of `arrivals` against `fx`'s governor
/// (ADR 9000 / P1 + P5). Each request holds a permit for `service_ns` of virtual
/// time; queued requests are promoted as inflight work completes. The recorded
/// latency is the *governance-induced admission wait* (start − arrival), not the
/// service time — that isolates the control-plane's contribution.
///
/// Latencies are recorded with coordinated-omission correction against
/// `mean_interval_ns`. Time is virtual and deterministic; the governor's
/// `ManualClock` is advanced to each event so retry/lease timestamps stay
/// consistent.
pub fn simulate(
    fx: &Fixture,
    arrivals: &[Arrival],
    service_ns: u64,
    mean_interval_ns: u64,
) -> (LatencyRecorder, SimResult) {
    let g = &*fx.governor;
    let mut latency = LatencyRecorder::new(mean_interval_ns);
    let mut heap: BinaryHeap<Reverse<Completion>> = BinaryHeap::new();
    let mut pending: BTreeMap<u64, u64> = BTreeMap::new(); // ticket -> arrival_ns
    let mut res = SimResult {
        offered: arrivals.len(),
        ..Default::default()
    };

    for (idx, arrival) in arrivals.iter().enumerate() {
        let t = (arrival.send_time_secs * 1.0e9) as u64;

        // Drain every completion that is due no later than this arrival, freeing
        // capacity and promoting queued work as we go.
        while let Some(&Reverse((ct, _))) = heap.peek() {
            if ct > t {
                break;
            }
            let Reverse((ct, permit)) = heap.pop().expect("peeked");
            fx.clock.set(ct / 1_000_000);
            g.release(permit);
            res.completed += 1;
            claim_promoted(
                fx,
                ct,
                service_ns,
                &mut pending,
                &mut heap,
                &mut latency,
                &mut res,
            );
        }

        fx.clock.set(t / 1_000_000);
        let op = format!("op-{idx}");
        let spec = root_spec(&arrival.class, &op);
        match g.admit(&spec) {
            AdmissionDecision::Admitted { permit_id } => {
                res.admitted_immediately += 1;
                latency.record(0);
                heap.push(Reverse((t + service_ns, permit_id)));
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
        fx.clock.set(ct / 1_000_000);
        g.release(permit);
        res.completed += 1;
        claim_promoted(
            fx,
            ct,
            service_ns,
            &mut pending,
            &mut heap,
            &mut latency,
            &mut res,
        );
    }

    // Anything still queued was never promotable (e.g. a binding global budget):
    // account it explicitly so conservation holds and nothing vanishes silently.
    res.leftover_queued = pending.len();

    (latency, res)
}

/// After a release at `now_ns`, claim every ticket the governor just promoted,
/// record its admission wait, and schedule its completion.
fn claim_promoted(
    fx: &Fixture,
    now_ns: u64,
    service_ns: u64,
    pending: &mut BTreeMap<u64, u64>,
    heap: &mut BinaryHeap<Reverse<Completion>>,
    latency: &mut LatencyRecorder,
    res: &mut SimResult,
) {
    loop {
        let mut promoted = None;
        for (&ticket, &arrival_ns) in pending.iter() {
            if let Some(permit) = fx.governor.claim(ticket) {
                promoted = Some((ticket, arrival_ns, permit));
                break;
            }
        }
        let Some((ticket, arrival_ns, permit)) = promoted else {
            break;
        };
        pending.remove(&ticket);
        res.queued_then_promoted += 1;
        latency.record(now_ns.saturating_sub(arrival_ns));
        heap.push(Reverse((now_ns + service_ns, permit)));
    }
}

/// Process-global pool of per-worker key strings. Interned once and reused
/// across every `contention_throughput` call, so repeated sweeps (e.g. the USL
/// probe) do not leak a fresh string per call — total leak is bounded by the
/// highest worker id ever used.
fn worker_key(tid: usize) -> &'static str {
    use std::sync::Mutex;
    use std::sync::OnceLock;
    static POOL: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
    let pool = POOL.get_or_init(|| Mutex::new(Vec::new()));
    let mut pool = pool.lock().expect("key pool");
    while pool.len() <= tid {
        let s: &'static str = Box::leak(format!("worker-{}", pool.len()).into_boxed_str());
        pool.push(s);
    }
    pool[tid]
}

/// Aggregate admit→release throughput (ops/sec) with `threads` workers hammering
/// one shared governor (ADR 9000 / P3 contention input for USL fitting).
///
/// Each thread reuses one interned `&'static str` key, so the per-op `RequestKey`
/// clone is allocation-free and the op loop measures pure lock + grant contention
/// (no key allocation). Keys are root-scoped, so the recursion guard is bypassed
/// entirely — uniqueness only keeps the request keys distinct, not the guard.
pub fn contention_throughput(threads: usize, ops_per_thread: usize) -> f64 {
    assert!(threads >= 1 && ops_per_thread >= 1);
    // Unbounded class: cost 0 and budgets 0 (disabled checks) → admit always Ok.
    let fx = fixture(vec![("bench", ClassPolicy::new())], 0, 0);
    let g = fx.governor.clone();

    let start = Instant::now();
    thread::scope(|scope| {
        for tid in 0..threads {
            let g = g.clone();
            scope.spawn(move || {
                let op = worker_key(tid);
                let spec = root_spec("bench", op);
                let mut admitted = 0u64;
                for _ in 0..ops_per_thread {
                    match g.admit(&spec) {
                        AdmissionDecision::Admitted { permit_id } => {
                            admitted += 1;
                            g.release(permit_id);
                        }
                        other => panic!("contention admit must succeed, got {other:?}"),
                    }
                }
                debug_assert_eq!(admitted as usize, ops_per_thread);
            });
        }
    });
    let elapsed = start.elapsed().as_secs_f64();
    (threads * ops_per_thread) as f64 / elapsed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correction_backfills_a_long_stall() {
        // expected one sample per 100ns; a single 10_000ns stall should inflate
        // the recorded count well beyond one via omission correction.
        let mut rec = LatencyRecorder::new(100);
        rec.record(10_000);
        assert!(
            rec.len() > 1,
            "omitted samples must be back-filled: {}",
            rec.len()
        );
        assert!(rec.p99() > 0);
    }

    #[test]
    fn quantiles_are_ordered() {
        let mut rec = LatencyRecorder::new(1_000);
        for v in 1..=1000u64 {
            rec.record(v * 10);
        }
        assert!(rec.p50() <= rec.p99());
        assert!(rec.p99() <= rec.p999());
    }

    use crate::workload::{fixture, retrieval_policy, Arrival};

    /// A constant-rate open-loop schedule of `n` arrivals at `rate` req/sec.
    fn cadence(n: usize, rate: f64) -> Vec<Arrival> {
        let step = 1.0 / rate;
        (0..n)
            .map(|i| Arrival {
                send_time_secs: i as f64 * step,
                class: "retrieval".to_string(),
            })
            .collect()
    }

    #[test]
    fn undersaturated_admits_immediately_and_completes_all() {
        // Capacity (32 inflight) >> offered concurrency: nothing should queue.
        let fx = fixture(vec![("retrieval", retrieval_policy(32, 128))], 1_000, 1_000);
        let arrivals = cadence(500, 1_000.0); // 1ms apart
        let service_ns = 100_000; // 0.1ms < interarrival → never backs up
        let (lat, res) = simulate(&fx, &arrivals, service_ns, 1_000_000);

        assert_eq!(res.offered, 500);
        assert_eq!(res.rejected, 0, "no rejects when undersaturated");
        assert_eq!(res.admitted_immediately, 500, "all admit without queueing");
        assert_eq!(res.max_queue_observed, 0);
        assert_eq!(res.leftover_queued, 0);
        assert!(res.is_conserved(), "{res:?}");
        assert_eq!(lat.dropped(), 0, "no latency samples dropped");
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
        let (lat, res) = simulate(&fx, &arrivals, service_ns, 10_000);

        assert_eq!(res.offered, 2_000);
        assert!(res.rejected > 0, "overload must reject (fail-closed)");
        assert!(
            res.max_queue_observed <= depth as usize,
            "queue must stay bounded: {} > {depth}",
            res.max_queue_observed
        );
        // Conservation: every offered request either rejects, completes, or is
        // still queued at the (drained) end — none vanish silently. A single
        // class with a free budget fully drains, so leftover is 0 here.
        assert!(res.is_conserved(), "conservation violated: {res:?}");
        assert_eq!(res.leftover_queued, 0, "single-class run must fully drain");
        assert_eq!(
            lat.dropped(),
            0,
            "no latency samples dropped under overload"
        );
    }

    #[test]
    fn contention_runner_reports_positive_throughput() {
        let tput = contention_throughput(2, 2_000);
        assert!(tput > 0.0, "throughput must be positive: {tput}");
    }
}
