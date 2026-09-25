//! Randomized concurrency model-check of the **real** governor (ADR 9000 / P6,
//! ADR 0003 D-loom).
//!
//! `loom_governance.rs` explores *every* interleaving but only for tiny state.
//! Under `--cfg shuttle` the same `sync` seam builds `Governor` on shuttle's
//! mutex, and these models sample thousands of schedules over larger state —
//! more threads, more operations, two racing abandoners, newcomers racing a
//! promotion pass's continuation gap —
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
use shuttle::{Config as ShuttleConfig, FailurePersistence, MaxSteps, Runner};
use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, ExecutionPhase, FairnessPolicy, ManualClock,
    MemoryReleasePolicy, OverflowPolicy, PermitWaker, ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, CapacityBlock, ClaimOutcome, Governor, PermitId, PolicySet, ReleaseOutcome,
    ResolvedCapability, TerminalReason, Ticket, PROMOTION_BUDGET,
};

const SCHEDULES: usize = 10_000;
const CLASS: &str = "c";
const SHUTTLE_MAX_STEPS: usize = 1_000_000;

fn check_seeded_model<F>(model_id: &'static str, seed: u64, schedules: usize, model: F)
where
    F: Fn() + Sync + Send + 'static,
{
    assert!(
        schedules > 0,
        "shuttle model {model_id} requested zero schedules"
    );
    let mut config = ShuttleConfig::new();
    config.max_steps = MaxSteps::FailAfter(SHUTTLE_MAX_STEPS);
    config.failure_persistence = std::env::var_os("TASKMESH_MODEL_FAILURE_DIR")
        .map_or(FailurePersistence::Print, |directory| {
            FailurePersistence::File(Some(directory.into()))
        });
    let scheduler = shuttle::scheduler::RandomScheduler::new_from_seed(seed, schedules);
    let completed = Runner::new(scheduler, config).run(model);
    assert_eq!(
        completed, schedules,
        "shuttle model {model_id} stopped early"
    );
    println!(
        "taskmesh-model-witness checker=shuttle model_id={model_id} \
         scheduler=random seed={seed} requested={schedules} completed={completed} \
         max_steps={SHUTTLE_MAX_STEPS}"
    );
}

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

fn queue_with_waker(g: &Governor, op: &str) -> (Ticket, Arc<CountingWaker>) {
    let waker = Arc::new(CountingWaker(AtomicUsize::new(0)));
    let port: Arc<dyn PermitWaker> = waker.clone();
    match g.admit_waitable(&spec(op), port) {
        AdmissionDecision::Queued { ticket } => (ticket, waker),
        other => panic!("expected a queued ticket, got {other:?}"),
    }
}

fn assert_quiescent(g: &Governor) {
    let snapshot = g.snapshot();
    for (class, c) in &snapshot.classes {
        assert_eq!(c.inflight, 0, "class {class}: inflight must return to zero");
        assert_eq!(c.queued, 0, "class {class}: nothing may remain queued");
        assert_eq!(
            c.cpu_units_held, 0,
            "class {class}: cpu units must be returned"
        );
        assert_eq!(
            c.memory_units_held, 0,
            "class {class}: memory units must be returned"
        );
        assert_eq!(
            c.admitted_total, c.terminated_total,
            "class {class}: every admission must have terminated"
        );
    }
    for (pool, usage) in &snapshot.capabilities {
        assert_eq!(usage.in_use, 0, "pool {pool}: every slot must be returned");
    }
    assert_eq!(snapshot.conservation_violation(), None);
    assert!(g.permit_ledgers().is_empty(), "no permit may leak");
    assert_eq!(g.accounting_fault(), None);
}

/// Four churners, each admitting and releasing twice, over a two-slot class:
/// admissions queue, releases promote, and the ledgers must close.
#[test]
fn randomized_admit_release_churn_conserves_capacity() {
    check_seeded_model(
        "shuttle.admit_release_churn.v1",
        0x5a17_0001,
        SCHEDULES,
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
    );
}

/// The host `TicketGuard` handoff with *two* abandoners (cancel and
/// acquire-timeout firing together) racing a promoter and a claimer, on the
/// real engine. The permit is accounted exactly once; abandon is idempotent;
/// a claimer that won ownership is never robbed of it.
#[test]
fn randomized_promote_claim_double_abandon_is_exactly_once() {
    check_seeded_model(
        "shuttle.promote_claim_double_abandon.v1",
        0x5a17_0002,
        SCHEDULES,
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
        let _first_abandon_outcome = a1.join().unwrap();
        let _second_abandon_outcome = a2.join().unwrap();

            assert!(matches!(
                g.ticket_status(ticket),
                ClaimOutcome::Invalid | ClaimOutcome::Terminal(_)
            ));
            assert_quiescent(&g);
        },
    );
}

/// Lost-wakeup freedom at scale: three waiters park on three tickets, three
/// holders release concurrently. Every waiter that saw `Pending` on its first
/// look is woken afterwards; every promotion is claimed exactly once.
#[test]
fn randomized_waiters_are_never_parked_past_their_promotion() {
    check_seeded_model(
        "shuttle.waiter_promotion_wakeup.v1",
        0x5a17_0003,
        SCHEDULES,
        || {
            let g = governor(3, 8, Arc::new(ManualClock::new(1_000)));
            let holders: Vec<PermitId> = (0..3).map(|i| admit(&g, &format!("h{i}"))).collect();
            let queued: Vec<(Ticket, Arc<CountingWaker>)> = (0..3)
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
    );
}

/// The stale-lease race with a claimer that advances and a sweep racing it:
/// exactly one wins, both are told, the books close.
#[test]
fn randomized_claim_versus_reap_fails_closed_both_ways() {
    check_seeded_model("shuttle.claim_reap.v1", 0x5a17_0004, SCHEDULES, || {
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
                    let taskmesh_engine::AdvanceOutcome::Leased(lease) =
                        g.advance_phase(permit, ExecutionPhase::Accepted)
                    else {
                        panic!("a claimed lease is live and leases on its first advance");
                    };
                    assert_eq!(
                        g.release(permit),
                        ReleaseOutcome::HeldByLease {
                            phase: ExecutionPhase::Accepted
                        },
                        "a dispatched permit is not releasable by id"
                    );
                    assert_eq!(g.release_leased(lease), ReleaseOutcome::Released);
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
    });
}

// ---- D17: closing admission is decided under the admission lock --------------

/// Schedules in which the closer must have beaten at least one admitter, and
/// schedules in which at least one admitter must have beaten the closer (5% of
/// the sample each): a model where only one side ever wins proves nothing
/// about the race.
const MIN_MIXED: usize = SCHEDULES / 20;

/// Four admitters race one closer on the real engine. The property: the
/// snapshot the closer takes right after `close_admission` returns is the last
/// word — no admission that began before the close lands after it. A close
/// decided outside the admission lock (or checked before the lock is taken)
/// lets an admitter that passed the check before the close slip in after the
/// closer's snapshot, and the final `admitted_total` exceeds it.
#[test]
fn randomized_close_admission_is_the_last_word_on_what_was_admitted() {
    let closer_won_some = Arc::new(AtomicUsize::new(0));
    let admitters_won_some = Arc::new(AtomicUsize::new(0));
    let (closer_stat, admitter_stat) = (
        Arc::clone(&closer_won_some),
        Arc::clone(&admitters_won_some),
    );
    check_seeded_model(
        "shuttle.close_admission.v1",
        0x5a17_0005,
        SCHEDULES,
        move || {
            let g = governor(8, 8, Arc::new(ManualClock::new(1_000)));
            let admitters: Vec<_> = (0..4)
                .map(|i| {
                    let g = Arc::clone(&g);
                    thread::spawn(move || {
                        let mut mine = Vec::new();
                        let mut refused = 0usize;
                        for round in 0..2 {
                            match g.admit(&spec(&format!("t{i}r{round}"))) {
                                AdmissionDecision::Admitted { permit_id } => mine.push(permit_id),
                                AdmissionDecision::Rejected(
                                    AdmissionVerdict::RuntimeUnavailable,
                                ) => {
                                    assert!(
                                        g.admission_closed(),
                                        "a refusal must mean the close already happened"
                                    );
                                    refused += 1;
                                }
                                other => panic!("unexpected {other:?}"),
                            }
                        }
                        (mine, refused)
                    })
                })
                .collect();
            let closer = {
                let g = Arc::clone(&g);
                thread::spawn(move || {
                    g.close_admission();
                    g.snapshot().classes[&class()].admitted_total
                })
            };
            let at_close = closer.join().unwrap();
            let mut permits = Vec::new();
            let mut refused = 0usize;
            for h in admitters {
                let (mine, r) = h.join().unwrap();
                permits.extend(mine);
                refused += r;
            }
            let end = g.snapshot().classes[&class()].admitted_total;
            assert_eq!(
                end, at_close,
                "an admission landed after the closer's snapshot: the close is not the last word"
            );
            assert_eq!(
                end,
                u128::try_from(permits.len()).expect("fits"),
                "every admission the callers saw is in the total"
            );
            assert_eq!(
                permits.len() + refused,
                8,
                "every submission was admitted or refused"
            );
            if refused > 0 {
                closer_stat.fetch_add(1, Ordering::SeqCst);
            }
            if !permits.is_empty() {
                admitter_stat.fetch_add(1, Ordering::SeqCst);
            }
            for permit in permits {
                assert_eq!(g.release(permit), ReleaseOutcome::Released);
            }
            assert_quiescent(&g);
            assert!(g.admission_closed(), "draining to zero never reopens");
        },
    );
    let closer_won = closer_won_some.load(Ordering::SeqCst);
    let admitters_won = admitters_won_some.load(Ordering::SeqCst);
    assert!(
        closer_won >= MIN_MIXED,
        "only {closer_won} of {SCHEDULES} schedules had the closer beat an admitter"
    );
    assert!(
        admitters_won >= MIN_MIXED,
        "only {admitters_won} of {SCHEDULES} schedules had an admitter beat the closer"
    );
}

// ---- D08: the promotion-gap rule under real concurrency ----------------------
//
// `hardening_fairness_reference.rs` proves the gap rule from one deterministic
// vantage point: a waker that re-enters `admit` while the pass that woke it is
// still owed a continuation. That is single-threaded. Here the same production
// transitions run with the gap *open to other threads*: a release drains a
// queue longer than `PROMOTION_BUDGET` while newcomers race to admit, and
// shuttle decides — thousands of times — whether each lands before the release,
// inside the gap, or after the drain.

/// `a` arrivals queued behind the holder: one full pass plus a tail, so the drain
/// has exactly one continuation gap.
const GAP_QUEUED: usize = PROMOTION_BUDGET + 4;
/// Heavier per schedule than the models above (a 68-deep queue and a full drain),
/// so fewer schedules; still far more than the gap needs to be hit.
const GAP_SCHEDULES: usize = 5_000;
/// Schedules that must land a newcomer inside the gap (5% of the sample; the
/// measured rate is ≈49%). A sampler that stops reaching the gap fails here.
const MIN_IN_GAP: usize = GAP_SCHEDULES / 20;

fn class_named(name: &str) -> TaskClass {
    TaskClass::new(name.to_string())
}

fn spec_of(class: &str, op: &str) -> TaskSpec {
    TaskSpec::io(class_named(class)).operation(op.to_string())
}

fn queueable_fifo(cpu_units: u32) -> ClassPolicy {
    ClassPolicy::new()
        .max_inflight(1_000)
        .max_queue_depth(1_000)
        .cpu_units(cpu_units)
        .overflow_policy(OverflowPolicy::QueueWithinDepth)
        .fairness(FairnessPolicy::Fifo)
}

/// The gap fixture. The cpu budget is exactly `GAP_QUEUED` units and `holder`
/// takes all of it, so:
///  * before the release every `a` arrival queues on `Cpu`;
///  * the release opens the whole budget, pass 1 grants `PROMOTION_BUDGET`,
///    drops the lock (the gap), and the continuation grants the tail;
///  * after the drain the budget is full again — no cpu-costing newcomer fits.
///
/// A cpu-costing queueable newcomer therefore has exactly one way to be admitted
/// directly: inside the gap. That makes the overtake unambiguous without relying
/// on permit ids, which a direct admit allocates *before* taking the lock.
fn gap_governor(extra: Vec<(&str, ClassPolicy)>, limits: BTreeMap<String, u32>) -> Arc<Governor> {
    let units = u32::try_from(GAP_QUEUED).expect("small");
    let mut classes = BTreeMap::new();
    classes.insert(class_named("a"), queueable_fifo(1));
    classes.insert(
        class_named("holder"),
        ClassPolicy::new().max_inflight(1).cpu_units(units),
    );
    for (name, policy) in extra {
        classes.insert(class_named(name), policy);
    }
    let policy = PolicySet::new(ResourceBudget::new().cpu_units(units), classes)
        .with_capability_limits(limits)
        .expect("registered pools");
    Arc::new(Governor::new(policy, Arc::new(ManualClock::new(1_000))).expect("valid policy"))
}

fn resolved(g: &Governor, pool: Option<&str>) -> ResolvedCapability {
    pool.map_or(ResolvedCapability::Ungated, |name| {
        g.policy()
            .resolve_capability(name)
            .expect("fixture pool is registered")
    })
}

fn admit_on(g: &Governor, class: &str, op: &str, pool: Option<&str>) -> PermitId {
    match g
        .admit_resolved(&spec_of(class, op), resolved(g, pool), None)
        .expect("fixture capability belongs to this policy")
    {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected {class}/{op} to be admitted, got {other:?}"),
    }
}

fn queue_on(g: &Governor, class: &str, op: &str, pool: Option<&str>) -> Ticket {
    match g
        .admit_resolved(&spec_of(class, op), resolved(g, pool), None)
        .expect("fixture capability belongs to this policy")
    {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected {class}/{op} to queue, got {other:?}"),
    }
}

/// Queue `GAP_QUEUED` requests of `a` behind the holder, in arrival order.
fn queue_originals(g: &Governor) -> Vec<Ticket> {
    (0..GAP_QUEUED)
        .map(|i| queue_on(g, "a", &format!("a{i}"), None))
        .collect()
}

/// Claim every ticket, in the given order; each must have been promoted.
fn claim_all(g: &Governor, tickets: &[Ticket]) -> Vec<PermitId> {
    tickets
        .iter()
        .map(|ticket| match g.claim(*ticket) {
            ClaimOutcome::Ready(permit) => permit,
            other => panic!("ticket {ticket} must be promoted by the drain, got {other:?}"),
        })
        .collect()
}

/// Admit on another thread; report the decision and, when queued, the block
/// reason recorded at intake (`QueuedBehind` is the gap's signature).
fn spawn_newcomer(
    g: &Arc<Governor>,
    spec: TaskSpec,
    pool: Option<&'static str>,
) -> thread::JoinHandle<(AdmissionDecision, Option<CapacityBlock>)> {
    let g = Arc::clone(g);
    thread::spawn(move || {
        let decision = g
            .admit_resolved(&spec, resolved(&g, pool), None)
            .expect("fixture capability belongs to this policy");
        let blocked_on = match &decision {
            AdmissionDecision::Queued { ticket } => g.pending_block_reason(*ticket),
            _ => None,
        };
        (decision, blocked_on)
    })
}

/// A cpu-costing queueable newcomer must have queued, whatever the schedule:
/// on `Cpu` if it landed before the release or after the drain, as
/// `QueuedBehind` if it landed in the gap. `Admitted` is the overtake D08
/// forbids. Returns the ticket and whether the gap was where it landed.
fn queued_newcomer(
    who: &str,
    decision: &AdmissionDecision,
    blocked_on: Option<CapacityBlock>,
) -> (Ticket, bool) {
    let AdmissionDecision::Queued { ticket } = *decision else {
        panic!("{who} must queue: capacity that opens in the gap belongs to the queue, got {decision:?}");
    };
    match blocked_on {
        Some(CapacityBlock::QueuedBehind) => (ticket, true),
        Some(CapacityBlock::Cpu) => (ticket, false),
        other => panic!("{who} queued for an unexpected reason: {other:?}"),
    }
}

/// `promotion_pending` has no accessor; it is observed through behaviour. With
/// every queue empty and capacity free, a direct admit of a queueable class is
/// `Admitted` only if no continuation is still owed — a flag left set would
/// queue it for a pass that never comes.
fn assert_nothing_owed(g: &Governor, class: &str) {
    match g.admit(&spec_of(class, "after-the-drain")) {
        AdmissionDecision::Admitted { permit_id } => {
            assert_eq!(g.release(permit_id), ReleaseOutcome::Released);
        }
        other => {
            panic!("no continuation is owed after the drain: direct admit expected, got {other:?}")
        }
    }
}

/// The D08 gap rule with other threads in the gap. Three newcomers race the
/// release that drains the `a` queue:
///  * same class `a` — runnable earlier arrivals are queued ahead of it, so it
///    must queue behind them wherever it lands (rule 1, `queued_head_blocks`);
///  * another queueable class `b`, own queue empty — while the continuation is
///    owed it must become `b`'s head and let the pass order it, not take the
///    slot (rule 2, `promotion_pending`);
///  * class `c`, whose head is stuck behind the held `large_stack` slot, on the
///    `blocking` pool — the one same-class exception: admitted directly, mid-gap
///    or not, because queueing it would pin it to a pool it does not need.
///
/// Every schedule ends with every earlier `a` arrival granted in arrival order,
/// both queueable newcomers still waiting, the exception admitted, and nothing
/// owed once the drain is over.
#[test]
fn randomized_gap_arrivals_queue_behind_runnable_heads() {
    static SAME_CLASS_IN_GAP: AtomicUsize = AtomicUsize::new(0);
    static CROSS_CLASS_IN_GAP: AtomicUsize = AtomicUsize::new(0);
    check_seeded_model(
        "shuttle.gap_arrivals.v1",
        0x5a17_0006,
        GAP_SCHEDULES,
        || {
            let g = gap_governor(
                vec![("b", queueable_fifo(1)), ("c", queueable_fifo(0))],
                BTreeMap::from([("large_stack".to_string(), 1), ("blocking".to_string(), 10)]),
            );
            let slot = admit_on(&g, "c", "slot", Some("large_stack"));
            let blocked_head = queue_on(&g, "c", "blocked-head", Some("large_stack"));
            let holder = admit_on(&g, "holder", "holder", None);
            let originals = queue_originals(&g);

            let releaser = {
                let g = Arc::clone(&g);
                thread::spawn(move || assert_eq!(g.release(holder), ReleaseOutcome::Released))
            };
            let same = spawn_newcomer(&g, spec_of("a", "same-class"), None);
            let cross = spawn_newcomer(&g, spec_of("b", "cross-class"), None);
            let exception = spawn_newcomer(&g, spec_of("c", "other-pool"), Some("blocking"));
            releaser.join().unwrap();
            let (same_decision, same_blocked_on) = same.join().unwrap();
            let (cross_decision, cross_blocked_on) = cross.join().unwrap();
            let (exception_decision, _) = exception.join().unwrap();

            let (same_ticket, same_in_gap) =
                queued_newcomer("the same-class newcomer", &same_decision, same_blocked_on);
            let (cross_ticket, cross_in_gap) = queued_newcomer(
                "the cross-class newcomer",
                &cross_decision,
                cross_blocked_on,
            );
            if same_in_gap {
                SAME_CLASS_IN_GAP.fetch_add(1, Ordering::SeqCst);
            }
            if cross_in_gap {
                CROSS_CLASS_IN_GAP.fetch_add(1, Ordering::SeqCst);
            }
            let AdmissionDecision::Admitted {
                permit_id: exception_permit,
            } = exception_decision
            else {
                panic!(
                    "a newcomer on another pool must not be queued behind a capability-blocked head, \
                     even mid-gap: {exception_decision:?}"
                );
            };

            // Every earlier `a` arrival was promoted by the drain, in arrival
            // order: promotion allocates permit ids under the lock, so the ids
            // are the promotion order.
            let permits = claim_all(&g, &originals);
            assert!(
                permits.windows(2).all(|pair| pair[0] < pair[1]),
                "in-class FIFO: the drain grants earlier arrivals first"
            );
            // Neither queueable newcomer got a slot: the drain refilled the
            // budget with the arrivals that were ahead of them.
            assert!(
                matches!(g.ticket_status(same_ticket), ClaimOutcome::Pending),
                "the same-class newcomer waits behind every earlier arrival"
            );
            assert!(
                matches!(g.ticket_status(cross_ticket), ClaimOutcome::Pending),
                "the cross-class newcomer waits for the pass to order it"
            );
            assert!(
                matches!(g.ticket_status(blocked_head), ClaimOutcome::Pending),
                "the blocked head still waits for its own pool"
            );

            // Nothing is stranded: the originals' release promotes both
            // newcomers, the slot's release promotes the blocked head.
            for permit in permits {
                assert_eq!(g.release(permit), ReleaseOutcome::Released);
            }
            for ticket in [same_ticket, cross_ticket] {
                let ClaimOutcome::Ready(permit) = g.claim(ticket) else {
                    panic!("newcomer {ticket} is promoted once the arrivals ahead of it release");
                };
                assert_eq!(g.release(permit), ReleaseOutcome::Released);
            }
            assert_eq!(g.release(exception_permit), ReleaseOutcome::Released);
            assert_eq!(g.release(slot), ReleaseOutcome::Released);
            let ClaimOutcome::Ready(head_permit) = g.claim(blocked_head) else {
                panic!("the freed slot promotes the blocked head");
            };
            assert_eq!(g.release(head_permit), ReleaseOutcome::Released);
            assert_nothing_owed(&g, "b");
            assert_quiescent(&g);
        },
    );
    // The sample must actually have put newcomers inside the gap; a run that
    // only ever saw `Cpu` proved nothing about the rule. Measured hit rate is
    // ≈49% of schedules; the floor is set an order of magnitude below that so
    // it detects a sampler that stopped exploring the gap, not normal variance.
    let same = SAME_CLASS_IN_GAP.load(Ordering::SeqCst);
    let cross = CROSS_CLASS_IN_GAP.load(Ordering::SeqCst);
    assert!(
        same >= MIN_IN_GAP,
        "only {same} of {GAP_SCHEDULES} schedules landed the same-class newcomer in the gap"
    );
    assert!(
        cross >= MIN_IN_GAP,
        "only {cross} of {GAP_SCHEDULES} schedules landed the cross-class newcomer in the gap"
    );
}

/// The only work that may overtake a runnable head in the gap is work that
/// could not have waited: a class with no queue at all (`Reject` overflow, no
/// memory queue). `r` races the drain alongside a same-class `a` newcomer. `r`
/// is shed with `CpuSaturated` before the release or after the drain, and
/// admitted inside the gap — and that overtake costs `a` exactly one slot, at
/// its *tail*: in-class order is untouched, the tail arrival runs when `r`
/// releases, and the same-class newcomer still queues behind all of it.
#[test]
fn randomized_only_unqueueable_work_overtakes_in_the_gap() {
    static OVERTOOK_IN_GAP: AtomicUsize = AtomicUsize::new(0);
    check_seeded_model(
        "shuttle.unqueueable_overtake.v1",
        0x5a17_0007,
        GAP_SCHEDULES,
        || {
            let g = gap_governor(
                vec![("r", ClassPolicy::new().max_inflight(1_000).cpu_units(1))],
                BTreeMap::new(),
            );
            let holder = admit_on(&g, "holder", "holder", None);
            let originals = queue_originals(&g);

            let releaser = {
                let g = Arc::clone(&g);
                thread::spawn(move || assert_eq!(g.release(holder), ReleaseOutcome::Released))
            };
            let unqueueable = spawn_newcomer(&g, spec_of("r", "unqueueable"), None);
            let same = spawn_newcomer(&g, spec_of("a", "same-class"), None);
            releaser.join().unwrap();
            let (unqueueable_decision, _) = unqueueable.join().unwrap();
            let (same_decision, same_blocked_on) = same.join().unwrap();

            let (same_ticket, _) =
                queued_newcomer("the same-class newcomer", &same_decision, same_blocked_on);
            let overtaker = match unqueueable_decision {
                AdmissionDecision::Admitted { permit_id } => Some(permit_id),
                AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated {
                    retry_after_ms: None,
                }) => None,
                other => panic!(
                    "an unqueueable newcomer is admitted in the gap or shed with CpuSaturated, \
                     never queued: {other:?}"
                ),
            };

            // Everything but the tail was promoted by the drain, in order.
            let (tail, ahead) = originals.split_last().expect("non-empty");
            let mut permits = claim_all(&g, ahead);
            assert!(
                permits.windows(2).all(|pair| pair[0] < pair[1]),
                "in-class FIFO: the drain grants earlier arrivals first"
            );
            if let Some(permit_id) = overtaker {
                OVERTOOK_IN_GAP.fetch_add(1, Ordering::SeqCst);
                // The overtake displaced exactly one arrival — the last one.
                assert!(
                    matches!(g.ticket_status(*tail), ClaimOutcome::Pending),
                    "an in-gap overtake displaces only the tail arrival"
                );
                assert_eq!(g.release(permit_id), ReleaseOutcome::Released);
            }
            let ClaimOutcome::Ready(tail_permit) = g.claim(*tail) else {
                panic!("the tail arrival is promoted, by the drain or by the overtaker's release");
            };
            assert!(
                permits.iter().all(|permit| *permit < tail_permit),
                "the tail is granted after every earlier arrival"
            );
            permits.push(tail_permit);
            assert!(
                matches!(g.ticket_status(same_ticket), ClaimOutcome::Pending),
                "the same-class newcomer waits behind every earlier arrival"
            );
            for permit in permits {
                assert_eq!(g.release(permit), ReleaseOutcome::Released);
            }
            let ClaimOutcome::Ready(permit) = g.claim(same_ticket) else {
                panic!("the newcomer is promoted once the arrivals ahead of it release");
            };
            assert_eq!(g.release(permit), ReleaseOutcome::Released);
            assert_nothing_owed(&g, "a");
            assert_quiescent(&g);
        },
    );
    let overtook = OVERTOOK_IN_GAP.load(Ordering::SeqCst);
    assert!(
        overtook >= MIN_IN_GAP,
        "only {overtook} of {GAP_SCHEDULES} schedules admitted the unqueueable newcomer inside the gap"
    );
}
