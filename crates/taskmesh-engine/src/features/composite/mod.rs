//! Composite task governance (T06): child→root attribution, the recursive
//! admission guard, and reduce/checkpoint contract enforcement.
//!
//! Attribution itself lives in [`crate::engine::state`] (`grant`/`unwind` roll a
//! child's units into its `RootExecutionState`). This module owns the *rules*.

pub mod reduce;

use taskmesh_contract::{
    CheckpointPolicy, GovernorError, HeldCapacity, TaskClass, TaskScope, TaskSpec, TaskStage,
};

use crate::engine::state::{CapacityBlock, GovernedState};
use crate::shared::CapabilityName;

/// Validate the structural shape of a spec before any governance is applied
/// (fail-closed against malformed wire payloads): a spec must have at least one
/// stage, and every stage's `class` must match the task's `class` (the per-stage
/// `class` field is data, but admission governs by `TaskSpec::class`, so an
/// inconsistent per-stage class is a malformed, non-authoritative payload).
pub fn validate_shape(spec: &TaskSpec) -> Result<(), GovernorError> {
    if spec.stages.is_empty() {
        return Err(GovernorError::PolicyViolation(
            "task spec must declare at least one stage".into(),
        ));
    }
    for stage in &spec.stages {
        if stage.class != spec.class {
            return Err(GovernorError::PolicyViolation(
                format!(
                    "stage '{}' declares class '{}' but the task class is '{}'",
                    stage.stage, stage.class, spec.class
                )
                .into(),
            ));
        }
    }
    Ok(())
}

/// The stage a task occupies for recursion accounting and root attribution.
///
/// For a **child** this is its declared `parent_stage` — the lineage point under
/// the root it descends from. This makes `parent_stage` authoritative: two
/// admissions sharing `(root, parent_stage)` are the *same* composite-tree node,
/// independent of which substrate each child happens to run on. (Substrate is
/// *how* work runs, not *which* logical stage it is; keying recursion on the
/// substrate-derived bootstrap stage would let two CPU pipeline stages collide
/// while letting a true loop that alternates substrates slip through.)
///
/// For a **root** it is the first stage. Roots never trip the recursion guard, so
/// this only labels attribution.
pub fn target_stage(spec: &TaskSpec) -> TaskStage {
    match &spec.scope {
        TaskScope::Child { parent_stage, .. } => parent_stage.clone(),
        TaskScope::Root => spec.stages.first().map_or_else(
            || TaskStage::new(spec.class.as_str().to_owned()),
            |s| s.stage.clone(),
        ),
    }
}

// ---- domain (pure) --------------------------------------------------------

/// A child re-entering an already-active `(root, stage)` is a recursive
/// admission loop and must be rejected. Root tasks never trip this guard.
///
/// Scope note: the guard identity is `(root_operation_id, target_stage)` where
/// `target_stage` is the child's declared `parent_stage` (the authoritative
/// lineage point). It is occupancy-based (present/absent), so it cannot
/// distinguish *breadth* (two parallel children of the same root on the same
/// declared stage) from *depth* (a child re-entering its ancestor's stage). Per
/// T06 it rejects the second admission at the same `(root, parent_stage)`.
/// Parallel fan-out is therefore modeled as a single task's `reduce_stage`, not
/// as N `child_of` admissions sharing one stage; distinct logical stages under a
/// root must declare distinct `parent_stage`s. Lineage-aware breadth/depth
/// separation would require per-permit ancestry tracking (out of scope for the
/// startup set).
pub fn is_recursive(state: &GovernedState, spec: &TaskSpec) -> bool {
    // A child's target stage *is* its parent stage (see `target_stage`), so the
    // guard is consulted on borrowed spec data: no allocation under the mutex.
    let TaskScope::Child { parent_stage, .. } = &spec.scope else {
        return false;
    };
    state
        .active_recursion
        .get(spec.root_operation_id.as_str())
        .is_some_and(|stages| stages.contains(parent_stage))
}

/// The declared nested-wait cycle (ADR 0003 D12): a child whose parent awaits
/// it, blocked on capacity that is held *entirely* by root-scoped permits of
/// its own root. That capacity cannot be freed before the child runs, and the
/// child cannot run before it is freed. Returns the capacity, for the verdict.
///
/// Sound, not complete, by construction:
///  * only a *declared* wait counts (`parent_awaits`); lineage alone
///    (`child_of`) says nothing about who waits for whom, and an undeclared
///    wait graph is never inferred;
///  * only the root's **root-scoped** permits count as "cannot end before the
///    child": a sibling child holding a slot may finish on its own, so a block
///    it contributes to is a wait, not a cycle;
///  * the whole block must be the root's — one slot held by a stranger can be
///    released by that stranger.
///
/// `QueuedBehind` is never a cycle (capacity exists; only the order is owed),
/// and an accounting fault is refused on its own terms.
///
/// One pass over the live permits, on a path that is about to queue or shed
/// the request anyway; inflight is bounded by the class limits (H16-009).
pub fn declared_wait_cycle(
    state: &GovernedState,
    spec: &TaskSpec,
    class: &TaskClass,
    capability: Option<&CapabilityName>,
    block: CapacityBlock,
) -> Option<HeldCapacity> {
    let TaskScope::Child {
        parent_awaits: true,
        ..
    } = &spec.scope
    else {
        return None;
    };
    let mut in_class = 0u32;
    let mut in_pool = 0u32;
    let mut cpu_units = 0u128;
    let mut memory_units = 0u128;
    let mut root_permits = 0u32;
    for record in state.permits.values() {
        if record.root_operation_id != spec.root_operation_id
            || !matches!(record.scope, TaskScope::Root)
        {
            continue;
        }
        root_permits += 1;
        if record.class == *class {
            in_class += 1;
        }
        if capability.is_some() && record.capability.as_ref() == capability {
            in_pool += 1;
        }
        cpu_units += u128::from(record.ledger.cpu_units);
        memory_units += u128::from(record.ledger.effective_units);
    }
    if root_permits == 0 {
        return None;
    }
    match block {
        CapacityBlock::Inflight => (in_class > 0 && state.inflight(class) == in_class).then(|| {
            HeldCapacity::ClassInflight {
                class: class.clone(),
            }
        }),
        CapacityBlock::Capability => capability.and_then(|pool| {
            let in_use = state.capability_in_use.get(pool).copied().unwrap_or(0);
            (in_pool > 0 && in_use == in_pool).then(|| HeldCapacity::CapabilityPool {
                pool: pool.to_string(),
            })
        }),
        CapacityBlock::Cpu => {
            (cpu_units > 0 && state.cpu_units_held == cpu_units).then_some(HeldCapacity::CpuBudget)
        }
        CapacityBlock::Memory => (memory_units > 0 && state.memory_units_held == memory_units)
            .then_some(HeldCapacity::MemoryBudget),
        CapacityBlock::Ok | CapacityBlock::QueuedBehind | CapacityBlock::AccountingFault => None,
    }
}

/// Checkpoint metadata is preserved verbatim from the class policy so hook points
/// (before fan-out / every N / before allocation / before stage boundary /
/// before reduce) can inspect it. This is contract metadata, enforced at the
/// host, never mutated by the engine.
pub fn checkpoint_hooks(policy: &CheckpointPolicy) -> CheckpointPolicy {
    *policy
}
