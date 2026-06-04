//! Composite task governance (T06): child→root attribution, the recursive
//! admission guard, and reduce/checkpoint contract enforcement.
//!
//! Attribution itself lives in [`crate::engine::state`] (`grant`/`unwind` roll a
//! child's units into its `RootExecutionState`). This module owns the *rules*.

pub mod reduce;

use taskmesh_contract::{CheckpointPolicy, GovernorError, TaskScope, TaskSpec, TaskStage};

use crate::engine::state::GovernedState;

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
        TaskScope::Child { parent_stage } => parent_stage.clone(),
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
    if !matches!(spec.scope, TaskScope::Child { .. }) {
        return false;
    }
    let key = (spec.root_operation_id.clone(), target_stage(spec));
    state.active_recursion.contains(&key)
}

/// Checkpoint metadata is preserved verbatim from the class policy so hook points
/// (before fan-out / every N / before allocation / before stage boundary /
/// before reduce) can inspect it. This is contract metadata, enforced at the
/// host, never mutated by the engine.
pub fn checkpoint_hooks(policy: &CheckpointPolicy) -> CheckpointPolicy {
    *policy
}
