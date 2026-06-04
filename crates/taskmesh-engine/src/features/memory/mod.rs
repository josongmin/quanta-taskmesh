//! Memory governance (T05): permit sizing across `Estimated`/`Measured`/`Hybrid`,
//! reconcile, stage-boundary release, and the leak sweep. Estimated/Measured/
//! Hybrid all have executable semantics — none are name-only.

use taskmesh_contract::{MemoryPermitMode, MemoryUnitScale};

use crate::engine::state::GovernedState;
use crate::shared::{LeakSweepReport, PermitId};

/// Default staleness threshold for the leak sweep (startup-set default).
pub const DEFAULT_LEAK_STALE_MS: u64 = 60_000;

// ---- domain (pure) --------------------------------------------------------

/// The effective memory units a permit should hold once `measured_bytes` are
/// known, given its mode. `Hybrid` keeps the `max(estimate, measured)` invariant.
pub fn effective_units(
    mode: MemoryPermitMode,
    reserved_units: u32,
    measured_bytes: u64,
    scale: MemoryUnitScale,
) -> u32 {
    match mode {
        MemoryPermitMode::Estimated => reserved_units,
        MemoryPermitMode::Measured => scale.units_for(measured_bytes),
        MemoryPermitMode::Hybrid => reserved_units.max(scale.units_for(measured_bytes)),
    }
}

/// Whether a still-held permit looks leaked: untouched past the staleness window.
pub fn is_stale(last_touched_ms: u64, now_ms: u64, stale_after_ms: u64) -> bool {
    last_touched_ms.saturating_add(stale_after_ms) < now_ms
}

// ---- service (stateful) ---------------------------------------------------

/// Reconcile a permit's held memory against a fresh `measured_bytes` reading,
/// adjusting the global held units by the delta. Returns `false` if the permit
/// is unknown or already released.
pub fn reconcile(
    state: &mut GovernedState,
    permit_id: PermitId,
    measured_bytes: u64,
    mode: MemoryPermitMode,
    scale: MemoryUnitScale,
    now_ms: u64,
) -> bool {
    let Some(record) = state.permits.get_mut(&permit_id) else {
        return false;
    };
    if record.ledger.released {
        return false;
    }
    let new_effective = effective_units(mode, record.ledger.reserved_units, measured_bytes, scale);
    let old_effective = record.ledger.effective_units;
    record.ledger.measured_bytes = measured_bytes;
    record.ledger.effective_units = new_effective;
    record.ledger.last_touched_ms = now_ms;

    let scope_is_child = matches!(record.scope, taskmesh_contract::TaskScope::Child { .. });
    let root_id = record.root_operation_id.clone();
    let class = record.class.clone();

    if new_effective >= old_effective {
        let delta = new_effective - old_effective;
        state.memory_units_held = state.memory_units_held.saturating_add(delta);
        if let Some(cs) = state.classes.get_mut(&class) {
            cs.memory_units_held = cs.memory_units_held.saturating_add(delta);
        }
        if scope_is_child {
            if let Some(root) = state.roots.get_mut(&root_id) {
                root.memory_units = root.memory_units.saturating_add(delta);
            }
        }
    } else {
        let delta = old_effective - new_effective;
        state.memory_units_held = state.memory_units_held.saturating_sub(delta);
        if let Some(cs) = state.classes.get_mut(&class) {
            cs.memory_units_held = cs.memory_units_held.saturating_sub(delta);
        }
        if scope_is_child {
            if let Some(root) = state.roots.get_mut(&root_id) {
                root.memory_units = root.memory_units.saturating_sub(delta);
            }
        }
    }
    state.assert_consistent();
    true
}

/// Stage-boundary early release: return `freed_units` of memory to the global
/// pool while keeping the permit (and its root attribution) alive. Returns the
/// number of units actually freed.
pub fn release_stage_memory(
    state: &mut GovernedState,
    permit_id: PermitId,
    freed_units: u32,
) -> u32 {
    let Some(record) = state.permits.get_mut(&permit_id) else {
        return 0;
    };
    if record.ledger.released {
        return 0;
    }
    let freed = freed_units.min(record.ledger.effective_units);
    record.ledger.effective_units -= freed;
    let scope_is_child = matches!(record.scope, taskmesh_contract::TaskScope::Child { .. });
    let root_id = record.root_operation_id.clone();
    let class = record.class.clone();

    state.memory_units_held = state.memory_units_held.saturating_sub(freed);
    if let Some(cs) = state.classes.get_mut(&class) {
        cs.memory_units_held = cs.memory_units_held.saturating_sub(freed);
    }
    if scope_is_child {
        if let Some(root) = state.roots.get_mut(&root_id) {
            root.memory_units = root.memory_units.saturating_sub(freed);
        }
    }
    state.assert_consistent();
    freed
}

/// Sweep outstanding permits for leaks. Only classes that opted into
/// [`MemoryReleasePolicy::LeakDetecting`] are eligible: a stale, non-released
/// permit of such a class is reclaimed (its resources freed) and counted.
/// Classes with other release policies are never force-reclaimed by the sweep.
pub fn reap_leaks(
    state: &mut GovernedState,
    policies: &crate::shared::PolicySet,
    now_ms: u64,
    stale_after_ms: u64,
) -> LeakSweepReport {
    use taskmesh_contract::MemoryReleasePolicy;
    let stale: Vec<PermitId> = state
        .permits
        .values()
        .filter(|r| {
            !r.ledger.released
                && is_stale(r.ledger.last_touched_ms, now_ms, stale_after_ms)
                && policies
                    .class(&r.class)
                    .is_some_and(|p| p.memory_release_policy == MemoryReleasePolicy::LeakDetecting)
        })
        .map(|r| r.permit_id)
        .collect();

    let mut report = LeakSweepReport {
        reclaimed_permits: 0,
        suspected_leaks: u32::try_from(stale.len()).unwrap_or(u32::MAX),
    };
    for permit_id in stale {
        if state.unwind(permit_id).is_some() {
            report.reclaimed_permits += 1;
        }
    }
    report
}
