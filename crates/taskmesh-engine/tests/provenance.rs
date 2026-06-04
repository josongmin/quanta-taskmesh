//! Audit [P2]: classification provenance (source/reason) must survive intake and
//! stay auditable in runtime state — admitted directly and via the queue.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::*;
use taskmesh_engine::*;

fn gov(list: Vec<(&'static str, ClassPolicy)>) -> Governor {
    let classes: BTreeMap<TaskClass, ClassPolicy> = list
        .into_iter()
        .map(|(n, p)| (TaskClass::new(n), p))
        .collect();
    Governor::new_unchecked(
        PolicySet::new(ResourceBudget::new().cpu_units(100), classes),
        Arc::new(ManualClock::new(0)),
    )
}

fn spec_with(
    class: &str,
    op: &str,
    source: PlanSource,
    reason: ClassificationRationale,
) -> TaskSpec {
    let mut spec = TaskSpec::blocking(TaskClass::new(class.to_string())).operation(op.to_string());
    spec.source = source;
    spec.reason = reason;
    spec
}

#[test]
fn provenance_is_auditable_for_an_admitted_permit() {
    let g = gov(vec![("c", ClassPolicy::new().max_inflight(4).cpu_units(1))]);
    let spec = spec_with(
        "c",
        "op",
        PlanSource::SearchAdapter,
        ClassificationRationale::DerivedFromRequestKind,
    );
    let permit = match g.admit(&spec, RequestKey::new("op")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("{o:?}"),
    };
    let prov = g.permit_provenance(permit).expect("provenance recorded");
    assert_eq!(prov.source, PlanSource::SearchAdapter);
    assert_eq!(prov.reason, ClassificationRationale::DerivedFromRequestKind);

    // Gone after release (no longer a live permit).
    g.release(permit);
    assert!(g.permit_provenance(permit).is_none());
}

#[test]
fn provenance_survives_the_queue_promotion_path() {
    let g = gov(vec![(
        "c",
        ClassPolicy::new()
            .max_inflight(1)
            .max_queue_depth(4)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    )]);
    let occupy = match g.admit(
        &spec_with("c", "occ", PlanSource::Internal, ClassificationRationale::ExplicitMapping),
        RequestKey::new("occ"),
    ) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("{o:?}"),
    };
    // This one must queue, carrying its provenance into the pending record.
    let queued_spec = spec_with(
        "c",
        "q",
        PlanSource::Indexing,
        ClassificationRationale::DerivedFromStageMap,
    );
    let ticket = match g.admit(&queued_spec, RequestKey::new("q")) {
        AdmissionDecision::Queued { ticket } => ticket,
        o => panic!("{o:?}"),
    };

    g.release(occupy); // promotes the queued request
    let permit = g.claim(ticket).expect("promoted");
    let prov = g.permit_provenance(permit).expect("provenance preserved through promotion");
    assert_eq!(prov.source, PlanSource::Indexing);
    assert_eq!(prov.reason, ClassificationRationale::DerivedFromStageMap);
}
