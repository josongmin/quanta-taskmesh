//! Randomized concurrency model-check of the **real** governor (ADR 9000 / P6,
//! ADR 0003 D-loom).
//!
//! `loom_governance.rs` explores *every* interleaving but only for tiny state.
//! Under `--cfg shuttle` the same `sync` seam builds `Governor` on shuttle's
//! mutex, and these models sample thousands of schedules over larger state —
//! more threads, more operations, two racing abandoners —
//! on the production `admit` / `claim` / `abandon` / `release` / `reap_leaks`
//! transitions. No replica of the locking design: the thing checked is the
//! thing shipped.
//!
//! Run with: `just shuttle` (`RUSTFLAGS="--cfg shuttle" cargo test -p taskmesh-engine --features shuttle --test shuttle_governance --release`).

#![cfg(shuttle)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use shuttle::thread;
use taskmesh_contract::{
    ClassPolicy, ExecutionPhase, ManualClock, MemoryReleasePolicy, OverflowPolicy, PermitWaker,
    ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, ClaimOutcome, Governor, PermitId, PolicySet, ReleaseOutcome, TerminalReason,
};

const SCHEDULES: usize = 10_000;
const CLASS: &str = "c";

fn class() -> TaskClass {
    TaskClass::new(CLASS)
}

fn spec(op: &str) -> TaskSpec {
    TaskSpec::io(class()).operation(op.to_string())
}

fn governor(max_inflight: u32, queue_depth: u32, clock: Arc<ManualClock>) -> Arc<Governor> {
    let mut classes = BTreeMap::new();
    classes.insert(
        class(),
        ClassPolicy::new()
            .max_inflight(max_inflight)
            .max_queue_depth(queue_depth)
            .cpu_units(1)
            .memory_units(1)
            .memory_release_policy(MemoryReleasePolicy::LeakDetecting)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    );
    let policy = PolicySet::new(
        ResourceBudget::new().cpu_units(1_000).memory_units(1_000),
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

struct CountingWaker(AtomicUsize);

impl PermitWaker for CountingWaker {
    fn wake(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn queue_with_waker(g: &Governor, op: &str) -> (u64, Arc<CountingWaker>) {
    let waker = Arc::new(CountingWaker(AtomicUsize::new(0)));
    let port: Arc<dyn PermitWaker> = waker.clone();
    match g.admit_waitable(&spec(op), port) {
        AdmissionDecision::Queued { ticket } => (ticket, waker),
        other => panic!("expected a queued ticket, got {other:?}"),
    }
}

fn assert_quiescent(g: &Governor) {
    let snapshot = g.snapshot();
    let c = &snapshot.classes[&class()];
    assert_eq!(c.inflight, 0, "inflight must return to zero");
    assert_eq!(c.queued, 0, "nothing may remain queued");
    assert_eq!(c.cpu_units_held, 0);
    assert_eq!(c.memory_units_held, 0);
    assert_eq!(c.admitted_total, c.terminated_total);
    assert_eq!(snapshot.conservation_violation(), None);
    assert!(g.permit_ledgers().is_empty(), "no permit may leak");
    assert_eq!(g.accounting_fault(), None);
}

/// Four churners, each admitting and releasing twice, over a two-slot class:
/// admissions queue, releases promote, and the ledgers must close.
#[test]
fn randomized_admit_release_churn_conserves_capacity() {
    shuttle::check_random(
        || {
            let g = governor(2, 8, Arc::new(ManualClock::new(1_000)));
            let handles: Vec<_> = (0..4)
                .map(|i| {
                    let g = Arc::clone(&g);
                    thread::spawn(move || {
                        for round in 0..2 {
                            match g.admit(&spec(&format!("t{i}r{round}"))) {
                                AdmissionDecision::Admitted { permit_id } => {
                                    assert_eq!(g.release(permit_id), ReleaseOutcome::Released);
                                }
                                AdmissionDecision::Queued { ticket } => {
                                    // Spin on the real claim protocol until the
                                    // promotion lands (shuttle schedules the
                                    // releasers in between).
                                    loop {
                                        match g.claim(ticket) {
                                            ClaimOutcome::Ready(permit) => {
                                                assert_eq!(
                                                    g.release(permit),
                                                    ReleaseOutcome::Released
                                                );
                                                break;
                                            }
                                            ClaimOutcome::Pending => thread::yield_now(),
                                            other => panic!("unexpected {other:?}"),
                                        }
                                    }
                                }
                                other => panic!("unexpected {other:?}"),
                            }
                        }
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }
            assert_quiescent(&g);
        },
        SCHEDULES,
    );
}

/// The host `TicketGuard` handoff with *two* abandoners (cancel and
/// acquire-timeout firing together) racing a promoter and a claimer, on the
/// real engine. The permit is accounted exactly once; abandon is idempotent;
/// a claimer that won ownership is never robbed of it.
#[test]
fn randomized_promote_claim_double_abandon_is_exactly_once() {
    shuttle::check_random(
        || {
            let g = governor(1, 4, Arc::new(ManualClock::new(1_000)));
            let holder = admit(&g, "holder");
            let (ticket, _waker) = queue_with_waker(&g, "queued");

            let promoter = {
                let g = Arc::clone(&g);
                thread::spawn(move || assert_eq!(g.release(holder), ReleaseOutcome::Released))
            };
            let claimer = {
                let g = Arc::clone(&g);
                thread::spawn(move || match g.claim(ticket) {
                    ClaimOutcome::Ready(permit) => {
                        assert_eq!(g.release(permit), ReleaseOutcome::Released);
                    }
                    ClaimOutcome::Pending
                    | ClaimOutcome::Terminal(TerminalReason::Abandoned)
                    | ClaimOutcome::Invalid => {}
                    other => panic!("unexpected {other:?}"),
                })
            };
            let abandon = |g: Arc<Governor>| thread::spawn(move || g.abandon(ticket));
            let a1 = abandon(Arc::clone(&g));
            let a2 = abandon(Arc::clone(&g));
            promoter.join().unwrap();
            claimer.join().unwrap();
            a1.join().unwrap();
            a2.join().unwrap();

            assert!(matches!(
                g.ticket_status(ticket),
                ClaimOutcome::Invalid | ClaimOutcome::Terminal(_)
            ));
            assert_quiescent(&g);
        },
        SCHEDULES,
    );
}

/// Lost-wakeup freedom at scale: three waiters park on three tickets, three
/// holders release concurrently. Every waiter that saw `Pending` on its first
/// look is woken afterwards; every promotion is claimed exactly once.
#[test]
fn randomized_waiters_are_never_parked_past_their_promotion() {
    shuttle::check_random(
        || {
            let g = governor(3, 8, Arc::new(ManualClock::new(1_000)));
            let holders: Vec<PermitId> = (0..3).map(|i| admit(&g, &format!("h{i}"))).collect();
            let queued: Vec<(u64, Arc<CountingWaker>)> = (0..3)
                .map(|i| queue_with_waker(&g, &format!("q{i}")))
                .collect();

            let releasers: Vec<_> = holders
                .into_iter()
                .map(|holder| {
                    let g = Arc::clone(&g);
                    thread::spawn(move || assert_eq!(g.release(holder), ReleaseOutcome::Released))
                })
                .collect();
            let waiters: Vec<_> = queued
                .iter()
                .map(|(ticket, waker)| {
                    let g = Arc::clone(&g);
                    let ticket = *ticket;
                    let waker = Arc::clone(waker);
                    thread::spawn(move || {
                        let first = g.claim(ticket);
                        let permit = match first {
                            ClaimOutcome::Ready(permit) => permit,
                            ClaimOutcome::Pending => loop {
                                // Wait for the wake, as the host does, then look
                                // again. A wake must come: the promotion had not
                                // committed when `Pending` was observed.
                                if waker.0.load(Ordering::SeqCst) > 0 {
                                    match g.claim(ticket) {
                                        ClaimOutcome::Ready(permit) => break permit,
                                        other => panic!("woken but {other:?}"),
                                    }
                                }
                                thread::yield_now();
                            },
                            other => panic!("unexpected {other:?}"),
                        };
                        assert_eq!(g.release(permit), ReleaseOutcome::Released);
                    })
                })
                .collect();
            for r in releasers {
                r.join().unwrap();
            }
            for w in waiters {
                w.join().unwrap();
            }
            for (_, waker) in &queued {
                assert!(
                    waker.0.load(Ordering::SeqCst) <= 1,
                    "at most one wake per ticket"
                );
            }
            assert_quiescent(&g);
        },
        SCHEDULES,
    );
}

/// The stale-lease race with a claimer that advances and a sweep racing it:
/// exactly one wins, both are told, the books close.
#[test]
fn randomized_claim_versus_reap_fails_closed_both_ways() {
    shuttle::check_random(
        || {
            let clock = Arc::new(ManualClock::new(1_000));
            let g = governor(1, 4, clock.clone());
            let holder = admit(&g, "holder");
            let (ticket, _waker) = queue_with_waker(&g, "queued");
            assert_eq!(g.release(holder), ReleaseOutcome::Released);
            clock.set(1_002);

            let claimer = {
                let g = Arc::clone(&g);
                thread::spawn(move || match g.claim(ticket) {
                    ClaimOutcome::Ready(permit) => {
                        assert!(g.advance_phase(permit, ExecutionPhase::Accepted));
                        assert_eq!(g.release(permit), ReleaseOutcome::Released);
                        true
                    }
                    ClaimOutcome::Terminal(TerminalReason::Reclaimed) => false,
                    other => panic!("unexpected {other:?}"),
                })
            };
            let reaper = {
                let g = Arc::clone(&g);
                thread::spawn(move || g.reap_leaks_with(1).reclaimed_permits)
            };
            let claimed = claimer.join().unwrap();
            let reclaimed = reaper.join().unwrap();
            assert_eq!(
                (claimed, reclaimed),
                (claimed, u32::from(!claimed)),
                "exactly one side owns the permit"
            );
            assert_quiescent(&g);
        },
        SCHEDULES,
    );
}
