//! Governor memory governance transitions.

use super::{
    memory, Governor, LeakSweepReport, MeasurementSequence, PermitId, ReconcileOutcome,
    StageReleaseOutcome, TransitionEffects,
};

impl Governor {
    // ---- memory governance (T05) -----------------------------------------

    /// Reconcile a permit's held memory against a measured byte reading.
    ///
    /// A *downward* reconcile frees memory, so queued work is promoted. An
    /// *upward* reconcile records measured reality even if it pushes held memory
    /// past `max_memory_units`; the cap is then enforced fail-closed for *new*
    /// admissions, so an overcommitting in-flight task cannot let new work in.
    /// Observed overage is real memory: it is recorded as pressure, never
    /// discarded to keep the total under the cap.
    /// An unrepresentable reading reports `ConversionFailed` and sets a sticky
    /// accounting fault. Queued tickets receive `AccountingFault` terminals and
    /// their waiters are woken; new admission and promotion then stop, including
    /// after the measured permit is released. Recovery requires draining and
    /// replacing this Governor rather than guessing a safe charge.
    ///
    /// The epoch is assigned *inside* the same transition that applies the
    /// reading (the next one after whatever is currently recorded). Reading the
    /// epoch in one critical section and applying under another would let two
    /// unordered reporters pick the same "next" epoch and have the later commit
    /// rejected as stale — a lost measurement dressed up as ordering.
    pub fn reconcile_memory(&self, permit_id: PermitId, measured_bytes: u64) -> ReconcileOutcome {
        self.reconcile_memory_inner(permit_id, measured_bytes, None)
    }

    /// Reconcile with an explicit measurement epoch.
    ///
    /// Concurrent reporters need an order. Without one, a reading that was taken
    /// first but committed last silently overwrites newer truth — including the
    /// lease activity timestamp, which then makes live work look stale.
    pub fn reconcile_memory_at(
        &self,
        permit_id: PermitId,
        measured_bytes: u64,
        epoch: u64,
    ) -> ReconcileOutcome {
        self.reconcile_memory_inner(permit_id, measured_bytes, Some(epoch))
    }

    fn reconcile_memory_inner(
        &self,
        permit_id: PermitId,
        measured_bytes: u64,
        epoch: Option<u64>,
    ) -> ReconcileOutcome {
        let sampled = self.sample_clock_ms();
        let mut effects = TransitionEffects::default();
        let outcome = {
            let mut state = self.state.lock();
            let now = state.commit_time(sampled);
            let Some(record) = state.permits.get(&permit_id) else {
                return ReconcileOutcome::UnknownPermit;
            };
            let Some(policy) = self.policy.class(&record.class) else {
                return ReconcileOutcome::UnknownPermit;
            };
            let sequence = match epoch {
                Some(epoch) => MeasurementSequence::from_raw(epoch),
                None => match record.ledger.measurement_sequence.checked_next() {
                    Ok(sequence) => sequence,
                    Err(outcome) => return outcome,
                },
            };
            let mode = policy.memory_permit_mode;
            let scale = self.policy.resources.memory_unit_scale;
            let outcome = memory::reconcile(
                &mut state,
                permit_id,
                measured_bytes,
                sequence,
                mode,
                scale,
                now,
            );
            if matches!(outcome, ReconcileOutcome::ConversionFailed(_)) {
                self.terminalize_queued_for_accounting_fault(&mut state, &mut effects);
            } else if outcome.is_applied() {
                // Promote unconditionally on success: a downward reconcile freed
                // budget; an upward one leaves no capacity so promote is a no-op.
                let pass = self.promote(&mut state, now);
                effects.absorb(pass);
            } else {
                // Refused or stale observations did not change capacity.
            }
            outcome
        };
        self.drain_effects(effects, sampled);
        outcome
    }

    /// Return `freed_units` of a still-held permit's memory at a stage boundary.
    ///
    /// Governed by the class's [`taskmesh_contract::MemoryReleasePolicy`] (D02): an
    /// `OnTaskCompletion` class does not get an early release, and being told so
    /// is the point — folding a policy refusal into "freed 0 units" is
    /// indistinguishable from a permit that held nothing.
    pub fn release_stage_memory(
        &self,
        permit_id: PermitId,
        freed_units: u32,
    ) -> StageReleaseOutcome {
        self.release_stage_memory_inner(permit_id, freed_units, None)
    }

    /// Stage release with an explicit, strictly increasing sequence number.
    /// A duplicate or reordered stage event is rejected instead of double-applied.
    pub fn release_stage_memory_seq(
        &self,
        permit_id: PermitId,
        freed_units: u32,
        sequence: u64,
    ) -> StageReleaseOutcome {
        self.release_stage_memory_inner(permit_id, freed_units, Some(sequence))
    }

    fn release_stage_memory_inner(
        &self,
        permit_id: PermitId,
        freed_units: u32,
        sequence: Option<u64>,
    ) -> StageReleaseOutcome {
        let sampled = self.sample_clock_ms();
        let mut effects = TransitionEffects::default();
        let outcome = {
            let mut state = self.state.lock();
            let now = state.commit_time(sampled);
            let Some(record) = state.permits.get(&permit_id) else {
                return StageReleaseOutcome::UnknownPermit;
            };
            let Some(policy) = self.policy.class(&record.class) else {
                return StageReleaseOutcome::UnknownPermit;
            };
            let release_policy = policy.memory_release_policy;
            let outcome = memory::release_stage_memory(
                &mut state,
                permit_id,
                freed_units,
                release_policy,
                sequence,
                now,
            );
            // Promotion is safe on a no-op release and also repairs any
            // pending runnable queue. Avoid a second, subtly different
            // resource-change predicate at this composition boundary.
            let pass = self.promote(&mut state, now);
            effects.absorb(pass);
            outcome
        };
        self.drain_effects(effects, sampled);
        outcome
    }

    /// Sweep for leaked permits using the default staleness window.
    pub fn reap_leaks(&self) -> LeakSweepReport {
        self.reap_leaks_with(memory::DEFAULT_LEAK_STALE_MS)
    }

    /// Sweep for leaked permits older than `stale_after_ms`.
    ///
    /// Only permits that never reached an executor are reclaimed; a stale lease
    /// on running work is reported in `retained_active` and left charged.
    pub fn reap_leaks_with(&self, stale_after_ms: u64) -> LeakSweepReport {
        let sampled = self.sample_clock_ms();
        let mut effects = TransitionEffects::default();
        let report = {
            let mut state = self.state.lock();
            let now = state.commit_time(sampled);
            let report =
                memory::reap_leaks(&mut state, &self.policy, now, stale_after_ms, &mut effects);
            // The memory service owns reclamation truth; promotion is an
            // idempotent follow-up and must not depend on a duplicated
            // `reclaimed_permits > 0` interpretation here.
            let pass = self.promote(&mut state, now);
            effects.absorb(pass);
            report
        };
        self.drain_effects(effects, sampled);
        report
    }
}
