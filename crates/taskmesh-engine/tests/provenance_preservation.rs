//! T06-adjacent: classification provenance (source + reason) must survive every
//! admission path intact — direct admit, and especially queue → promote → claim,
//! where the engine rebuilds a permit from a `PendingRequest`. Regression guard
//! for the provenance ledger: each promoted permit must carry *its own*
//! provenance, never the filler's and never a neighbor's (no cross-contamination).

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    ClassPolicy, ClassificationRationale, ManualClock, OverflowPolicy, PlanSource, ResourceBudget,
    TaskClass, TaskSpec,
};
use taskmesh_engine::{AdmissionDecision, Governor, PolicySet, Provenance};

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
fn direct_admit_preserves_provenance_and_clears_on_release() {
    let g = governor(4, 0);
    let spec = spec_with(
        "x",
        PlanSource::SearchAdapter,
        ClassificationRationale::DerivedFromStageMap,
    );
    let permit = match g.admit(&spec) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("must admit, got {o:?}"),
    };
    assert_eq!(
        g.permit_provenance(permit),
        Some(Provenance::of(&spec)),
        "admitted permit must carry the spec's provenance"
    );
    g.release(permit);
    assert_eq!(
        g.permit_provenance(permit),
        None,
        "released permit has no provenance"
    );
}

#[test]
fn promotion_preserves_each_requests_own_provenance() {
    // Single slot: the filler holds it while three distinctly-tagged requests
    // queue. Draining must hand each promoted permit *its own* provenance.
    let g = governor(1, 1);

    let filler = match g.admit(&spec_with(
        "filler",
        PlanSource::Internal,
        ClassificationRationale::ExplicitMapping,
    )) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("filler must admit, got {o:?}"),
    };

    let tagged = [
        (
            "q0",
            PlanSource::PublicSdk,
            ClassificationRationale::ExplicitMapping,
        ),
        (
            "q1",
            PlanSource::Warmup,
            ClassificationRationale::DerivedFromRequestKind,
        ),
        (
            "q2",
            PlanSource::Indexing,
            ClassificationRationale::DerivedFromStageMap,
        ),
    ];
    let mut queued: Vec<(u64, Provenance)> = Vec::new();
    for (op, source, reason) in tagged {
        let spec = spec_with(op, source, reason);
        match g.admit(&spec) {
            AdmissionDecision::Queued { ticket } => queued.push((ticket, Provenance::of(&spec))),
            o => panic!("{op} must queue, got {o:?}"),
        }
    }

    // Drain: release the holder, claim whoever was promoted, and verify it kept
    // exactly its own provenance.
    let mut current = filler;
    let mut checked = 0;
    loop {
        g.release(current);
        let mut found = None;
        for (idx, (ticket, prov)) in queued.iter().enumerate() {
            if let Some(permit) = g.claim(*ticket) {
                found = Some((idx, permit, *prov));
                break;
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
    g.release(current);

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
