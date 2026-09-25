//! `Governor::close_admission` (ADR 0003 D17): the engine half of the host's
//! drain.
//!
//! Closing is decided under the admission lock, before the class is looked up,
//! so a refusal touches nothing — no queue entry, no charge, no total — and a
//! snapshot taken after `close_admission` returns is the last word on what was
//! ever admitted. Work the engine already holds is *not* cancelled: a queued
//! request is still promoted and claimed, every lease is still released, and
//! the gauges converge to zero on their own. The state is one-way.
//!
//! Each property is pinned by its own mutant in
//! `tools/verification/mutations.json` (`close-admission-*`).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, ManualClock, OverflowPolicy, PermitWaker, ResourceBudget,
    TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, CapabilityResolutionError, ClaimOutcome, Governor, PolicySet, ReleaseOutcome,
};

fn class(name: &'static str) -> TaskClass {
    TaskClass::new(name)
}

fn spec(class_name: &'static str, op: &str) -> TaskSpec {
    TaskSpec::io(class(class_name)).operation(op.to_string())
}

/// One queueing slot in `c` plus an idle, unqueueable `other`.
fn governor() -> Arc<Governor> {
    let mut classes = BTreeMap::new();
    classes.insert(
        class("c"),
        ClassPolicy::new()
            .max_inflight(1)
            .max_queue_depth(4)
            .cpu_units(1)
            .memory_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    );
    classes.insert(
        class("other"),
        ClassPolicy::new()
            .max_inflight(2)
            .cpu_units(1)
            .memory_units(1),
    );
    let policy = PolicySet::new(
        ResourceBudget::new().cpu_units(100).memory_units(100),
        classes,
    );
    Arc::new(Governor::new(policy, Arc::new(ManualClock::new(1_000))).expect("valid policy"))
}

/// Counts drops: a refused submission's waker is host code and must still be
/// retired (dropped) exactly once, outside the lock.
struct DropCountingWaker(Arc<AtomicUsize>);

impl PermitWaker for DropCountingWaker {
    fn wake(&self) {}
}

impl Drop for DropCountingWaker {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn refused(what: &str, decision: AdmissionDecision) {
    match decision {
        AdmissionDecision::Rejected(AdmissionVerdict::RuntimeUnavailable) => {}
        other => {
            panic!("{what}: a closed governor must refuse with RuntimeUnavailable, got {other:?}")
        }
    }
}

#[test]
fn a_fresh_governor_is_open_and_closing_is_one_way() {
    let g = governor();
    assert!(!g.admission_closed(), "a fresh governor admits");
    g.close_admission();
    assert!(g.admission_closed(), "close_admission must be observable");
    // Idempotent: a second close changes nothing and never reopens.
    g.close_admission();
    assert!(
        g.admission_closed(),
        "closing is one-way: there is no reopen"
    );
    refused("admit after close", g.admit(&spec("c", "late")));
}

#[test]
fn a_closed_governor_refuses_before_the_class_is_looked_up_and_charges_nothing() {
    let g = governor();
    g.close_admission();
    let before = g.snapshot();

    // Every intake entry point, every class — including one the governor does
    // not know: the close is decided before the class lookup, so an unknown
    // class is refused as *unavailable*, not as unknown.
    refused("admit", g.admit(&spec("c", "a")));
    refused("admit (other)", g.admit(&spec("other", "b")));
    refused(
        "admit_resolved",
        g.admit_resolved(
            &spec("c", "c"),
            g.policy()
                .resolve_capability("blocking")
                .expect("registered blocking pool"),
            None,
        )
        .expect("validated capability"),
    );
    refused("admit (unknown class)", g.admit(&spec("nobody", "d")));
    let drops = Arc::new(AtomicUsize::new(0));
    let waker: Arc<dyn PermitWaker> = Arc::new(DropCountingWaker(Arc::clone(&drops)));
    refused("admit_waitable", g.admit_waitable(&spec("c", "e"), waker));
    assert_eq!(
        drops.load(Ordering::SeqCst),
        1,
        "a refused submission's waker is retired exactly once, not kept alive"
    );

    let after = g.snapshot();
    assert_eq!(
        after.classes, before.classes,
        "a refusal must not move any gauge or total in any class"
    );
    assert_eq!(after.capabilities, before.capabilities);
    assert!(g.permit_ledgers().is_empty(), "nothing was granted");
    assert_eq!(after.conservation_violation(), None);
    assert_eq!(
        g.accounting_fault(),
        None,
        "closing is not a fault: it refuses with the same verdict but reports separately"
    );
}

#[test]
fn closed_preflight_precedence_is_explicit_and_side_effect_free() {
    let g = governor();
    let foreign_governor = governor();
    let local = g
        .policy()
        .resolve_capability("blocking")
        .expect("local built-in pool");
    let foreign = foreign_governor
        .policy()
        .resolve_capability("blocking")
        .expect("same name, different policy authority");
    let malformed = TaskSpec::io(class("c")); // operation is deliberately empty
    let valid = spec("c", "after-close");
    g.close_admission();
    let before = g.snapshot();
    let drops = Arc::new(AtomicUsize::new(0));

    refused("valid request", g.admit(&valid));
    assert!(matches!(
        g.admit(&malformed),
        AdmissionDecision::Rejected(AdmissionVerdict::MalformedTask)
    ));
    assert!(matches!(
        g.admit_waitable(
            &malformed,
            Arc::new(DropCountingWaker(Arc::clone(&drops))),
        ),
        AdmissionDecision::Rejected(AdmissionVerdict::MalformedTask)
    ));
    assert_eq!(
        g.admit_resolved(&valid, foreign.clone(), None),
        Err(CapabilityResolutionError::ForeignId),
        "capability preflight precedes the closed-state verdict"
    );
    assert_eq!(
        g.admit_resolved(
            &malformed,
            foreign,
            Some(Arc::new(DropCountingWaker(Arc::clone(&drops)))),
        ),
        Err(CapabilityResolutionError::ForeignId),
        "capability preflight also precedes malformed-spec preflight"
    );
    assert!(matches!(
        g.admit_resolved(&malformed, local.clone(), None),
        Ok(AdmissionDecision::Rejected(AdmissionVerdict::MalformedTask))
    ));
    refused(
        "valid local capability",
        g.admit_resolved(&valid, local, None)
            .expect("local capability is valid"),
    );

    assert_eq!(drops.load(Ordering::SeqCst), 2);
    assert_eq!(g.snapshot(), before);
    assert!(g.permit_ledgers().is_empty());
    assert!(g.admission_closed());
}

#[test]
fn work_held_at_close_still_promotes_claims_and_releases_to_zero() {
    let g = governor();
    let AdmissionDecision::Admitted { permit_id: holder } = g.admit(&spec("c", "holder")) else {
        panic!("the first submission takes the single slot")
    };
    let AdmissionDecision::Queued { ticket } = g.admit(&spec("c", "queued")) else {
        panic!("the second submission queues behind it")
    };
    let AdmissionDecision::Admitted { permit_id: other } = g.admit(&spec("other", "idle")) else {
        panic!("the other class has room")
    };

    g.close_admission();
    let at_close = g.snapshot();
    assert_eq!(
        (
            at_close.classes[&class("c")].inflight,
            at_close.classes[&class("c")].queued
        ),
        (1, 1),
        "closing must cancel neither the holder nor the queued request"
    );
    refused("newcomer after close", g.admit(&spec("c", "newcomer")));

    // The holder finishes: the queued request is promoted despite the close,
    // and claiming it hands over a real permit.
    assert_eq!(g.release(holder), ReleaseOutcome::Released);
    let ClaimOutcome::Ready(promoted) = g.claim(ticket) else {
        panic!("a queued request must still be promoted after the close")
    };
    let mid = g.snapshot();
    assert_eq!(
        (
            mid.classes[&class("c")].inflight,
            mid.classes[&class("c")].queued
        ),
        (1, 0),
        "the promoted request now holds the slot the holder returned"
    );
    assert_eq!(g.release(promoted), ReleaseOutcome::Released);
    assert_eq!(g.release(other), ReleaseOutcome::Released);

    let end = g.snapshot();
    for (name, c) in &end.classes {
        assert_eq!(
            (c.inflight, c.queued),
            (0, 0),
            "class {name}: everything held at close drained on its own"
        );
        assert_eq!(
            c.admitted_total, c.terminated_total,
            "class {name}: every admission terminated"
        );
    }
    assert_eq!(
        end.classes[&class("c")].admitted_total,
        2,
        "exactly the two pre-close submissions were ever admitted to c"
    );
    assert_eq!(end.conservation_violation(), None);
    assert!(
        g.admission_closed(),
        "draining to zero does not reopen admission"
    );
    refused(
        "submission on the drained governor",
        g.admit(&spec("c", "after")),
    );
}
