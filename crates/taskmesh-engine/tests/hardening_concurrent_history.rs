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
    AdmissionDecision, ClaimOutcome, Governor, PolicySet, ReleaseOutcome, TerminalReason, Ticket,
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
