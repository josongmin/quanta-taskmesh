//! The shared mutable state behind the governor's mutex, plus the pure capacity
//! arithmetic every feature consults. Feature *services* mutate this; feature
//! *domain* functions never touch it.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use taskmesh_contract::{PermitWaker, TaskClass, TaskScope, TaskStage};

use crate::shared::{PermitId, RequestKey, ResolvedCost, Seq, Ticket};

/// A request waiting in a class queue for capacity.
///
/// `class`, `request_key`, and `enqueued_at_ms` are part of the frozen pending
/// shape (T03) and retained for diagnostics/audit even where promotion does not
/// read them.
pub struct PendingRequest {
    pub ticket: Ticket,
    pub seq_no: Seq,
    #[allow(
        dead_code,
        reason = "part of the frozen T03/T05 pending/ledger shape; kept for audit"
    )]
    pub class: TaskClass,
    #[allow(
        dead_code,
        reason = "part of the frozen T03/T05 pending/ledger shape; kept for audit"
    )]
    pub request_key: RequestKey,
    pub root_operation_id: String,
    pub scope: TaskScope,
    pub target_stage: TaskStage,
    pub cost: ResolvedCost,
    #[allow(
        dead_code,
        reason = "part of the frozen T03/T05 pending/ledger shape; kept for audit"
    )]
    pub enqueued_at_ms: u64,
    /// Absolute deadline used by `DeadlineAware`; `u64::MAX` when unset.
    pub deadline_ms: u64,
    /// WFQ virtual finish tag assigned at enqueue.
    pub finish_tag: u128,
    pub waker: Option<Arc<dyn PermitWaker>>,
}

/// Per-class admission state and queue.
#[derive(Default)]
pub struct ClassState {
    pub inflight: u32,
    pub queue: VecDeque<PendingRequest>,
    /// WFQ: the finish tag of the most recently enqueued request.
    pub last_finish_tag: u128,
    /// DRR: accumulated deficit.
    pub deficit: u64,
    /// Running per-class CPU units held by inflight permits (snapshot in O(1)).
    pub cpu_units_held: u32,
    /// Running per-class effective memory units held by inflight permits.
    pub memory_units_held: u32,
}

impl ClassState {
    pub fn queued(&self) -> u32 {
        u32::try_from(self.queue.len()).unwrap_or(u32::MAX)
    }
}

/// Memory/CPU ledger for one outstanding permit (T05).
#[derive(Debug, Clone)]
pub struct PermitLedger {
    pub cpu_units: u32,
    pub reserved_units: u32,
    pub measured_bytes: u64,
    pub effective_units: u32,
    /// Lease timestamp; retained per the T05 ledger shape for audit/diagnostics.
    #[allow(
        dead_code,
        reason = "part of the frozen T03/T05 pending/ledger shape; kept for audit"
    )]
    pub leased_at_ms: u64,
    pub last_touched_ms: u64,
    pub released: bool,
}

/// A granted permit and everything needed to unwind it on release.
#[derive(Debug, Clone)]
pub struct PermitRecord {
    pub permit_id: PermitId,
    pub class: TaskClass,
    pub root_operation_id: String,
    pub scope: TaskScope,
    pub target_stage: TaskStage,
    pub ledger: PermitLedger,
}

/// Aggregated accounting for one root operation and its children (T06).
#[derive(Debug, Clone, Default)]
pub struct RootExecutionState {
    pub child_inflight: u32,
    pub cpu_units: u32,
    pub memory_units: u32,
    pub active_stages: BTreeSet<TaskStage>,
}

/// Which limit, if any, blocks an admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityBlock {
    Ok,
    Inflight,
    Cpu,
    Memory,
}

/// The full governed state. One instance lives behind the governor's mutex.
#[derive(Default)]
pub struct GovernedState {
    pub classes: BTreeMap<TaskClass, ClassState>,
    pub permits: BTreeMap<PermitId, PermitRecord>,
    pub roots: BTreeMap<String, RootExecutionState>,
    pub active_recursion: BTreeSet<(String, TaskStage)>,
    pub granted: BTreeMap<Ticket, PermitId>,
    pub cpu_units_held: u32,
    pub memory_units_held: u32,
    /// WFQ global virtual time, advanced on each dispatch.
    pub virtual_time: u128,
    /// DRR active-class ring cursor: the class currently being served. Persists
    /// across promotions so a quantum-N class is served ~N times before the ring
    /// advances (proportional service, not priority-by-quantum).
    pub drr_cursor: Option<TaskClass>,
}

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

    /// Pure capacity check: would a permit of `cost` for `class` fit right now,
    /// given the class inflight cap and the global resource budget?
    pub fn capacity_for(
        &self,
        class: &TaskClass,
        max_inflight: u32,
        cost: ResolvedCost,
        budget: &taskmesh_contract::ResourceBudget,
    ) -> CapacityBlock {
        if self.inflight(class) >= max_inflight {
            return CapacityBlock::Inflight;
        }
        if budget.max_cpu_units != 0
            && self.cpu_units_held.saturating_add(cost.cpu_units) > budget.max_cpu_units
        {
            return CapacityBlock::Cpu;
        }
        if budget.max_memory_units != 0
            && self.memory_units_held.saturating_add(cost.memory_units) > budget.max_memory_units
        {
            return CapacityBlock::Memory;
        }
        CapacityBlock::Ok
    }

    /// Commit a granted permit: bump inflight, global held, root accounting, and
    /// the recursion guard. Returns the stored record's permit id.
    #[allow(
        clippy::too_many_arguments,
        reason = "single admission path threads request + ids + waker together"
    )]
    pub fn grant(
        &mut self,
        permit_id: PermitId,
        class: &TaskClass,
        root_operation_id: &str,
        scope: TaskScope,
        target_stage: TaskStage,
        cost: ResolvedCost,
        reserved_units: u32,
        now_ms: u64,
    ) {
        {
            let cstate = self.class_mut(class);
            cstate.inflight += 1;
            cstate.cpu_units_held = cstate.cpu_units_held.saturating_add(cost.cpu_units);
            cstate.memory_units_held = cstate.memory_units_held.saturating_add(cost.memory_units);
        }
        self.cpu_units_held = self.cpu_units_held.saturating_add(cost.cpu_units);
        self.memory_units_held = self.memory_units_held.saturating_add(cost.memory_units);

        if matches!(scope, TaskScope::Child { .. }) {
            let root = self.roots.entry(root_operation_id.to_owned()).or_default();
            root.child_inflight += 1;
            root.cpu_units = root.cpu_units.saturating_add(cost.cpu_units);
            root.memory_units = root.memory_units.saturating_add(cost.memory_units);
            root.active_stages.insert(target_stage.clone());
        }
        self.active_recursion
            .insert((root_operation_id.to_owned(), target_stage.clone()));

        self.permits.insert(
            permit_id,
            PermitRecord {
                permit_id,
                class: class.clone(),
                root_operation_id: root_operation_id.to_owned(),
                scope,
                target_stage,
                ledger: PermitLedger {
                    cpu_units: cost.cpu_units,
                    reserved_units,
                    measured_bytes: 0,
                    effective_units: cost.memory_units,
                    leased_at_ms: now_ms,
                    last_touched_ms: now_ms,
                    released: false,
                },
            },
        );
        self.assert_consistent();
    }

    /// Unwind a permit fully: inflight, global held, root accounting, recursion
    /// guard, and the permit record. Returns the freed record if it existed.
    pub fn unwind(&mut self, permit_id: PermitId) -> Option<PermitRecord> {
        let record = self.permits.remove(&permit_id)?;
        if let Some(state) = self.classes.get_mut(&record.class) {
            state.inflight = state.inflight.saturating_sub(1);
            state.cpu_units_held = state.cpu_units_held.saturating_sub(record.ledger.cpu_units);
            state.memory_units_held = state
                .memory_units_held
                .saturating_sub(record.ledger.effective_units);
        }
        self.cpu_units_held = self.cpu_units_held.saturating_sub(record.ledger.cpu_units);
        self.memory_units_held = self
            .memory_units_held
            .saturating_sub(record.ledger.effective_units);

        if matches!(record.scope, TaskScope::Child { .. }) {
            if let Some(root) = self.roots.get_mut(&record.root_operation_id) {
                root.child_inflight = root.child_inflight.saturating_sub(1);
                root.cpu_units = root.cpu_units.saturating_sub(record.ledger.cpu_units);
                root.memory_units = root
                    .memory_units
                    .saturating_sub(record.ledger.effective_units);
                root.active_stages.remove(&record.target_stage);
                if root.child_inflight == 0 {
                    self.roots.remove(&record.root_operation_id);
                }
            }
        }
        self.active_recursion.remove(&(
            record.root_operation_id.clone(),
            record.target_stage.clone(),
        ));
        self.assert_consistent();
        Some(record)
    }

    /// Debug-only structural invariants. Saturating arithmetic hides accounting
    /// bugs in release builds; this catches them in tests/debug. Compiles to a
    /// no-op without `debug_assertions`.
    #[cfg(debug_assertions)]
    pub fn assert_consistent(&self) {
        let class_cpu: u32 = self.classes.values().map(|c| c.cpu_units_held).sum();
        let class_mem: u32 = self.classes.values().map(|c| c.memory_units_held).sum();
        debug_assert_eq!(
            class_cpu, self.cpu_units_held,
            "per-class cpu sum != global"
        );
        debug_assert_eq!(
            class_mem, self.memory_units_held,
            "per-class mem sum != global"
        );

        let mut permit_cpu = 0u32;
        let mut permit_mem = 0u32;
        let mut inflight_by_class: BTreeMap<&TaskClass, u32> = BTreeMap::new();
        for record in self.permits.values() {
            permit_cpu = permit_cpu.saturating_add(record.ledger.cpu_units);
            permit_mem = permit_mem.saturating_add(record.ledger.effective_units);
            *inflight_by_class.entry(&record.class).or_default() += 1;
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
        }
        for permit_id in self.granted.values() {
            debug_assert!(
                self.permits.contains_key(permit_id),
                "granted ticket maps to a permit that no longer exists"
            );
        }
    }

    #[cfg(not(debug_assertions))]
    #[inline]
    pub fn assert_consistent(&self) {}
}
