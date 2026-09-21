//! Unified current-state admission resolver.

use taskmesh_contract::{HeldCapacity, TaskClass, TaskScope, TaskSpec};

use crate::engine::state::{CapacityBlock, GovernedState, PendingRequest};
use crate::shared::{CapabilityId, CapabilityRequirementSet, PolicySet, ResolvedCost};

/// Every capacity dimension blocking a request at the same state revision.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlockerSet {
    pub class_inflight: bool,
    pub capabilities: Vec<CapabilityId>,
    pub cpu: bool,
    pub memory: bool,
    pub queued_behind: bool,
    pub accounting_fault: bool,
}

impl BlockerSet {
    pub fn is_empty(&self) -> bool {
        !self.class_inflight
            && self.capabilities.is_empty()
            && !self.cpu
            && !self.memory
            && !self.queued_behind
            && !self.accounting_fault
    }

    pub fn primary(&self) -> CapacityBlock {
        if self.accounting_fault {
            CapacityBlock::AccountingFault
        } else if self.class_inflight {
            CapacityBlock::Inflight
        } else if !self.capabilities.is_empty() {
            CapacityBlock::Capability
        } else if self.cpu {
            CapacityBlock::Cpu
        } else if self.memory {
            CapacityBlock::Memory
        } else if self.queued_behind {
            CapacityBlock::QueuedBehind
        } else {
            CapacityBlock::Ok
        }
    }
}

/// Exact immediate-parent evidence for an irreversible wait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleWitness {
    pub parent_operation_id: String,
    pub held: Vec<HeldCapacity>,
}

/// One authoritative capacity answer used by intake, promotion, and
/// diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapacityAssessment {
    Runnable,
    ReversiblyBlocked(BlockerSet),
    IrreversibleWaitCycle(CycleWitness),
}

impl CapacityAssessment {
    pub fn blockers(&self) -> Option<&BlockerSet> {
        match self {
            Self::Runnable | Self::IrreversibleWaitCycle(_) => None,
            Self::ReversiblyBlocked(blockers) => Some(blockers),
        }
    }
}

/// Calculate all current capacity blockers. No priority short-circuiting.
pub fn blockers_for(
    state: &GovernedState,
    policies: &PolicySet,
    class: &TaskClass,
    max_inflight: u32,
    cost: ResolvedCost,
    capabilities: &CapabilityRequirementSet,
) -> BlockerSet {
    let mut blockers = BlockerSet::default();
    if state.accounting_fault.is_some() {
        blockers.accounting_fault = true;
    }
    blockers.class_inflight = state.inflight(class) >= max_inflight;
    for id in capabilities.iter() {
        let Some(record) = policies.capability_record(id) else {
            blockers.accounting_fault = true;
            continue;
        };
        let limit = record.capacity().limit();
        if limit != 0 && state.capability_in_use(id) >= limit {
            blockers.capabilities.push(id.clone());
        }
    }
    let budget = &policies.resources;
    if budget.max_cpu_units != 0 {
        blockers.cpu = state
            .cpu_units_held
            .checked_add(u128::from(cost.cpu_units))
            .map_or(true, |next| next > u128::from(budget.max_cpu_units));
    }
    if budget.max_memory_units != 0 {
        blockers.memory = state
            .memory_units_held
            .checked_add(u128::from(cost.memory_units))
            .map_or(true, |next| next > u128::from(budget.max_memory_units));
    }
    blockers
}

#[derive(Clone, Copy)]
pub struct AssessmentRequest<'a> {
    pub spec: &'a TaskSpec,
    pub class: &'a TaskClass,
    pub max_inflight: u32,
    pub cost: ResolvedCost,
    pub capabilities: &'a CapabilityRequirementSet,
    pub queued_behind: bool,
}

pub fn assess(
    state: &GovernedState,
    policies: &PolicySet,
    request: AssessmentRequest<'_>,
) -> CapacityAssessment {
    assess_scope(
        state,
        policies,
        ScopeAssessment {
            root_operation_id: &request.spec.root_operation_id,
            scope: &request.spec.scope,
            class: request.class,
            max_inflight: request.max_inflight,
            cost: request.cost,
            capabilities: request.capabilities,
            queued_behind: request.queued_behind,
        },
    )
}

pub fn assess_pending(
    state: &GovernedState,
    policies: &PolicySet,
    request: &PendingRequest,
    max_inflight: u32,
    queued_behind: bool,
) -> CapacityAssessment {
    assess_scope(
        state,
        policies,
        ScopeAssessment {
            root_operation_id: &request.root_operation_id,
            scope: &request.scope,
            class: &request.class,
            max_inflight,
            cost: request.cost,
            capabilities: &request.capabilities,
            queued_behind,
        },
    )
}

#[derive(Clone, Copy)]
struct ScopeAssessment<'a> {
    root_operation_id: &'a str,
    scope: &'a TaskScope,
    class: &'a TaskClass,
    max_inflight: u32,
    cost: ResolvedCost,
    capabilities: &'a CapabilityRequirementSet,
    queued_behind: bool,
}

fn assess_scope(
    state: &GovernedState,
    policies: &PolicySet,
    request: ScopeAssessment<'_>,
) -> CapacityAssessment {
    let mut blockers = blockers_for(
        state,
        policies,
        request.class,
        request.max_inflight,
        request.cost,
        request.capabilities,
    );
    blockers.queued_behind = request.queued_behind;
    if blockers.is_empty() {
        return CapacityAssessment::Runnable;
    }
    if let Some(witness) = cycle_witness(
        state,
        policies,
        request.root_operation_id,
        request.scope,
        request.class,
        &blockers,
    ) {
        return CapacityAssessment::IrreversibleWaitCycle(witness);
    }
    CapacityAssessment::ReversiblyBlocked(blockers)
}

fn cycle_witness(
    state: &GovernedState,
    policies: &PolicySet,
    root_operation_id: &str,
    scope: &TaskScope,
    class: &TaskClass,
    blockers: &BlockerSet,
) -> Option<CycleWitness> {
    let TaskScope::Child {
        parent_operation_id,
        parent_awaits: true,
        ..
    } = scope
    else {
        return None;
    };
    let parent_id = state.permits_for_operation(root_operation_id, parent_operation_id)?;

    let mut parent_in_class = 0u32;
    let mut parent_cpu = 0u128;
    let mut parent_memory = 0u128;
    let mut parent_capabilities = std::collections::BTreeMap::<&CapabilityId, u32>::new();
    if let Some(record) = state.permits.get(&parent_id) {
        if record.class == *class {
            parent_in_class += 1;
        }
        parent_cpu += u128::from(record.ledger.cpu_units);
        parent_memory += u128::from(record.ledger.effective_units);
        for id in record.capabilities.iter() {
            *parent_capabilities.entry(id).or_default() += 1;
        }
    }

    let mut held = Vec::new();
    if blockers.class_inflight && parent_in_class > 0 && state.inflight(class) == parent_in_class {
        held.push(HeldCapacity::ClassInflight {
            class: class.clone(),
        });
    }
    for id in &blockers.capabilities {
        let by_parent = parent_capabilities.get(id).copied().unwrap_or(0);
        if by_parent > 0 && state.capability_in_use(id) == by_parent {
            let pool = policies.capability_record(id)?.name().to_owned();
            held.push(HeldCapacity::CapabilityPool { pool });
        }
    }
    if blockers.cpu && parent_cpu > 0 && state.cpu_units_held == parent_cpu {
        held.push(HeldCapacity::CpuBudget);
    }
    if blockers.memory && parent_memory > 0 && state.memory_units_held == parent_memory {
        held.push(HeldCapacity::MemoryBudget);
    }
    (!held.is_empty()).then(|| CycleWitness {
        parent_operation_id: parent_operation_id.clone(),
        held,
    })
}

/// Whether a request has an earlier same-domain predecessor in its class queue.
pub fn queued_behind_index(
    queue: &std::collections::VecDeque<PendingRequest>,
    index: usize,
) -> bool {
    let Some(request) = queue.get(index) else {
        // An unknown position must not bypass an earlier same-domain request.
        return true;
    };
    queue
        .iter()
        .take(index)
        .any(|earlier| earlier.capabilities.intersects(&request.capabilities))
}

#[cfg(test)]
mod tests {
    use super::{cycle_witness, queued_behind_index, BlockerSet};
    use crate::engine::state::{ClassState, GovernedState};
    use crate::shared::{CapabilityRequirementSet, PolicySet, Provenance, ResolvedCapability};
    use std::collections::BTreeMap;
    use taskmesh_contract::{
        ClassPolicy, ResourceBudget, TaskClass, TaskScope, TaskSpec, TaskStage,
    };

    #[test]
    fn an_unknown_queue_position_fails_closed_as_queued_behind() {
        assert!(
            queued_behind_index(&std::collections::VecDeque::new(), 0),
            "an index outside the frozen queue must not become runnable"
        );
    }

    #[test]
    fn unrelated_capacity_cannot_be_reported_as_held_by_the_waiting_parent() {
        let requested_class = TaskClass::new("child");
        let parent_class = TaskClass::new("parent");
        let policy = PolicySet::new(
            ResourceBudget::new().cpu_units(1).memory_units(1),
            BTreeMap::from([(requested_class.clone(), ClassPolicy::new().max_inflight(1))]),
        )
        .with_capability_limits(BTreeMap::from([("cpu".to_owned(), 1)]))
        .expect("built-in cpu capability");
        let capability = policy.resolve_capability("cpu").expect("registered");
        assert!(
            matches!(capability, ResolvedCapability::Registered(_)),
            "cpu is a governed built-in capability"
        );
        let ResolvedCapability::Registered(capability) = capability else {
            return;
        };

        let parent_spec = TaskSpec::io(parent_class.clone()).operation("parent");
        let mut state = GovernedState::default();
        // Admit the exact parent with zero cost, then inject unrelated pressure.
        // The cycle detector must attribute only capacity held by this parent.
        state.grant(crate::engine::state::GrantRequest {
            permit_id: 7,
            class: &parent_class,
            operation: "parent",
            root_operation_id: "root",
            scope: TaskScope::Root,
            target_stage: TaskStage::new("io"),
            provenance: Provenance::of(&parent_spec),
            capabilities: CapabilityRequirementSet::empty(),
            cost: crate::shared::ResolvedCost::default(),
            reserved_units: 0,
            now_ms: 0,
            pending_ticket: None,
            claim_waker: None,
        });
        state.classes.insert(
            requested_class.clone(),
            ClassState {
                inflight: 1,
                ..ClassState::default()
            },
        );
        state.cpu_units_held = 1;
        state.memory_units_held = 1;
        state.capability_in_use.insert(capability.clone(), 1);

        let blockers = BlockerSet {
            class_inflight: true,
            capabilities: vec![capability],
            cpu: true,
            memory: true,
            ..BlockerSet::default()
        };
        let scope = TaskScope::Child {
            parent_operation_id: "parent".to_owned(),
            parent_stage: TaskStage::new("io"),
            parent_awaits: true,
        };
        assert_eq!(
            cycle_witness(&state, &policy, "root", &scope, &requested_class, &blockers,),
            None,
            "capacity held only by unrelated work is reversible for this parent"
        );
    }
}
