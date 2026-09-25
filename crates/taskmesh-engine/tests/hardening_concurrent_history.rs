//! H16: a fixed two-thread schedule is checked only at barrier-delimited
//! quiescent states. The independent ledger admits either legal ordering of
//! each concurrent pair; it never treats a racing snapshot as a model step.

use std::collections::BTreeMap;
use std::sync::{Arc, Barrier};
use std::time::Duration;

use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, Clock, ManualClock, MemoryReleasePolicy, OverflowPolicy,
    ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AbandonOutcome, AdmissionDecision, ClaimOutcome, Governor, LeakSweepReport, PolicySet,
    ReleaseOutcome, TerminalReason, Ticket,
};

const HANG: Duration = Duration::from_secs(5);

fn spec(operation: &str) -> TaskSpec {
    TaskSpec::io(TaskClass::new("c")).operation(operation.to_owned())
}

/// Both workers reach the same barrier before either starts its operation.
/// Joining both is the quiescent cut used by the model assertions below.
fn concurrent_pair<L, R, FL, FR>(left: FL, right: FR) -> (L, R)
where
    L: Send + 'static,
    R: Send + 'static,
    FL: FnOnce() -> L + Send + 'static,
    FR: FnOnce() -> R + Send + 'static,
{
    let start = Arc::new(Barrier::new(3));
    let (left_tx, left_rx) = std::sync::mpsc::sync_channel(1);
    let (right_tx, right_rx) = std::sync::mpsc::sync_channel(1);
    let left_start = Arc::clone(&start);
    let left = std::thread::spawn(move || {
        left_start.wait();
        left_tx.send(left()).expect("left observer alive");
    });
    let right_start = Arc::clone(&start);
    let right = std::thread::spawn(move || {
        right_start.wait();
        right_tx.send(right()).expect("right observer alive");
    });
    start.wait();
    // Never let an engine deadlock turn a normal owner-local test into an
    // unbounded hang. On timeout the process-local workers are left detached;
    // nextest terminates this one-test binary after reporting the failure.
    let results = (
        left_rx
            .recv_timeout(HANG)
            .expect("left operation terminates"),
        right_rx
            .recv_timeout(HANG)
            .expect("right operation terminates"),
    );
    left.join().expect("left operation does not panic");
    right.join().expect("right operation does not panic");
    results
}

fn assert_ledger(governor: &Governor, admitted: u128, terminated: u128, queued: u32) {
    let snapshot = governor.snapshot();
    let class = &snapshot.classes[&TaskClass::new("c")];
    assert_eq!(class.admitted_total, admitted);
    assert_eq!(class.terminated_total, terminated);
    assert_eq!(u128::from(class.inflight), admitted - terminated);
    assert_eq!(class.queued, queued);
    assert_eq!(class.cpu_units_held, admitted - terminated);
    assert_eq!(class.memory_units_held, 0);
    assert_eq!(snapshot.conservation_violation(), None);
}

#[test]
fn two_thread_admit_release_and_reap_match_a_quiescent_reference_ledger() {
    // Replay script: (1) two admits race, (2) release races a third admit,
    // (3) advance the injected clock and reap the promoted-but-unclaimed
    // lease. The only nondeterminism is whether phase 2's third request sees
    // the full queue or the newly freed queue slot. Both reference branches
    // are stated explicitly, so the failing branch is visible in the output.
    let clock = Arc::new(ManualClock::new(1_000));
    let governor_clock: Arc<dyn Clock> = Arc::<ManualClock>::clone(&clock);
    let governor = Arc::new(
        Governor::new(
            PolicySet::new(
                ResourceBudget::new().cpu_units(1),
                BTreeMap::from([(
                    TaskClass::new("c"),
                    ClassPolicy::new()
                        .max_inflight(1)
                        .max_queue_depth(1)
                        .cpu_units(1)
                        .overflow_policy(OverflowPolicy::QueueWithinDepth)
                        .memory_release_policy(MemoryReleasePolicy::LeakDetecting),
                )]),
            ),
            governor_clock,
        )
        .expect("valid single-slot policy"),
    );

    let (first, second) = concurrent_pair(
        {
            let governor = Arc::clone(&governor);
            move || governor.admit(&spec("first"))
        },
        {
            let governor = Arc::clone(&governor);
            move || governor.admit(&spec("second"))
        },
    );
    let (holder, queued) = match (first, second) {
        (AdmissionDecision::Admitted { permit_id }, AdmissionDecision::Queued { ticket })
        | (AdmissionDecision::Queued { ticket }, AdmissionDecision::Admitted { permit_id }) => {
            (permit_id, ticket)
        }
        outcomes => panic!("phase 1: exactly one admit and one queue: {outcomes:?}"),
    };
    assert_ledger(&governor, 1, 0, 1);
    assert_eq!(governor.ticket_status(queued), ClaimOutcome::Pending);

    let (released, third) = concurrent_pair(
        {
            let governor = Arc::clone(&governor);
            move || governor.release(holder)
        },
        {
            let governor = Arc::clone(&governor);
            move || governor.admit(&spec("third"))
        },
    );
    assert_eq!(released, ReleaseOutcome::Released);
    let third: Option<Ticket> = match third {
        AdmissionDecision::Queued { ticket } => Some(ticket),
        AdmissionDecision::Rejected(AdmissionVerdict::QueueFull { .. }) => None,
        outcome => {
            panic!("phase 2: third may queue or meet full depth, never overtake: {outcome:?}")
        }
    };
    // Whichever operation linearized first, the original queued request owns
    // the freed permit. The third request owns no permit yet.
    assert_ledger(&governor, 2, 1, u32::from(third.is_some()));
    assert!(matches!(
        governor.ticket_status(queued),
        ClaimOutcome::Ready(_)
    ));
    if let Some(third) = third {
        assert_eq!(governor.ticket_status(third), ClaimOutcome::Pending);
    }

    clock.advance(100);
    let reaped = governor.reap_leaks_with(10);
    assert_eq!(reaped.reclaimed_permits, 1);
    assert_eq!(
        governor.claim(queued),
        ClaimOutcome::Terminal(TerminalReason::Reclaimed)
    );
    assert_eq!(governor.claim(queued), ClaimOutcome::Invalid);
    if let Some(third) = third {
        assert_ledger(&governor, 3, 2, 0);
        let ClaimOutcome::Ready(permit) = governor.claim(third) else {
            panic!("phase 3: the surviving queued request must inherit the freed slot")
        };
        assert_eq!(governor.release(permit), ReleaseOutcome::Released);
        assert_ledger(&governor, 3, 3, 0);
    } else {
        assert_ledger(&governor, 2, 2, 0);
    }
}

#[derive(Debug)]
enum Settlement {
    Release(ReleaseOutcome),
    Claim(ClaimOutcome),
    Timeout(AbandonOutcome),
    Cancel(AbandonOutcome),
    Sweep(LeakSweepReport),
}

#[test]
fn release_claim_timeout_cancel_and_sweep_preserve_one_owner() {
    // All five calls begin behind one barrier. The first queued ticket is the
    // only possible successor to the holder; the two later tickets are
    // independently removed by a timeout and an explicit cancellation.
    // A stale, undispatched holder may be ended by release or the sweep, but
    // exactly one of those operations can return its capacity.
    for attempt in 0..32 {
        let clock = Arc::new(ManualClock::new(1_000));
        let governor_clock: Arc<dyn Clock> = Arc::<ManualClock>::clone(&clock);
        let governor = Arc::new(
            Governor::new(
                PolicySet::new(
                    ResourceBudget::new().cpu_units(1),
                    BTreeMap::from([(
                        TaskClass::new("c"),
                        ClassPolicy::new()
                            .max_inflight(1)
                            .max_queue_depth(3)
                            .cpu_units(1)
                            .overflow_policy(OverflowPolicy::QueueWithinDepth)
                            .memory_release_policy(MemoryReleasePolicy::LeakDetecting),
                    )]),
                ),
                governor_clock,
            )
            .expect("valid one-slot policy"),
        );
        let holder = match governor.admit(&spec("holder")) {
            AdmissionDecision::Admitted { permit_id } => permit_id,
            other => panic!("attempt {attempt}: holder must admit: {other:?}"),
        };
        let mut queued = Vec::new();
        for operation in ["claim", "timeout", "cancel"] {
            match governor.admit(&spec(operation)) {
                AdmissionDecision::Queued { ticket } => queued.push(ticket),
                other => panic!("attempt {attempt}: {operation} must queue: {other:?}"),
            }
        }
        let [claim_ticket, timeout_ticket, cancel_ticket]: [Ticket; 3] =
            queued.try_into().expect("three queued tickets");
        assert_ledger(&governor, 1, 0, 3);
        clock.advance(100);

        let start = Arc::new(Barrier::new(6));
        let (sender, receiver) = std::sync::mpsc::channel();
        let operations: Vec<Box<dyn FnOnce() -> Settlement + Send>> = vec![
            Box::new({
                let governor = Arc::clone(&governor);
                move || Settlement::Release(governor.release(holder))
            }),
            Box::new({
                let governor = Arc::clone(&governor);
                move || Settlement::Claim(governor.claim(claim_ticket))
            }),
            Box::new({
                let governor = Arc::clone(&governor);
                move || Settlement::Timeout(governor.abandon(timeout_ticket))
            }),
            Box::new({
                let governor = Arc::clone(&governor);
                move || Settlement::Cancel(governor.abandon(cancel_ticket))
            }),
            Box::new({
                let governor = Arc::clone(&governor);
                move || Settlement::Sweep(governor.reap_leaks_with(10))
            }),
        ];
        let mut workers = Vec::new();
        for operation in operations {
            let worker_start = Arc::clone(&start);
            let worker_sender = sender.clone();
            workers.push(std::thread::spawn(move || {
                worker_start.wait();
                worker_sender
                    .send(operation())
                    .expect("settlement observer alive");
            }));
        }
        drop(sender);
        start.wait();
        let results: Vec<_> = (0..5)
            .map(|_| {
                receiver
                    .recv_timeout(HANG)
                    .expect("each racing settlement terminates")
            })
            .collect();
        for worker in workers {
            worker.join().expect("settlement worker does not panic");
        }

        let mut release = None;
        let mut claim = None;
        let mut timeout = None;
        let mut cancel = None;
        let mut sweep = None;
        for result in results {
            match result {
                Settlement::Release(value) => release = Some(value),
                Settlement::Claim(value) => claim = Some(value),
                Settlement::Timeout(value) => timeout = Some(value),
                Settlement::Cancel(value) => cancel = Some(value),
                Settlement::Sweep(value) => sweep = Some(value),
            }
        }
        let release = release.expect("release result");
        let sweep = sweep.expect("sweep result");
        assert_eq!(
            u32::from(release == ReleaseOutcome::Released) + sweep.reclaimed_permits,
            1,
            "attempt {attempt}: holder capacity returns exactly once"
        );
        assert!(matches!(
            release,
            ReleaseOutcome::Released | ReleaseOutcome::UnknownPermit
        ));
        assert_eq!(timeout, Some(AbandonOutcome::Abandoned));
        assert_eq!(cancel, Some(AbandonOutcome::Abandoned));
        assert_eq!(governor.claim(timeout_ticket), ClaimOutcome::Invalid);
        assert_eq!(governor.claim(cancel_ticket), ClaimOutcome::Invalid);

        let successor = match claim.expect("claim result") {
            ClaimOutcome::Ready(permit) => permit,
            ClaimOutcome::Pending => match governor.claim(claim_ticket) {
                ClaimOutcome::Ready(permit) => permit,
                other => {
                    panic!("attempt {attempt}: pending claimant did not become ready: {other:?}")
                }
            },
            other => panic!("attempt {attempt}: claimant lost ownership: {other:?}"),
        };
        assert_ne!(successor, holder);
        assert_eq!(governor.claim(claim_ticket), ClaimOutcome::Invalid);
        assert_ledger(&governor, 2, 1, 0);
        assert_eq!(governor.release(successor), ReleaseOutcome::Released);
        assert_ledger(&governor, 2, 2, 0);
    }
}
