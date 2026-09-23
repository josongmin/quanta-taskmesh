//! T06-adjacent: classification provenance (source + reason) must survive
//! queue → promote → claim,
//! where the engine rebuilds a permit from a `PendingRequest`. Regression guard
//! for the provenance ledger: each promoted permit must carry *its own*
//! provenance, never the filler's and never a neighbor's (no cross-contamination).

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    ClassPolicy, ClassificationRationale, ManualClock, OverflowPolicy, PlanSource, ResourceBudget,
    TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, ClaimOutcome, Governor, PolicySet, Provenance, ReleaseOutcome,
};

fn spec_with(op: &str, source: PlanSource, reason: ClassificationRationale) -> TaskSpec {
    let mut s = TaskSpec::blocking(TaskClass::new("c")).operation(op.to_string());
    s.source = source;
    s.reason = reason;
    s
}

fn governor(max_inflight: u32, cpu_budget: u32) -> Governor {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("c"),
        ClassPolicy::new()
            .max_inflight(max_inflight)
            .max_queue_depth(16)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    );
    let resources = ResourceBudget::new()
        .cpu_units(cpu_budget)
        .memory_units(1_000_000);
    Governor::new_unchecked(
        PolicySet::new(resources, classes),
        Arc::new(ManualClock::new(1000)),
    )
}

#[test]
fn promotion_preserves_each_requests_own_provenance() {
    // Single slot: the filler holds it while three distinctly-tagged requests
    // queue. Draining must hand each promoted permit *its own* provenance.
    let g = governor(1, 1);

    let filler = match g.admit(&spec_with(
        "filler",
        PlanSource::INTERNAL,
        ClassificationRationale::ExplicitMapping,
    )) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("filler must admit, got {o:?}"),
    };

    let tagged = [
        (
            "q0",
            PlanSource::new("public-sdk").expect("valid source"),
            ClassificationRationale::ExplicitMapping,
        ),
        (
            "q1",
            PlanSource::new("warmup").expect("valid source"),
            ClassificationRationale::DerivedFromRequestKind,
        ),
        (
            "q2",
            PlanSource::new("indexing").expect("valid source"),
            ClassificationRationale::DerivedFromStageMap,
        ),
    ];
    let mut queued: Vec<(u64, Provenance)> = Vec::new();
    for (op, source, reason) in tagged {
        let spec = spec_with(op, source.clone(), reason);
        match g.admit(&spec) {
            AdmissionDecision::Queued { ticket } => {
                queued.push((ticket, Provenance { source, reason }));
            }
            o => panic!("{op} must queue, got {o:?}"),
        }
    }

    // Drain: release the holder, claim whoever was promoted, and verify it kept
    // exactly its own provenance.
    let mut current = filler;
    let mut checked = 0;
    loop {
        assert_eq!(g.release(current), ReleaseOutcome::Released);
        let mut found = None;
        for (idx, (ticket, prov)) in queued.iter().enumerate() {
            match g.claim(*ticket) {
                ClaimOutcome::Ready(permit) => {
                    found = Some((idx, permit, prov.clone()));
                    break;
                }
                ClaimOutcome::Pending => {}
                other => panic!("queued provenance ticket must not terminate: {other:?}"),
            }
        }
        let Some((idx, permit, expected)) = found else {
            break;
        };
        assert_eq!(
            g.permit_provenance(permit),
            Some(expected),
            "a promoted permit must carry its own provenance, uncontaminated"
        );
        queued.remove(idx);
        current = permit;
        checked += 1;
    }
    // The loop released `current` before finding nothing further to promote.
    assert_eq!(g.release(current), ReleaseOutcome::UnknownPermit);

    assert_eq!(
        checked, 3,
        "all three queued requests must promote and verify"
    );
    assert!(queued.is_empty(), "no queued provenance left unaccounted");
}

#[test]
fn unknown_permit_has_no_provenance() {
    let g = governor(4, 0);
    assert_eq!(g.permit_provenance(404), None);
}
