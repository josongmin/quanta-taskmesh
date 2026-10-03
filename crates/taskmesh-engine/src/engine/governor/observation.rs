//! Governor observation transitions.

use super::{
    admission, inventory, BTreeMap, CapabilityUsage, CapacityBlock, ClassSnapshot, Governor,
    PendingView, PermitId, PermitLedgerView, Snapshot, TaskClass, Ticket, TicketState,
    SNAPSHOT_SCHEMA_VERSION,
};

impl Governor {
    // ---- observation ------------------------------------------------------

    pub fn snapshot(&self) -> Snapshot {
        let state = self.state.lock();
        let mut classes: BTreeMap<TaskClass, ClassSnapshot> = BTreeMap::new();
        // O(classes): per-class held totals are maintained incrementally in
        // grant/unwind/reconcile, so no permit scan is needed.
        let known = self
            .policy
            .classes
            .keys()
            .chain(state.classes.keys())
            .cloned();
        for class in known {
            let projection = state
                .classes
                .get(&class)
                .map_or_else(ClassSnapshot::default, |c| ClassSnapshot {
                    inflight: c.inflight,
                    queued: c.queued(),
                    cpu_units_held: c.cpu_units_held,
                    memory_units_held: c.memory_units_held,
                    dispatch_reserved: c.dispatch_reserved,
                    accepted: c.accepted,
                    running: c.running,
                    cleanup_pending: c.cleanup_pending,
                    admitted_total: c.admitted_total,
                    started_total: c.started_total,
                    terminated_total: c.terminated_total,
                });
            classes.entry(class).or_insert(projection);
        }
        // Every authority record is always present. The same record supplies
        // both the capacity decision and this projection.
        let mut capabilities: BTreeMap<String, CapabilityUsage> = BTreeMap::new();
        for record in self.policy.capability_records() {
            capabilities.insert(
                record.name().to_owned(),
                CapabilityUsage {
                    in_use: state.capability_in_use(record.id()),
                    limit: record.capacity().limit(),
                },
            );
        }
        Snapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            classes,
            substrates: inventory::snapshot(self.policy.substrates()),
            capabilities,
        }
    }

    /// Diagnostic view of a queued ticket using the same current-state
    /// assessment as admission and promotion.
    pub fn pending_view(&self, ticket: Ticket) -> Option<PendingView> {
        let state = self.state.lock();
        let TicketState::Queued { class } = state.tickets.get(&ticket)? else {
            return None;
        };
        let cstate = state.classes.get(class)?;
        let (index, request) = cstate
            .queue
            .iter()
            .enumerate()
            .find(|(_, request)| request.ticket == ticket)?;
        let policy = self.policy.class(class)?;
        let assessment = admission::pending::assess_pending(
            &state,
            &self.policy,
            request,
            policy.max_inflight,
            state.promotion_pending
                || admission::pending::queued_behind_index(&cstate.queue, index),
        );
        let blocked_on = match &assessment {
            admission::pending::CapacityAssessment::Runnable => CapacityBlock::Ok,
            admission::pending::CapacityAssessment::ReversiblyBlocked(blockers) => {
                blockers.primary()
            }
            admission::pending::CapacityAssessment::IrreversibleWaitCycle(_) => request.blocked_on,
        };
        Some(PendingView {
            class: request.class.clone(),
            request_key: request.request_key.clone(),
            blocked_on,
            assessment,
            enqueued_at_ms: request.enqueued_at_ms,
            queue_wait_ms: state
                .commit_watermark_ms
                .saturating_sub(request.enqueued_at_ms),
            capabilities: request.capabilities.clone(),
        })
    }

    /// The primary current blocker for a queued ticket, or `None` if it is not
    /// queued. Use [`Self::pending_assessment`] when every simultaneous blocker
    /// or an irreversible-cycle witness is required.
    pub fn pending_block_reason(&self, ticket: Ticket) -> Option<CapacityBlock> {
        self.pending_view(ticket).map(|view| view.blocked_on)
    }

    pub fn pending_assessment(
        &self,
        ticket: Ticket,
    ) -> Option<admission::pending::CapacityAssessment> {
        self.pending_view(ticket).map(|view| view.assessment)
    }

    /// Full ledger view of a live permit.
    ///
    /// Exposed so an *independent* observer can recompute the aggregates rather
    /// than re-reading the same incrementally-maintained counters the engine
    /// uses — a conservation check that shares its arithmetic with the thing it
    /// checks proves nothing.
    pub fn permit_ledger(&self, permit_id: PermitId) -> Option<PermitLedgerView> {
        let state = self.state.lock();
        let record = state.permits.get(&permit_id)?;
        Some(PermitLedgerView {
            permit_id,
            class: record.class.clone(),
            capabilities: record.capabilities.clone(),
            phase: record.phase,
            cpu_units: record.ledger.cpu_units,
            original_estimate_units: record.ledger.original_estimate_units,
            remaining_reservation_units: record.ledger.remaining_reservation_units,
            effective_units: record.ledger.effective_units,
            measured_bytes: record.ledger.measured_bytes,
            measurement_epoch: record.ledger.measurement_sequence.get(),
            leased_at_ms: record.ledger.leased_at_ms,
            last_touched_ms: record.ledger.last_touched_ms,
        })
    }

    /// Every live permit ledger, for an independent conservation oracle.
    pub fn permit_ledgers(&self) -> Vec<PermitLedgerView> {
        let state = self.state.lock();
        state
            .permits
            .values()
            .map(|record| PermitLedgerView {
                permit_id: record.permit_id,
                class: record.class.clone(),
                capabilities: record.capabilities.clone(),
                phase: record.phase,
                cpu_units: record.ledger.cpu_units,
                original_estimate_units: record.ledger.original_estimate_units,
                remaining_reservation_units: record.ledger.remaining_reservation_units,
                effective_units: record.ledger.effective_units,
                measured_bytes: record.ledger.measured_bytes,
                measurement_epoch: record.ledger.measurement_sequence.get(),
                leased_at_ms: record.ledger.leased_at_ms,
                last_touched_ms: record.ledger.last_touched_ms,
            })
            .collect()
    }

    /// The sticky accounting-fault reason, if the ledger ever failed to
    /// represent a total. While set, admission is refused.
    pub fn accounting_fault(&self) -> Option<&'static str> {
        self.state.lock().accounting_fault
    }

    /// Stop admitting new work (ADR 0003 D17). One-way: there is no reopen.
    ///
    /// From the moment this returns, a well-formed request using a valid local
    /// capability is refused with
    /// [`AdmissionVerdict::RuntimeUnavailable`](taskmesh_contract::AdmissionVerdict::RuntimeUnavailable)
    /// before anything is queued, charged, or counted. Direct `admit*` methods
    /// still run their public preflight first: malformed specs return
    /// `MalformedTask`, and `admit_resolved` can return a capability error.
    /// Neither preflight path mutates the closed governor. Work already admitted
    /// or queued is untouched:
    /// queued requests are still promoted and claimed, leases are still
    /// released, and the gauges still converge to zero. The flag is read and
    /// written under the admission lock, so a [`Self::snapshot`] taken after
    /// this returns already contains every admission that will ever happen.
    ///
    /// Distinct from an accounting fault, which refuses admission with the
    /// same verdict but reports through [`Self::accounting_fault`].
    pub fn close_admission(&self) {
        self.state.lock().close_admission();
    }

    /// Whether [`Self::close_admission`] has been called.
    pub fn admission_closed(&self) -> bool {
        self.state.lock().admission_closed
    }

    /// How many terminal ticket records are currently retained for late
    /// claimers. Never exceeds [`crate::MAX_TERMINAL_TICKETS`].
    pub fn retained_terminal_tickets(&self) -> usize {
        self.state.lock().retained_terminal_tickets()
    }

    /// Ring positions the DRR scheduler has examined on this governor — the
    /// work oracle behind TM16-012 (`O(runnable classes)` per selection, never
    /// `O(cost / quantum)`).
    #[cfg(feature = "test-util")]
    #[doc(hidden)]
    pub fn drr_ring_visits(&self) -> u64 {
        self.state.lock().drr_ring_visits
    }

    /// Survivor entries repriced by WFQ after queue mutations. Head-only
    /// service is `O(1)` and therefore contributes zero.
    #[cfg(feature = "test-util")]
    #[doc(hidden)]
    pub fn wfq_reprice_visits(&self) -> u64 {
        self.state
            .lock()
            .classes
            .values()
            .map(|cstate| cstate.wfq_reprice_visits)
            .fold(0, u64::saturating_add)
    }
}
