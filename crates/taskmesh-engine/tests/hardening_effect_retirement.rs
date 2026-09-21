//! TM16-030 regression: host code never runs under the engine's state mutex —
//! not `wake()`, and not a destructor either.
//!
//! The original defect: `abandon` removed a queued request and dropped it while
//! still holding the state lock. The request owns `Option<Arc<dyn PermitWaker>>`,
//! and a driven port's destructor is host code. A waker whose `Drop` touched the
//! governor re-entered a non-reentrant mutex the same thread already held, and
//! the whole governor stopped — admit, release, claim, snapshot, everything.
//!
//! Explicit `wake()` calls were already deferred past the lock; the *retirement*
//! of the same object was not. Both are covered here, plus the promotion budget:
//! bounding the critical section is only safe if a pass that stops early
//! guarantees it will be resumed.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Weak};
use std::time::Duration;

use taskmesh_contract::{
    ClassPolicy, ManualClock, MemoryOvercommitPolicy, OverflowPolicy, PermitWaker, ResourceBudget,
    TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, ClaimOutcome, Governor, PermitId, PolicySet, ReleaseOutcome,
    ResolvedCapability, TerminalReason,
};

/// A waker that re-enters the governor from its destructor — the exact shape a
/// driven port is allowed to have, and the one that deadlocked.
struct ReentrantDropWaker {
    governor: Weak<Governor>,
    entered: Arc<AtomicUsize>,
    completed: Arc<AtomicUsize>,
    woke: Arc<AtomicUsize>,
}

impl PermitWaker for ReentrantDropWaker {
    fn wake(&self) {
        self.woke.fetch_add(1, Ordering::SeqCst);
        // Re-entering from `wake()` must be safe too.
        if let Some(governor) = self.governor.upgrade() {
            let _ = governor.snapshot();
        }
    }
}

impl Drop for ReentrantDropWaker {
    fn drop(&mut self) {
        self.entered.fetch_add(1, Ordering::SeqCst);
        if let Some(governor) = self.governor.upgrade() {
            // If this runs under the state mutex, this line never returns.
            let _ = governor.snapshot();
        }
        self.completed.fetch_add(1, Ordering::SeqCst);
    }
}

fn single_slot_governor() -> Arc<Governor> {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("c"),
        ClassPolicy::new()
            .max_inflight(1)
            .max_queue_depth(8)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    );
    Arc::new(
        Governor::new(
            PolicySet::new(ResourceBudget::new().cpu_units(64), classes),
            Arc::new(ManualClock::new(0)),
        )
        .expect("valid policy"),
    )
}

fn spec(op: &str) -> TaskSpec {
    TaskSpec::io(TaskClass::new("c")).operation(op.to_string())
}

/// Run `body` on a worker thread and fail if it does not finish in time.
///
/// The failure being tested is a deadlock, so the test cannot simply call the
/// operation and assert: it has to be able to *observe* not-returning. A hung
/// worker is left behind deliberately rather than joined — joining it would
/// hang the harness too.
fn with_deadline<F: FnOnce() + Send + 'static>(what: &str, body: F) {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        body();
        let _ = tx.send(());
    });
    assert!(
        rx.recv_timeout(Duration::from_secs(10)).is_ok(),
        "{what} did not return: the governor mutex is held by a re-entrant caller"
    );
}

#[test]
fn abandoning_a_queued_request_retires_its_waker_outside_the_lock() {
    let governor = single_slot_governor();
    let entered = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let woke = Arc::new(AtomicUsize::new(0));

    let holder = match governor.admit(&spec("holder")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    };

    let waker: Arc<dyn PermitWaker> = Arc::new(ReentrantDropWaker {
        governor: Arc::downgrade(&governor),
        entered: Arc::clone(&entered),
        completed: Arc::clone(&completed),
        woke: Arc::clone(&woke),
    });
    let ticket = match governor
        .admit_resolved(&spec("queued"), ResolvedCapability::Ungated, Some(waker))
        .expect("ungated is always valid")
    {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected a queued ticket, got {other:?}"),
    };
    // The queue now owns the only strong reference, so abandoning it is what
    // destroys the waker.
    assert_eq!(entered.load(Ordering::SeqCst), 0);

    let abandoning = Arc::clone(&governor);
    with_deadline("abandon with a re-entrant waker destructor", move || {
        abandoning.abandon(ticket);
    });

    assert_eq!(entered.load(Ordering::SeqCst), 1, "the destructor ran");
    assert_eq!(
        completed.load(Ordering::SeqCst),
        1,
        "the destructor ran to completion"
    );

    // And the governor is still usable afterwards.
    let usable = Arc::clone(&governor);
    with_deadline("governor after retirement", move || {
        // Reaching this line at all is the assertion: the mutex is free.
        assert!(usable.snapshot().conservation_violation().is_none());
        assert_eq!(usable.release(holder), ReleaseOutcome::Released);
    });
    assert_eq!(
        governor.snapshot().classes[&TaskClass::new("c")].inflight,
        0
    );
}

#[test]
fn a_rejected_admission_retires_its_waker_outside_the_lock() {
    // The reject path also takes the waker by value, so it also destroys it.
    let governor = single_slot_governor();
    let entered = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let waker: Arc<dyn PermitWaker> = Arc::new(ReentrantDropWaker {
        governor: Arc::downgrade(&governor),
        entered: Arc::clone(&entered),
        completed: Arc::clone(&completed),
        woke: Arc::new(AtomicUsize::new(0)),
    });

    let rejecting = Arc::clone(&governor);
    with_deadline("rejected admission with a re-entrant waker", move || {
        let unknown = TaskSpec::io(TaskClass::new("ghost")).operation("x");
        let decision = rejecting
            .admit_resolved(&unknown, ResolvedCapability::Ungated, Some(waker))
            .expect("ungated is always valid");
        assert!(matches!(decision, AdmissionDecision::Rejected(_)));
    });
    assert_eq!(completed.load(Ordering::SeqCst), 1);
}

#[test]
fn promotion_wakes_a_reentrant_waker_outside_the_lock() {
    let governor = single_slot_governor();
    let woke = Arc::new(AtomicUsize::new(0));
    let holder = match governor.admit(&spec("holder")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    };
    let waker: Arc<dyn PermitWaker> = Arc::new(ReentrantDropWaker {
        governor: Arc::downgrade(&governor),
        entered: Arc::new(AtomicUsize::new(0)),
        completed: Arc::new(AtomicUsize::new(0)),
        woke: Arc::clone(&woke),
    });
    let ticket = match governor
        .admit_resolved(&spec("queued"), ResolvedCapability::Ungated, Some(waker))
        .expect("ungated is always valid")
    {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected a queued ticket, got {other:?}"),
    };

    let releasing = Arc::clone(&governor);
    with_deadline("release with a re-entrant waker", move || {
        assert_eq!(releasing.release(holder), ReleaseOutcome::Released);
    });
    assert_eq!(woke.load(Ordering::SeqCst), 1);

    let ClaimOutcome::Ready(permit) = governor.claim(ticket) else {
        panic!("the queued request must have been promoted");
    };
    assert_eq!(governor.release(permit), ReleaseOutcome::Released);
}

#[test]
fn a_backlog_larger_than_the_promotion_budget_drains_without_further_events() {
    // Bounding the critical section is only safe if a pass that stops early
    // resumes. One release frees far more capacity than one pass may grant, and
    // nothing else is going to arrive to nudge the queue.
    let backlog = taskmesh_engine::PROMOTION_BUDGET * 2 + 7;
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("big"),
        ClassPolicy::new()
            .max_inflight(4)
            .memory_units(u32::try_from(backlog).expect("fits"))
            .memory_overcommit_policy(MemoryOvercommitPolicy::Reject),
    );
    classes.insert(
        TaskClass::new("small"),
        ClassPolicy::new()
            .max_inflight(u32::try_from(backlog).expect("fits") + 8)
            .max_queue_depth(u32::try_from(backlog).expect("fits") + 8)
            .memory_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth)
            .memory_overcommit_policy(MemoryOvercommitPolicy::Queue),
    );
    let governor = Governor::new(
        PolicySet::new(
            ResourceBudget::new().memory_units(u32::try_from(backlog).expect("fits")),
            classes,
        ),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid policy");

    // One permit holds the entire memory budget.
    let big = match governor.admit(&TaskSpec::io(TaskClass::new("big")).operation("hold")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    };

    let tickets: Vec<u64> = (0..backlog)
        .map(|i| {
            match governor.admit(&TaskSpec::io(TaskClass::new("small")).operation(format!("q{i}")))
            {
                AdmissionDecision::Queued { ticket } => ticket,
                other => panic!("expected a queued ticket at {i}, got {other:?}"),
            }
        })
        .collect();
    assert_eq!(
        governor.snapshot().classes[&TaskClass::new("small")].queued,
        u32::try_from(backlog).expect("fits")
    );

    // A single release frees the whole budget at once.
    assert_eq!(governor.release(big), ReleaseOutcome::Released);

    // Every queued request is promoted, with no further external event.
    let mut permits: Vec<PermitId> = Vec::with_capacity(backlog);
    for (i, ticket) in tickets.iter().enumerate() {
        match governor.claim(*ticket) {
            ClaimOutcome::Ready(permit) => permits.push(permit),
            other => panic!("ticket {i} was not promoted: {other:?}"),
        }
    }
    let snapshot = governor.snapshot();
    assert_eq!(snapshot.classes[&TaskClass::new("small")].queued, 0);
    assert_eq!(
        snapshot.classes[&TaskClass::new("small")].inflight,
        u32::try_from(backlog).expect("fits")
    );
    assert_eq!(snapshot.conservation_violation(), None);

    for permit in permits {
        assert_eq!(governor.release(permit), ReleaseOutcome::Released);
    }
    assert_eq!(
        governor.snapshot().classes[&TaskClass::new("small")].inflight,
        0
    );
}

/// A waker that panics when woken, standing in for a broken host port.
struct PanickingWaker;

impl PermitWaker for PanickingWaker {
    fn wake(&self) {
        panic!("host port misbehaved");
    }
}

/// A waker that records having been woken.
struct CountingWaker(Arc<AtomicUsize>);

impl PermitWaker for CountingWaker {
    fn wake(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct DropPanickingWaker {
    woke: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}

impl PermitWaker for DropPanickingWaker {
    fn wake(&self) {
        self.woke.fetch_add(1, Ordering::SeqCst);
    }
}

impl Drop for DropPanickingWaker {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
        panic!("final host reference destructor failed");
    }
}

#[test]
fn claim_final_drop_panic_compensates_and_promotes_the_next_waiter() {
    let governor = single_slot_governor();
    let holder = match governor.admit(&spec("holder")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    };

    let bad_woke = Arc::new(AtomicUsize::new(0));
    let bad_dropped = Arc::new(AtomicUsize::new(0));
    let bad: Arc<dyn PermitWaker> = Arc::new(DropPanickingWaker {
        woke: Arc::clone(&bad_woke),
        dropped: Arc::clone(&bad_dropped),
    });
    let bad_ticket = match governor
        .admit_resolved(&spec("bad"), ResolvedCapability::Ungated, Some(bad))
        .expect("ungated is always valid")
    {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected queued bad ticket, got {other:?}"),
    };
    let good_woke = Arc::new(AtomicUsize::new(0));
    let good: Arc<dyn PermitWaker> = Arc::new(CountingWaker(Arc::clone(&good_woke)));
    let good_ticket = match governor
        .admit_resolved(&spec("good"), ResolvedCapability::Ungated, Some(good))
        .expect("ungated is always valid")
    {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected queued good ticket, got {other:?}"),
    };

    assert_eq!(governor.release(holder), ReleaseOutcome::Released);
    assert_eq!(bad_woke.load(Ordering::SeqCst), 1);
    assert_eq!(bad_dropped.load(Ordering::SeqCst), 0);

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        drop(governor.claim(bad_ticket));
    }));
    assert!(panic.is_err(), "the first host panic remains observable");
    assert_eq!(bad_dropped.load(Ordering::SeqCst), 1);
    assert_eq!(
        governor.claim(bad_ticket),
        ClaimOutcome::Terminal(TerminalReason::ClaimDeliveryFailed),
        "failed custody transfer publishes a terminal answer"
    );
    assert_eq!(
        good_woke.load(Ordering::SeqCst),
        1,
        "compensation immediately promotes and notifies the next waiter"
    );
    let ClaimOutcome::Ready(good_permit) = governor.claim(good_ticket) else {
        panic!("the next waiter must own the compensated slot");
    };
    assert_eq!(governor.release(good_permit), ReleaseOutcome::Released);

    let snapshot = governor.snapshot();
    assert_eq!(snapshot.classes[&TaskClass::new("c")].inflight, 0);
    assert_eq!(snapshot.classes[&TaskClass::new("c")].queued, 0);
    assert_eq!(governor.permit_ledgers().len(), 0);
    assert_eq!(governor.retained_terminal_tickets(), 0);
    assert_eq!(snapshot.conservation_violation(), None);
}

#[test]
fn a_panicking_waker_does_not_starve_the_other_waiters() {
    // Two queued requests are promoted by one release. The first waker panics.
    // The second must still be notified, and the panic must still surface — a
    // contract fault is reported, not swallowed, but not at other waiters'
    // expense.
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("c"),
        ClassPolicy::new()
            .max_inflight(3)
            .max_queue_depth(8)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    );
    let governor = Governor::new(
        PolicySet::new(ResourceBudget::new().cpu_units(1), classes),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid policy");

    let holder = match governor.admit(&spec("holder")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    };
    let woken = Arc::new(AtomicUsize::new(0));
    let bad: Arc<dyn PermitWaker> = Arc::new(PanickingWaker);
    let good: Arc<dyn PermitWaker> = Arc::new(CountingWaker(Arc::clone(&woken)));
    let bad_ticket = match governor
        .admit_resolved(&spec("bad"), ResolvedCapability::Ungated, Some(bad))
        .expect("ungated is always valid")
    {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected a queued ticket, got {other:?}"),
    };
    let good_ticket = match governor
        .admit_resolved(&spec("good"), ResolvedCapability::Ungated, Some(good))
        .expect("ungated is always valid")
    {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected a queued ticket, got {other:?}"),
    };

    // Free enough budget for both at once. Budget is 1 cpu unit, so release the
    // holder and then reconcile nothing — instead, use a wider budget path: the
    // holder's release promotes the first, whose promotion wakes `bad`.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_eq!(governor.release(holder), ReleaseOutcome::Released);
    }));
    assert!(outcome.is_err(), "the host port's panic must surface");

    // The first promoted request (bad) was granted; claim and release it so
    // the second (good) is promoted — its waker must run despite the earlier
    // panic having gone through the same effect path.
    let ClaimOutcome::Ready(first) = governor.claim(bad_ticket) else {
        panic!("first request was promoted");
    };
    assert_eq!(governor.release(first), ReleaseOutcome::Released);
    assert_eq!(
        woken.load(Ordering::SeqCst),
        1,
        "the second waiter was notified"
    );
    let ClaimOutcome::Ready(second) = governor.claim(good_ticket) else {
        panic!("second request was promoted");
    };
    assert_eq!(governor.release(second), ReleaseOutcome::Released);
    assert_eq!(governor.snapshot().conservation_violation(), None);
}

#[test]
fn every_waiter_in_one_pass_is_woken_even_if_an_earlier_one_panics() {
    // ONE release frees the whole budget, so ONE promotion pass wakes four
    // waiters. The first of them panics. The other three must still be woken
    // in that same pass — a loop that stops at the first panic leaves them
    // parked with nobody left to wake them.
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("big"),
        ClassPolicy::new().max_inflight(4).memory_units(4),
    );
    classes.insert(
        TaskClass::new("small"),
        ClassPolicy::new()
            .max_inflight(8)
            .max_queue_depth(8)
            .memory_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth)
            .memory_overcommit_policy(MemoryOvercommitPolicy::Queue),
    );
    let governor = Governor::new(
        PolicySet::new(ResourceBudget::new().memory_units(4), classes),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid policy");

    let big = match governor.admit(&TaskSpec::io(TaskClass::new("big")).operation("hold")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    };
    let small = |op: &str| TaskSpec::io(TaskClass::new("small")).operation(op.to_string());
    let woken = Arc::new(AtomicUsize::new(0));
    let bad: Arc<dyn PermitWaker> = Arc::new(PanickingWaker);
    let mut tickets = vec![match governor
        .admit_resolved(&small("bad"), ResolvedCapability::Ungated, Some(bad))
        .expect("ungated is always valid")
    {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("{other:?}"),
    }];
    for i in 0..3 {
        let good: Arc<dyn PermitWaker> = Arc::new(CountingWaker(Arc::clone(&woken)));
        tickets.push(
            match governor
                .admit_resolved(
                    &small(&format!("good{i}")),
                    ResolvedCapability::Ungated,
                    Some(good),
                )
                .expect("ungated is always valid")
            {
                AdmissionDecision::Queued { ticket } => ticket,
                other => panic!("{other:?}"),
            },
        );
    }
    assert_eq!(
        governor.snapshot().classes[&TaskClass::new("small")].queued,
        4
    );

    // One release, one pass, four wakers.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_eq!(governor.release(big), ReleaseOutcome::Released);
    }));
    assert!(outcome.is_err(), "the broken port's panic surfaces");
    assert_eq!(
        woken.load(Ordering::SeqCst),
        3,
        "every healthy waiter in the same pass was notified"
    );
    for ticket in tickets {
        let ClaimOutcome::Ready(permit) = governor.claim(ticket) else {
            panic!("every queued request was promoted in that pass");
        };
        assert_eq!(governor.release(permit), ReleaseOutcome::Released);
    }
    assert_eq!(governor.snapshot().conservation_violation(), None);
}
