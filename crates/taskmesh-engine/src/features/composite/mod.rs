//! Composite task governance (T06): child→root attribution, the recursive
//! admission guard, and reduce/checkpoint contract enforcement.
//!
//! Attribution itself lives in [`crate::engine::state`] (`grant`/`unwind` roll a
//! child's units into its `RootExecutionState`). This module owns the *rules*.

pub mod reduce;

use taskmesh_contract::{CheckpointPolicy, TaskScope, TaskSpec, TaskStage};

use crate::engine::state::GovernedState;

/// The stage a task occupies for recursion accounting: its first stage.
pub fn target_stage(spec: &TaskSpec) -> TaskStage {
    spec.stages
        .first()
        .map(|s| s.stage.clone())
        .unwrap_or_else(|| TaskStage::new(spec.class.as_str().to_string()))
}

// ---- domain (pure) --------------------------------------------------------

/// A child re-entering an already-active `(root, stage)` is a recursive
/// admission loop and must be rejected. Root tasks never trip this guard.
///
/// Scope note: the guard identity is `(root_operation_id, target_stage)` and is
/// occupancy-based (present/absent), so it cannot distinguish *breadth* (two
/// parallel children of the same root on the same stage) from *depth* (a child
/// re-entering its ancestor's stage). Per T06 it rejects the second admission at
/// the same `(root, stage)`. Parallel fan-out is therefore modeled as a single
/// task's `reduce_stage`, not as N `child_of` admissions sharing one stage.
/// Lineage-aware breadth/depth separation would require per-permit ancestry
/// tracking (out of scope for the startup set).
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
