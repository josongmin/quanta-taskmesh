use super::{has_runnable, rebase_virtual_time, select, WFQ_SCALE};
use crate::engine::state::{CapacityBlock, ClassState, GovernedState, PendingRequest};
use crate::shared::{
    CapabilityRequirementSet, PermitId, PolicySet, Provenance, RequestKey, ResolvedCost, Ticket,
};
use std::collections::BTreeMap;
use taskmesh_contract::{
    ClassPolicy, FairnessPolicy, ResourceBudget, TaskClass, TaskScope, TaskSpec, TaskStage,
};

fn pending_wfq(
    class: &TaskClass,
    operation: &str,
    seq_no: u64,
    cpu_units: u32,
    finish_tag: u128,
    capabilities: CapabilityRequirementSet,
) -> PendingRequest {
    let spec = TaskSpec::io(class.clone()).operation(operation);
    PendingRequest {
        permit_id: PermitId::new(1, seq_no),
        ticket: Ticket::new(1, seq_no),
        seq_no,
        class: class.clone(),
        request_key: RequestKey::from_root(operation),
        operation: operation.to_owned(),
        root_operation_id: operation.to_owned(),
        scope: TaskScope::Root,
        parent_permit_id: None,
        target_stage: TaskStage::new("io"),
        provenance: Provenance::of(&spec),
        cost: ResolvedCost {
            cpu_units,
            memory_units: 0,
        },
        capabilities,
        enqueued_at_ms: 0,
        deadline_ms: u64::MAX,
        finish_tag,
        wfq_arrival_tag: 0,
        blocked_on: CapacityBlock::Cpu,
        waker: None,
    }
}

#[test]
fn has_runnable_is_false_when_every_class_queue_is_empty() {
    let class = TaskClass::new("idle");
    let policies = PolicySet::new(
        ResourceBudget::new(),
        BTreeMap::from([(class.clone(), ClassPolicy::new())]),
    );
    let mut state = GovernedState::default();
    state.classes.insert(class, ClassState::default());

    assert!(
        !has_runnable(&state, &policies),
        "an empty active-class ring cannot owe a promotion continuation"
    );
}

#[test]
fn virtual_time_rebase_preserves_relative_finish_distances() {
    let mut state = GovernedState::default();
    state.virtual_time = 40;
    state.classes.insert(
        TaskClass::new("a"),
        ClassState {
            last_finish_tag: 55,
            ..ClassState::default()
        },
    );
    state.classes.insert(
        TaskClass::new("b"),
        ClassState {
            last_finish_tag: 75,
            ..ClassState::default()
        },
    );

    rebase_virtual_time(&mut state);

    assert_eq!(state.virtual_time, 0);
    assert_eq!(state.classes[&TaskClass::new("a")].last_finish_tag, 15);
    assert_eq!(state.classes[&TaskClass::new("b")].last_finish_tag, 35);
}

#[test]
fn wfq_rebased_head_dispatches_with_its_stored_finish_tag() {
    // This head was queued before other classes advanced virtual time.
    // Once the global clock rebases, its older tag is zero; recomputing a
    // head as if it were a later runnable follower adds a false service
    // quantum and changes cross-class dispatch order.
    let class = TaskClass::new("older-head");
    let policy = ClassPolicy::new()
        .max_inflight(2)
        .fairness(FairnessPolicy::WeightedFairQueue {
            weight: 1,
            burst: 0,
        });
    let policies = PolicySet::new(
        ResourceBudget::new(),
        BTreeMap::from([(class.clone(), policy)]),
    );
    let request = pending_wfq(
        &class,
        "older-head",
        1,
        1,
        WFQ_SCALE,
        CapabilityRequirementSet::empty(),
    );
    let mut state = GovernedState::default();
    state.virtual_time = 3 * WFQ_SCALE;
    state.classes.insert(
        class.clone(),
        ClassState {
            queue: std::collections::VecDeque::from([request]),
            last_finish_tag: WFQ_SCALE,
            ..ClassState::default()
        },
    );

    rebase_virtual_time(&mut state);
    assert_eq!(state.classes[&class].queue[0].finish_tag, 0);
    assert_eq!(state.classes[&class].last_served_finish_tag, 0);
    assert_eq!(
        select(&mut state, &policies),
        Some(super::Selection {
            class,
            queue_index: 0,
            service_finish_tag: 0,
        })
    );
}

#[test]
fn wfq_runnable_follower_excludes_blocked_head_debt() {
    // One CPU unit is already held. The two-unit head cannot run, while a
    // one-unit follower on another capability domain can bypass it. The
    // follower's service tag starts at the last *served* tag, not at the
    // finish tag it inherited behind the still-unserved head.
    let class = TaskClass::new("follower");
    let policy = ClassPolicy::new()
        .max_inflight(2)
        .fairness(FairnessPolicy::WeightedFairQueue {
            weight: 1,
            burst: 0,
        });
    let policies = PolicySet::new(
        ResourceBudget::new().cpu_units(2),
        BTreeMap::from([(class.clone(), policy)]),
    );
    let cpu_capability = CapabilityRequirementSet::from_resolved([policies
        .resolve_capability("cpu")
        .expect("built-in pool")])
    .expect("one registered capability");
    let mut state = GovernedState::default();
    state.cpu_units_held = 1;
    state.classes.insert(
        class.clone(),
        ClassState {
            inflight: 1,
            queue: std::collections::VecDeque::from([
                pending_wfq(&class, "blocked-head", 1, 2, 2 * WFQ_SCALE, cpu_capability),
                pending_wfq(
                    &class,
                    "runnable-follower",
                    2,
                    1,
                    3 * WFQ_SCALE,
                    CapabilityRequirementSet::empty(),
                ),
            ]),
            last_finish_tag: 3 * WFQ_SCALE,
            ..ClassState::default()
        },
    );

    assert_eq!(
        select(&mut state, &policies),
        Some(super::Selection {
            class,
            queue_index: 1,
            service_finish_tag: WFQ_SCALE,
        })
    );
}
