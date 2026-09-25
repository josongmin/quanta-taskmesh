//! Exhaustive concurrency model-check of the **real** governor (ADR 9000 / P6,
//! ADR 0003 D-loom).
//!
//! Under `--cfg loom` the engine's `sync` seam builds `Governor` on
//! `loom::sync::Mutex` and loom atomics, so every model below drives the
//! production `admit` / `claim` / `abandon` / `release` / `reap_leaks` /
//! `reconcile_memory` transitions — including the effects the governor drains
//! *outside* its lock — and loom explores every interleaving of the lock
//! acquisitions those transitions make. There is no hand-written replica of the
//! locking design here; a replica only proves the replica.
//!
//! State is kept tiny on purpose (two or three threads, one or two lock
//! acquisitions each) so the search is exhaustive rather than bounded.
//!
//! Run with: `just loom` (`RUSTFLAGS="--cfg loom" cargo test -p taskmesh-engine --features loom --test loom_governance --release`).

#![cfg(loom)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use loom::thread;
use taskmesh_contract::{
    ClassPolicy, ManualClock, MemoryPermitMode, MemoryReleasePolicy, OverflowPolicy, PermitWaker,
    ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, ClaimOutcome, Governor, PermitId, PolicySet, ReleaseOutcome, TerminalReason,
    Ticket,
};

const CLASS: &str = "c";
const LOOM_MAX_THREADS: usize = 5;
const LOOM_MAX_BRANCHES: usize = 1_000;

fn check_loom_model<F>(model_id: &'static str, model: F)
where
    F: Fn() + Sync + Send + 'static,
{
    let completed = Arc::new(AtomicUsize::new(0));
    let completed_in_model = Arc::clone(&completed);
    let mut builder = loom::model::Builder::new();
    builder.max_threads = LOOM_MAX_THREADS;
    builder.max_branches = LOOM_MAX_BRANCHES;
    builder.max_permutations = None;
    builder.max_duration = None;
    builder.preemption_bound = None;
    builder.check(move || {
        model();
        completed_in_model.fetch_add(1, Ordering::SeqCst);
    });
    let completed = completed.load(Ordering::SeqCst);
    assert!(
        completed > 0,
        "loom model {model_id} explored no permutations"
    );
    println!(
        "taskmesh-model-witness checker=loom model_id={model_id} \
         max_threads={LOOM_MAX_THREADS} max_branches={LOOM_MAX_BRANCHES} \
         max_permutations=exhaustive preemption_bound=unbounded completed={completed}"
    );
}

fn class() -> TaskClass {
    TaskClass::new(CLASS)
}

fn spec(op: &str) -> TaskSpec {
    TaskSpec::io(class()).operation(op.to_string())
}

/// A governor with one class: `max_inflight` slots, queue depth 4, leak
/// detection on, measured memory so `reconcile_memory` has an effect.
fn governor(max_inflight: u32, clock: Arc<ManualClock>) -> Arc<Governor> {
    let mut classes = BTreeMap::new();
    classes.insert(
        class(),
        ClassPolicy::new()
            .max_inflight(max_inflight)
            .max_queue_depth(4)
            .cpu_units(1)
            .memory_units(4)
            .memory_permit_mode(MemoryPermitMode::Measured)
            .memory_release_policy(MemoryReleasePolicy::LeakDetecting)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    );
    let policy = PolicySet::new(
        ResourceBudget::new()
            .cpu_units(64)
            .memory_units(64)
            .memory_unit_scale(1),
        classes,
    );
    Arc::new(Governor::new(policy, clock).expect("valid policy"))
}

fn admit(g: &Governor, op: &str) -> PermitId {
    match g.admit(&spec(op)) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    }
}

/// A waker that only counts. Whether it fired is the observable the lost-wakeup
/// model needs; it must not touch the governor (the real host waker does not).
struct CountingWaker(AtomicUsize);

impl PermitWaker for CountingWaker {
    fn wake(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn queue_with_waker(g: &Governor, op: &str) -> (Ticket, Arc<CountingWaker>) {
    let waker = Arc::new(CountingWaker(AtomicUsize::new(0)));
    let port: Arc<dyn PermitWaker> = waker.clone();
    match g.admit_waitable(&spec(op), port) {
        AdmissionDecision::Queued { ticket } => (ticket, waker),
        other => panic!("expected a queued ticket, got {other:?}"),
    }
}

/// Every model ends here: nothing inflight, nothing queued, the incremental
/// gauges agree with the totals, and every admission was terminated exactly
/// once (`admitted_total == terminated_total`).
fn assert_quiescent(g: &Governor) {
    let snapshot = g.snapshot();
    let c = &snapshot.classes[&class()];
    assert_eq!(c.inflight, 0, "inflight must return to zero");
    assert_eq!(c.queued, 0, "nothing may remain queued");
    assert_eq!(c.cpu_units_held, 0);
    assert_eq!(c.memory_units_held, 0);
    assert_eq!(
        c.admitted_total, c.terminated_total,
        "every admission is terminated exactly once"
    );
    assert_eq!(snapshot.conservation_violation(), None);
    assert!(g.permit_ledgers().is_empty(), "no permit may leak");
    assert_eq!(g.accounting_fault(), None);
}

#[test]
fn concurrent_admits_and_releases_conserve_capacity() {
    check_loom_model("loom.concurrent_admits_releases.v1", || {
        let g = governor(2, Arc::new(ManualClock::new(1_000)));
        let handles: Vec<_> = (0..2)
            .map(|i| {
                let g = Arc::clone(&g);
                thread::spawn(move || {
                    let permit = admit(&g, &format!("op{i}"));
                    assert_eq!(g.release(permit), ReleaseOutcome::Released);
                    permit
                })
            })
            .collect();
        let ids: Vec<PermitId> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_ne!(ids[0], ids[1], "permit ids are unique across threads");
        assert_quiescent(&g);
    });
}

/// The three-way race the host `TicketGuard` must survive, on the real engine:
/// a queued ticket is concurrently promoted (by the releasing holder), claimed
/// (by its waiter), and abandoned (cancel / acquire-timeout). Whatever the
/// interleaving, the permit is accounted exactly once — never leaked, never
/// double-freed, never stranded promoted-but-unclaimed — and the two racers
/// never both believe they own it.
#[test]
fn promote_claim_abandon_three_way_is_exactly_once() {
    check_loom_model("loom.promote_claim_abandon.v1", || {
        let g = governor(1, Arc::new(ManualClock::new(1_000)));
        let holder = admit(&g, "holder");
        let (ticket, _waker) = queue_with_waker(&g, "queued");

        let releaser = {
            let g = Arc::clone(&g);
            thread::spawn(move || assert_eq!(g.release(holder), ReleaseOutcome::Released))
        };
        let claimer = {
            let g = Arc::clone(&g);
            thread::spawn(move || match g.claim(ticket) {
                ClaimOutcome::Ready(permit) => {
                    // Ownership transferred to this thread: it releases, and
                    // that release must find the permit (abandon may not have
                    // taken it from under a claimer).
                    assert_eq!(g.release(permit), ReleaseOutcome::Released);
                    true
                }
                ClaimOutcome::Pending
                | ClaimOutcome::Terminal(TerminalReason::Abandoned)
                | ClaimOutcome::Invalid => false,
                other => panic!("unexpected claim outcome {other:?}"),
            })
        };
        let abandoner = {
            let g = Arc::clone(&g);
            thread::spawn(move || g.abandon(ticket))
        };
        releaser.join().unwrap();
        let claimed = claimer.join().unwrap();
        let _abandon_outcome = abandoner.join().unwrap();

        // Whoever ended the ticket, it is over: a late look never says "wait".
        assert!(
            matches!(
                g.ticket_status(ticket),
                ClaimOutcome::Invalid | ClaimOutcome::Terminal(_)
            ),
            "ticket must be terminal after the race (claimed = {claimed})"
        );
        // If the claimer never got it, the abandon (or the claim seeing a
        // terminal record) must have unwound the promotion.
        assert_quiescent(&g);
    });
}

/// Lost-wakeup freedom on the real engine. The waiter's protocol is
/// `claim → Pending → park`; the promoter's is `promote (under lock) → wake
/// (after lock)`. If the waiter observed `Pending`, the promotion had not
/// committed, so the promoter's wake is still ahead of it and must arrive:
/// there is no interleaving in which `Pending` is followed by silence.
#[test]
fn a_pending_claim_is_always_followed_by_a_wake() {
    check_loom_model("loom.pending_claim_wakeup.v1", || {
        let g = governor(1, Arc::new(ManualClock::new(1_000)));
        let holder = admit(&g, "holder");
        let (ticket, waker) = queue_with_waker(&g, "queued");

        let promoter = {
            let g = Arc::clone(&g);
            thread::spawn(move || assert_eq!(g.release(holder), ReleaseOutcome::Released))
        };
        let waiter = {
            let g = Arc::clone(&g);
            thread::spawn(move || g.claim(ticket))
        };
        promoter.join().unwrap();
        let first_look = waiter.join().unwrap();

        match first_look {
            ClaimOutcome::Pending => {
                assert_eq!(
                    waker.0.load(Ordering::SeqCst),
                    1,
                    "a waiter that saw Pending was promoted afterwards and must be woken"
                );
                // The wake means "look again": now it is ready.
                let ClaimOutcome::Ready(permit) = g.claim(ticket) else {
                    panic!("woken waiter must find its permit");
                };
                assert_eq!(g.release(permit), ReleaseOutcome::Released);
            }
            ClaimOutcome::Ready(permit) => {
                // The claim beat the wake or followed it; either way the waker is
                // spent (at most one fire) and the claimer owns the permit.
                assert!(waker.0.load(Ordering::SeqCst) <= 1);
                assert_eq!(g.release(permit), ReleaseOutcome::Released);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_quiescent(&g);
    });
}

/// The stale-lease race: a claimer takes ownership and moves to `Accepted`
/// while the leak sweep, on another thread, reclaims every `DispatchReserved`
/// lease older than zero. Exactly one of them wins the permit, both learn the
/// truth through typed outcomes, and the accounting closes either way.
#[test]
fn claim_and_reap_of_a_stale_promotion_fail_closed_both_ways() {
    check_loom_model("loom.claim_reap_stale_promotion.v1", || {
        let clock = Arc::new(ManualClock::new(1_000));
        let g = governor(1, clock.clone());
        let holder = admit(&g, "holder");
        let (ticket, _waker) = queue_with_waker(&g, "queued");
        assert_eq!(g.release(holder), ReleaseOutcome::Released); // promoted at 1000
        clock.set(1_002); // the promotion is now stale for a 1ms window

        let claimer = {
            let g = Arc::clone(&g);
            thread::spawn(move || match g.claim(ticket) {
                ClaimOutcome::Ready(permit) => {
                    // The claim touched the lease (t=1002), so a sweep that runs
                    // after it sees a fresh lease and must leave it alone.
                    let taskmesh_engine::AdvanceOutcome::Leased(lease) =
                        g.advance_phase(permit, taskmesh_contract::ExecutionPhase::Accepted)
                    else {
                        panic!("a claimed, touched lease is live and leases on its first advance");
                    };
                    // Dispatched: the sweep can no longer take it, and only
                    // its lease can end it.
                    assert_eq!(
                        g.release(permit),
                        ReleaseOutcome::HeldByLease {
                            phase: taskmesh_contract::ExecutionPhase::Accepted
                        }
                    );
                    assert_eq!(g.release_leased(lease), ReleaseOutcome::Released);
                    Some(permit)
                }
                ClaimOutcome::Terminal(TerminalReason::Reclaimed) => None,
                other => panic!("unexpected {other:?}"),
            })
        };
        let reaper = {
            let g = Arc::clone(&g);
            thread::spawn(move || g.reap_leaks_with(1))
        };
        let claimed = claimer.join().unwrap();
        let report = reaper.join().unwrap();

        match claimed {
            Some(_) => assert_eq!(
                report.reclaimed_permits, 0,
                "the claim won: the sweep saw a touched lease"
            ),
            None => assert_eq!(
                report.reclaimed_permits, 1,
                "the sweep won: the claimer was told Reclaimed"
            ),
        }
        assert_quiescent(&g);
    });
}

/// Two reporters reconcile the same permit without agreeing on an order. Each
/// reading is assigned its epoch inside the transition that applies it, so
/// both land: neither is rejected as stale for having read a "next" epoch
/// under an earlier lock.
#[test]
fn unordered_concurrent_reconciles_both_apply() {
    check_loom_model("loom.concurrent_reconcile.v1", || {
        let g = governor(2, Arc::new(ManualClock::new(1_000)));
        let permit = admit(&g, "measured");
        let reporters: Vec<_> = [2u64, 3]
            .into_iter()
            .map(|bytes| {
                let g = Arc::clone(&g);
                thread::spawn(move || g.reconcile_memory(permit, bytes))
            })
            .collect();
        for reporter in reporters {
            assert!(
                reporter.join().unwrap().is_applied(),
                "every reading is applied"
            );
        }
        let ledger = g.permit_ledger(permit).expect("live");
        assert_eq!(ledger.measurement_epoch, 2, "two readings, two epochs");
        assert!(
            ledger.effective_units == 2 || ledger.effective_units == 3,
            "the last reading applied is one of the two"
        );
        assert_eq!(g.release(permit), ReleaseOutcome::Released);
        assert_quiescent(&g);
    });
}
