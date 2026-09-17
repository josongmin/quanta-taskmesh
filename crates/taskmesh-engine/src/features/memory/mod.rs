//! Memory governance (T05): permit sizing across `Estimated`/`Measured`/`Hybrid`,
//! reconcile, stage-boundary release, and the leak sweep.
//!
//! # The three memory quantities (D02)
//!
//! A reservation and a measurement are different facts, and conflating them is
//! how returned capacity comes back to life. The ledger keeps them apart:
//!
//! | quantity | meaning | changes |
//! |---|---|---|
//! | `original_estimate_units` | what the policy reserved | never |
//! | `remaining_reservation_units` | the part still held | shrinks on stage release |
//! | `effective_units` | what is charged now | recomputed from mode |
//!
//! `Estimated` charges the *remaining* reservation, never the original. So
//! `reserve 8 → release 6 → reconcile` charges 2, not 8: a reconcile reports a
//! measurement, it does not re-reserve capacity that was given back.
//!
//! # Stale is not dead (Q16)
//!
//! The sweep reclaims only permits that never reached an executor. A permit
//! whose work is `Running` or `CleanupPending` is *known* to be executing; an
//! untouched lease is evidence of a quiet task, not of a dead one. Reclaiming it
//! would hand its capacity to a second task while the first still uses it, so
//! the sweep reports it as suspected and leaves it charged.

use taskmesh_contract::{
    ExecutionPhase, MemoryPermitMode, MemoryReleasePolicy, MemoryUnitScale, ResourceConversionError,
};

use crate::engine::state::{GovernedState, TerminalReason, TransitionEffects};
use crate::shared::{LeakSweepReport, PermitId, StageReleaseOutcome};

/// Default staleness threshold for the leak sweep (startup-set default).
pub const DEFAULT_LEAK_STALE_MS: u64 = 60_000;

// ---- domain (pure) --------------------------------------------------------

/// The effective memory units a permit should hold once `measured_bytes` are
/// known, given its mode.
///
/// `remaining_reservation_units` — not the original estimate — is the floor, so
/// `Hybrid` keeps `max(remaining reservation, measured)` and `Estimated` follows
/// the reservation that is actually still held.
pub fn effective_units(
    mode: MemoryPermitMode,
    remaining_reservation_units: u32,
    measured_bytes: u64,
    scale: MemoryUnitScale,
) -> Result<u32, ResourceConversionError> {
    Ok(match mode {
        MemoryPermitMode::Estimated => remaining_reservation_units,
        MemoryPermitMode::Measured => scale.units_for(measured_bytes)?,
        MemoryPermitMode::Hybrid => {
            remaining_reservation_units.max(scale.units_for(measured_bytes)?)
        }
    })
}

/// Whether a still-held permit looks leaked: untouched past the staleness window.
pub fn is_stale(last_touched_ms: u64, now_ms: u64, stale_after_ms: u64) -> bool {
    last_touched_ms.saturating_add(stale_after_ms) < now_ms
}

/// Whether a class's release policy permits an early stage-boundary release.
///
/// `OnTaskCompletion` means exactly what it says: units come back when the task
/// completes. Honoring an early release for such a class would make the policy
/// decorative.
pub fn stage_release_allowed(policy: MemoryReleasePolicy) -> bool {
    matches!(
        policy,
        MemoryReleasePolicy::OnStageBoundary | MemoryReleasePolicy::LeakDetecting
    )
}

// ---- service (stateful) ---------------------------------------------------

/// Outcome of a memory reconcile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[must_use = "a rejected or stale reading was not applied; check the outcome"]
pub enum ReconcileOutcome {
    /// The reading was applied; `held_units` is the new effective charge.
    Applied { held_units: u32 },
    /// No such live permit.
    UnknownPermit,
    /// A newer reading has already been applied; this one is discarded.
    StaleEpoch { current_epoch: u64 },
    /// The reading cannot be expressed in memory units.
    ConversionFailed(ResourceConversionError),
}

impl ReconcileOutcome {
    pub fn is_applied(self) -> bool {
        matches!(self, Self::Applied { .. })
    }
}

/// Reconcile a permit's held memory against a fresh `measured_bytes` reading.
///
/// `epoch` orders concurrent reporters: a reading whose epoch does not exceed
/// the last applied one is rejected rather than allowed to overwrite newer
/// truth. `now_ms` must already be clamped to the commit watermark.
pub fn reconcile(
    state: &mut GovernedState,
    permit_id: PermitId,
    measured_bytes: u64,
    epoch: u64,
    mode: MemoryPermitMode,
    scale: MemoryUnitScale,
    now_ms: u64,
) -> ReconcileOutcome {
    let Some(record) = state.permits.get(&permit_id) else {
        return ReconcileOutcome::UnknownPermit;
    };
    if epoch <= record.ledger.measurement_epoch {
        return ReconcileOutcome::StaleEpoch {
            current_epoch: record.ledger.measurement_epoch,
        };
    }
    let new_effective = match effective_units(
        mode,
        record.ledger.remaining_reservation_units,
        measured_bytes,
        scale,
    ) {
        Ok(units) => units,
        Err(error) => return ReconcileOutcome::ConversionFailed(error),
    };

    let record = state
        .permits
        .get_mut(&permit_id)
        .expect("permit presence checked above");
    let old_effective = record.ledger.effective_units;
    record.ledger.measured_bytes = measured_bytes;
    record.ledger.measurement_epoch = epoch;
    record.ledger.effective_units = new_effective;
    // A late arrival must not drag a lease's activity backwards in time.
    record.ledger.last_touched_ms = record.ledger.last_touched_ms.max(now_ms);

    let scope_is_child = matches!(record.scope, taskmesh_contract::TaskScope::Child { .. });
    let root_id = record.root_operation_id.clone();
    let class = record.class.clone();

    apply_memory_delta(
        state,
        &class,
        scope_is_child.then_some(root_id),
        i64::from(new_effective) - i64::from(old_effective),
    );
    state.assert_consistent();
    ReconcileOutcome::Applied {
        held_units: new_effective,
    }
}

/// Stage-boundary early release: return `freed_units` of memory to the global
/// pool while keeping the permit (and its root attribution) alive.
///
/// Enforces the class's [`MemoryReleasePolicy`] (D02) and touches the lease:
/// returning memory is permit activity, and a sweep that ignores it reclaims
/// work that was demonstrably alive moments ago.
///
/// `sequence` deduplicates stage events. `None` disables ordering (the caller
/// asserts single ownership of the release path); `Some(n)` requires a strictly
/// increasing `n`.
pub fn release_stage_memory(
    state: &mut GovernedState,
    permit_id: PermitId,
    freed_units: u32,
    policy: MemoryReleasePolicy,
    sequence: Option<u64>,
    now_ms: u64,
) -> StageReleaseOutcome {
    if !stage_release_allowed(policy) {
        return StageReleaseOutcome::PolicyForbids { policy };
    }
    let Some(record) = state.permits.get_mut(&permit_id) else {
        return StageReleaseOutcome::UnknownPermit;
    };
    if let Some(sequence) = sequence {
        if sequence <= record.ledger.stage_sequence {
            return StageReleaseOutcome::StaleSequence {
                current_sequence: record.ledger.stage_sequence,
            };
        }
        record.ledger.stage_sequence = sequence;
    }

    let freed = freed_units.min(record.ledger.effective_units);
    record.ledger.effective_units -= freed;
    // The reservation shrinks with the release. Without this an `Estimated`
    // reconcile would restore the original estimate and resurrect the units.
    record.ledger.remaining_reservation_units = record
        .ledger
        .remaining_reservation_units
        .saturating_sub(freed);
    // Successful stage activity is a heartbeat, including a zero-unit release:
    // the caller demonstrably reached a stage boundary.
    record.ledger.last_touched_ms = record.ledger.last_touched_ms.max(now_ms);

    let scope_is_child = matches!(record.scope, taskmesh_contract::TaskScope::Child { .. });
    let root_id = record.root_operation_id.clone();
    let class = record.class.clone();

    apply_memory_delta(
        state,
        &class,
        scope_is_child.then_some(root_id),
        -i64::from(freed),
    );
    state.assert_consistent();
    StageReleaseOutcome::Released { freed_units: freed }
}

/// Apply a signed effective-memory delta to class, global, and root ledgers.
/// One function so the three totals can never drift from each other.
///
/// A negative delta larger than a ledger currently holds is not clipped to
/// zero: it means the totals already disagree with the permit that produced
/// it, and a clipped total would keep admitting against a number the engine
/// knows is wrong. It is recorded as a sticky accounting fault instead.
fn apply_memory_delta(
    state: &mut GovernedState,
    class: &taskmesh_contract::TaskClass,
    root_id: Option<String>,
    delta: i64,
) {
    let magnitude = u128::from(delta.unsigned_abs());
    if delta >= 0 {
        state.memory_units_held += magnitude;
        if let Some(cs) = state.classes.get_mut(class) {
            cs.memory_units_held += magnitude;
        }
        if let Some(root_id) = root_id {
            if let Some(root) = state.roots.get_mut(&root_id) {
                root.memory_units += magnitude;
            }
        }
        return;
    }
    let mut underflow = false;
    match state.memory_units_held.checked_sub(magnitude) {
        Some(next) => state.memory_units_held = next,
        None => underflow = true,
    }
    if let Some(cs) = state.classes.get_mut(class) {
        match cs.memory_units_held.checked_sub(magnitude) {
            Some(next) => cs.memory_units_held = next,
            None => underflow = true,
        }
    }
    if let Some(root_id) = root_id {
        if let Some(root) = state.roots.get_mut(&root_id) {
            match root.memory_units.checked_sub(magnitude) {
                Some(next) => root.memory_units = next,
                None => underflow = true,
            }
        }
    }
    if underflow {
        state.record_accounting_fault("memory release exceeds the units a ledger holds");
    }
}

/// Sweep outstanding permits for leaks.
///
/// Eligibility is deliberately narrow: the class must have opted into
/// [`MemoryReleasePolicy::LeakDetecting`], the lease must be stale, **and** the
/// permit must never have been handed to an executor. A stale lease on running
/// work is reported as suspected and left charged — see the module docs.
pub fn reap_leaks(
    state: &mut GovernedState,
    policies: &crate::shared::PolicySet,
    now_ms: u64,
    stale_after_ms: u64,
    effects: &mut TransitionEffects,
) -> LeakSweepReport {
    let mut reclaimable: Vec<PermitId> = Vec::new();
    let mut suspected: u32 = 0;
    let mut retained_active: u32 = 0;
    for record in state.permits.values() {
        let leak_detecting = policies
            .class(&record.class)
            .is_some_and(|p| p.memory_release_policy == MemoryReleasePolicy::LeakDetecting);
        if !leak_detecting || !is_stale(record.ledger.last_touched_ms, now_ms, stale_after_ms) {
            continue;
        }
        suspected = suspected.saturating_add(1);
        if record.phase == ExecutionPhase::DispatchReserved {
            reclaimable.push(record.permit_id);
        } else {
            retained_active = retained_active.saturating_add(1);
        }
    }

    let mut report = LeakSweepReport {
        reclaimed_permits: 0,
        suspected_leaks: suspected,
        retained_active,
    };
    for permit_id in reclaimable {
        if state
            .unwind(permit_id, TerminalReason::Reclaimed, effects)
            .is_some()
        {
            report.reclaimed_permits += 1;
        }
    }
    report
}
