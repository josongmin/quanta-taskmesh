//! Governed state transitions and accounting invariants.

use super::{
    mint_lease_nonce, AdvanceOutcome, AdvanceRefusal, BTreeMap, CapabilityId, ClassState,
    ExecutionPhase, GovernedState, GrantRequest, LeaseToken, MeasurementSequence, PermitId,
    PermitLedger, PermitRecord, ReleaseOutcome, RootOperationPermits, TaskClass, TaskScope,
    TaskStage, TerminalReason, Ticket, TicketState, TransitionEffects, MAX_TERMINAL_TICKETS,
};

impl GovernedState {
    pub fn class_mut(&mut self, class: &TaskClass) -> &mut ClassState {
        self.classes.entry(class.clone()).or_default()
    }

    pub fn inflight(&self, class: &TaskClass) -> u32 {
        self.classes.get(class).map_or(0, |c| c.inflight)
    }

    pub fn queued(&self, class: &TaskClass) -> u32 {
        self.classes.get(class).map_or(0, ClassState::queued)
    }

    /// Clamp a clock sample to the monotonic commit watermark and advance it.
    /// Call this **after** taking the lock, with a sample read before it (D07).
    pub fn commit_time(&mut self, sampled_ms: u64) -> u64 {
        let committed = sampled_ms.max(self.commit_watermark_ms);
        self.commit_watermark_ms = committed;
        committed
    }

    pub fn capability_in_use(&self, capability: &CapabilityId) -> u32 {
        self.capability_in_use.get(capability).copied().unwrap_or(0)
    }

    pub fn permits_for_operation(
        &self,
        root_operation_id: &str,
        operation: &str,
    ) -> Option<PermitId> {
        self.operation_permits
            .get(root_operation_id)?
            .get(root_operation_id, operation)
    }

    /// Resolve the currently live parent generation named by a child scope.
    /// Callers that persist lineage must store the returned permit ID rather
    /// than resolving the operation text again after it can be reused.
    pub fn parent_permit_for_scope(
        &self,
        root_operation_id: &str,
        scope: &TaskScope,
    ) -> Option<PermitId> {
        let TaskScope::Child {
            parent_operation_id,
            ..
        } = scope
        else {
            return None;
        };
        self.permits_for_operation(root_operation_id, parent_operation_id)
    }

    pub fn has_operation_identity(&self, root_operation_id: &str, operation: &str) -> bool {
        self.permits_for_operation(root_operation_id, operation)
            .is_some()
            || self.classes.values().any(|class| {
                class.queue.iter().any(|request| {
                    request.root_operation_id == root_operation_id && request.operation == operation
                })
            })
    }

    /// Commit a granted permit: bump inflight, global held, capability
    /// occupancy, root accounting, and the recursion guard.
    pub fn grant(&mut self, request: GrantRequest<'_>) {
        let GrantRequest {
            permit_id,
            class,
            operation,
            root_operation_id,
            scope,
            parent_permit_id,
            target_stage,
            provenance,
            capabilities,
            cost,
            reserved_units,
            now_ms,
            pending_ticket,
            claim_waker,
        } = request;

        {
            let cstate = self.class_mut(class);
            cstate.inflight += 1;
            cstate.cpu_units_held += u128::from(cost.cpu_units);
            cstate.memory_units_held += u128::from(cost.memory_units);
            cstate.admitted_total += 1;
            cstate.enter_phase(ExecutionPhase::DispatchReserved);
        }
        self.cpu_units_held += u128::from(cost.cpu_units);
        self.memory_units_held += u128::from(cost.memory_units);
        for id in capabilities.iter() {
            let slot = self.capability_in_use.entry(id.clone()).or_insert(0);
            *slot = slot.saturating_add(1);
        }
        if let Some(operations) = self.operation_permits.get_mut(root_operation_id) {
            operations.insert(root_operation_id, operation, permit_id);
        } else {
            let mut operations = RootOperationPermits::default();
            operations.insert(root_operation_id, operation, permit_id);
            self.operation_permits
                .insert(root_operation_id.to_owned(), operations);
        }

        // Recursion guard and root attribution are CHILD-only. A root must never
        // occupy `(root, stage)` itself, else its own first child — which targets
        // the parent stage — would be (wrongly) rejected as recursive.
        if matches!(scope, TaskScope::Child { .. }) {
            let root = match self.roots.get_mut(root_operation_id) {
                Some(root) => root,
                None => self.roots.entry(root_operation_id.to_owned()).or_default(),
            };
            root.child_inflight += 1;
            root.cpu_units += u128::from(cost.cpu_units);
            root.memory_units += u128::from(cost.memory_units);
            root.active_stages.insert(target_stage.clone());
            self.occupy_recursion_guard(root_operation_id, &target_stage);
        }

        if let Some(ticket) = pending_ticket {
            self.tickets.insert(ticket, TicketState::Granted(permit_id));
        }

        self.permits.insert(
            permit_id,
            PermitRecord {
                permit_id,
                class: class.clone(),
                operation: operation.to_owned(),
                root_operation_id: root_operation_id.to_owned(),
                scope,
                parent_permit_id,
                target_stage,
                provenance,
                capabilities,
                phase: ExecutionPhase::DispatchReserved,
                lease_nonce: None,
                pending_ticket,
                claim_waker,
                ledger: PermitLedger {
                    cpu_units: cost.cpu_units,
                    original_estimate_units: reserved_units,
                    remaining_reservation_units: reserved_units,
                    measured_bytes: 0,
                    measurement_sequence: MeasurementSequence::INITIAL,
                    stage_sequence: 0,
                    effective_units: cost.memory_units,
                    leased_at_ms: now_ms,
                    last_touched_ms: now_ms,
                },
            },
        );
        self.assert_consistent();
    }

    /// Advance a permit's ownership phase. Monotonic: a request never moves back
    /// to an earlier phase, and re-declaring the current phase is refused.
    ///
    /// The move out of `DispatchReserved` is the one that mints the permit's
    /// [`LeaseToken`]: the nonce is recorded in the ledger here, in the same
    /// transition that changes the phase, so there is no instant at which the
    /// permit is dispatched but unowned.
    pub fn advance_phase(&mut self, permit_id: PermitId, phase: ExecutionPhase) -> AdvanceOutcome {
        let Some(record) = self.permits.get_mut(&permit_id) else {
            return AdvanceOutcome::Refused(AdvanceRefusal::UnknownPermit);
        };
        if phase.rank() <= record.phase.rank() {
            return AdvanceOutcome::Refused(AdvanceRefusal::NotLater {
                current: record.phase,
            });
        }
        let previous = record.phase;
        record.phase = phase;
        let outcome = if record.lease_nonce.is_none() {
            let nonce = mint_lease_nonce();
            record.lease_nonce = Some(nonce);
            AdvanceOutcome::Leased(LeaseToken { permit_id, nonce })
        } else {
            AdvanceOutcome::Advanced
        };
        let class = record.class.clone();
        let newly_started = !previous.has_started() && phase.has_started();
        let cstate = self.class_mut(&class);
        cstate.leave_phase(previous);
        cstate.enter_phase(phase);
        if newly_started {
            cstate.started_total += 1;
        }
        self.assert_consistent();
        outcome
    }

    /// Unwind a permit if — and only if — `proof` is its custody: `None` for a
    /// permit that has not left `DispatchReserved`, or its minted lease nonce
    /// once it has. A permit is never ended by a caller that does not hold
    /// exactly what the ledger says its owner holds.
    pub fn release_with_proof(
        &mut self,
        permit_id: PermitId,
        proof: Option<u64>,
        effects: &mut TransitionEffects,
    ) -> ReleaseOutcome {
        let Some(record) = self.permits.get(&permit_id) else {
            return ReleaseOutcome::UnknownPermit;
        };
        if record.lease_nonce != proof {
            return ReleaseOutcome::HeldByLease {
                phase: record.phase,
            };
        }
        match self.unwind(permit_id, TerminalReason::Released, effects) {
            Some(_) => ReleaseOutcome::Released,
            // The record was found a moment ago under the same lock.
            None => ReleaseOutcome::UnknownPermit,
        }
    }

    /// Record a terminal outcome for a ticket, evicting the oldest retained
    /// outcome when the bound is reached.
    pub fn record_terminal_ticket(&mut self, ticket: Ticket, reason: TerminalReason) {
        self.tickets.insert(ticket, TicketState::Terminal(reason));
        if let Some(previous) = self.terminal_index.remove(&ticket) {
            self.terminal_order.remove(&previous);
        }
        let seq = self.terminal_seq;
        self.terminal_seq = self.terminal_seq.wrapping_add(1);
        self.terminal_order.insert(seq, ticket);
        self.terminal_index.insert(ticket, seq);
        while self.terminal_order.len() > MAX_TERMINAL_TICKETS {
            let Some((&oldest_seq, &oldest)) = self.terminal_order.iter().next() else {
                break;
            };
            self.terminal_order.remove(&oldest_seq);
            self.terminal_index.remove(&oldest);
            if matches!(self.tickets.get(&oldest), Some(TicketState::Terminal(_))) {
                self.tickets.remove(&oldest);
            }
        }
    }

    /// Drop a retained terminal record once it has been observed. Only a
    /// terminal record is dropped: a ticket that is queued or granted stays.
    pub fn forget_terminal_ticket(&mut self, ticket: Ticket) {
        if let Some(seq) = self.terminal_index.remove(&ticket) {
            self.terminal_order.remove(&seq);
        }
        if matches!(self.tickets.get(&ticket), Some(TicketState::Terminal(_))) {
            self.tickets.remove(&ticket);
        }
    }

    /// Number of terminal records currently retained (bounded by
    /// [`crate::MAX_TERMINAL_TICKETS`]).
    pub fn retained_terminal_tickets(&self) -> usize {
        self.terminal_order.len()
    }

    /// Mark `(root, stage)` as occupied by a child (queued or granted).
    pub fn occupy_recursion_guard(&mut self, root_operation_id: &str, stage: &TaskStage) {
        let stages = match self.active_recursion.get_mut(root_operation_id) {
            Some(stages) => stages,
            None => self
                .active_recursion
                .entry(root_operation_id.to_owned())
                .or_default(),
        };
        stages.insert(stage.clone());
    }

    /// Release `(root, stage)`; the root's entry disappears with its last stage.
    pub fn vacate_recursion_guard(&mut self, root_operation_id: &str, stage: &TaskStage) {
        if let Some(stages) = self.active_recursion.get_mut(root_operation_id) {
            stages.remove(stage);
            if stages.is_empty() {
                self.active_recursion.remove(root_operation_id);
            }
        }
    }

    /// Unwind a permit fully: inflight, phase gauges, global held, capability
    /// occupancy, root accounting, recursion guard, the permit record, and — the
    /// part a bare `permits.remove` forgets — the ticket that granted it.
    ///
    /// Any host `Arc` the record still owns is moved into `effects` so it is
    /// dropped outside the lock.
    pub fn unwind(
        &mut self,
        permit_id: PermitId,
        reason: TerminalReason,
        effects: &mut TransitionEffects,
    ) -> Option<PermitRecord> {
        let mut record = self.permits.remove(&permit_id)?;
        if let Some(state) = self.classes.get_mut(&record.class) {
            state.inflight = state.inflight.saturating_sub(1);
            state.cpu_units_held = state
                .cpu_units_held
                .saturating_sub(u128::from(record.ledger.cpu_units));
            state.memory_units_held = state
                .memory_units_held
                .saturating_sub(u128::from(record.ledger.effective_units));
            state.leave_phase(record.phase);
            state.terminated_total += 1;
        }
        self.cpu_units_held = self
            .cpu_units_held
            .saturating_sub(u128::from(record.ledger.cpu_units));
        self.memory_units_held = self
            .memory_units_held
            .saturating_sub(u128::from(record.ledger.effective_units));
        for id in record.capabilities.iter() {
            if let Some(slot) = self.capability_in_use.get_mut(id) {
                *slot = slot.saturating_sub(1);
                if *slot == 0 {
                    self.capability_in_use.remove(id);
                }
            }
        }
        if let Some(operations) = self.operation_permits.get_mut(&record.root_operation_id) {
            let removed = operations.remove(&record.root_operation_id, &record.operation);
            debug_assert_eq!(
                removed,
                Some(permit_id),
                "permit missing from exact operation identity index"
            );
            if operations.is_empty() {
                self.operation_permits.remove(&record.root_operation_id);
            }
        }

        if matches!(record.scope, TaskScope::Child { .. }) {
            if let Some(root) = self.roots.get_mut(&record.root_operation_id) {
                root.child_inflight = root.child_inflight.saturating_sub(1);
                root.cpu_units = root
                    .cpu_units
                    .saturating_sub(u128::from(record.ledger.cpu_units));
                root.memory_units = root
                    .memory_units
                    .saturating_sub(u128::from(record.ledger.effective_units));
                root.active_stages.remove(&record.target_stage);
                if root.child_inflight == 0 {
                    self.roots.remove(&record.root_operation_id);
                }
            }
            // Symmetric with grant: only children occupy the recursion guard.
            self.vacate_recursion_guard(&record.root_operation_id, &record.target_stage);
        }

        // The ticket dies with the permit. Leaving the mapping behind is what
        // hands a waiter a permit id that no longer exists; removing it without
        // recording *why* is what parks that waiter forever.
        let pending_ticket = record.pending_ticket.take();
        let claim_waker = record.claim_waker.take();
        match (pending_ticket, claim_waker) {
            // A waiter is still owed an answer: record why, and notify it.
            (Some(ticket), waker) => {
                self.record_terminal_ticket(ticket, reason);
                if let Some(waker) = waker {
                    effects.wake.push(waker);
                }
            }
            // Already claimed: nobody to tell, but the reference still has to be
            // destroyed outside the lock.
            (None, Some(waker)) => effects.retire.push(waker),
            (None, None) => {}
        }

        self.assert_consistent();
        Some(record)
    }

    /// Flag an unrepresentable accounting result. Admission fails closed from
    /// here on; the condition is sticky because a ledger that lost a total
    /// cannot be trusted to have lost only one.
    pub fn record_accounting_fault(&mut self, what: &'static str) {
        if self.accounting_fault.is_none() {
            self.accounting_fault = Some(what);
        }
    }

    /// Stop admitting. One-way, idempotent; nothing already in the state is
    /// touched (D17).
    pub fn close_admission(&mut self) {
        self.admission_closed = true;
    }

    /// Debug-only structural invariants. This is the *independent* recomputation
    /// of every incrementally-maintained total, so a drift shows up in tests
    /// rather than in production accounting.
    #[cfg(debug_assertions)]
    pub fn assert_consistent(&self) {
        let class_cpu: u128 = self.classes.values().map(|c| c.cpu_units_held).sum();
        let class_mem: u128 = self.classes.values().map(|c| c.memory_units_held).sum();
        debug_assert_eq!(
            class_cpu, self.cpu_units_held,
            "per-class cpu sum != global"
        );
        debug_assert_eq!(
            class_mem, self.memory_units_held,
            "per-class mem sum != global"
        );

        let mut permit_cpu: u128 = 0;
        let mut permit_mem: u128 = 0;
        let mut inflight_by_class: BTreeMap<&TaskClass, u32> = BTreeMap::new();
        let mut capability_by_pool: BTreeMap<&CapabilityId, u32> = BTreeMap::new();
        for record in self.permits.values() {
            permit_cpu += u128::from(record.ledger.cpu_units);
            permit_mem += u128::from(record.ledger.effective_units);
            *inflight_by_class.entry(&record.class).or_default() += 1;
            for id in record.capabilities.iter() {
                *capability_by_pool.entry(id).or_default() += 1;
            }
            debug_assert_eq!(
                record.lease_nonce.is_some(),
                record.phase != ExecutionPhase::DispatchReserved,
                "permit {} is {} with lease nonce {:?}: a permit is leased exactly when it has left DispatchReserved",
                record.permit_id,
                record.phase,
                record.lease_nonce
            );
        }
        debug_assert_eq!(
            permit_cpu, self.cpu_units_held,
            "permit cpu sum != global held"
        );
        debug_assert_eq!(
            permit_mem, self.memory_units_held,
            "permit mem sum != global held"
        );
        for (class, cstate) in &self.classes {
            let from_permits = inflight_by_class.get(class).copied().unwrap_or(0);
            debug_assert_eq!(
                cstate.inflight, from_permits,
                "class {class} inflight {} != live permit count {from_permits}",
                cstate.inflight
            );
            let phases = cstate.dispatch_reserved
                + cstate.accepted
                + cstate.running
                + cstate.cleanup_pending;
            debug_assert_eq!(
                cstate.inflight, phases,
                "class {class} inflight {} != phase sum {phases}",
                cstate.inflight
            );
            debug_assert_eq!(
                cstate.admitted_total,
                u128::from(cstate.inflight) + cstate.terminated_total,
                "class {class} admitted_total != inflight + terminated_total"
            );
        }
        for (pool, in_use) in &self.capability_in_use {
            let from_permits = capability_by_pool.get(pool).copied().unwrap_or(0);
            debug_assert_eq!(
                *in_use, from_permits,
                "capability {pool} in_use {in_use} != live permit count {from_permits}"
            );
        }
        for (ticket, ticket_state) in &self.tickets {
            if let TicketState::Granted(permit_id) | TicketState::Claiming(permit_id) = ticket_state
            {
                debug_assert!(
                    self.permits.contains_key(permit_id),
                    "granted ticket {ticket} maps to a permit that no longer exists"
                );
            }
        }
    }

    #[cfg(not(debug_assertions))]
    #[inline]
    pub fn assert_consistent(&self) {}
}
