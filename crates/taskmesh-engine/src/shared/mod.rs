//! Cross-slice kernel: identifiers, the resolved policy set, and the value types
//! every feature shares. No feature logic lives here.

use std::borrow::Cow;
use std::collections::BTreeMap;

use taskmesh_contract::{ClassPolicy, ResourceBudget, SubstrateRecord, TaskClass};

/// Identity of a granted permit.
pub type PermitId = u64;

/// Identity of a queued request awaiting promotion.
pub type Ticket = u64;

/// Monotonic admission sequence number (also used as the FIFO arrival key).
pub type Seq = u64;

/// Admission key, derived from the root operation id. Same-key dedupe is out of
/// scope for the startup set.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestKey(Cow<'static, str>);

impl RequestKey {
    pub fn new(value: impl Into<Cow<'static, str>>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

/// The concrete resource a permit reserves, after mode resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResolvedCost {
    pub cpu_units: u32,
    pub memory_units: u32,
}

/// The validated, resolved policy a [`crate::Governor`] runs against. Substrates
/// are kept in a registry for explicit, duplicate-checked inventory (T08).
#[derive(Debug, Clone, Default)]
pub struct PolicySet {
    pub resources: ResourceBudget,
    pub classes: BTreeMap<TaskClass, ClassPolicy>,
    pub substrates: BTreeMap<String, SubstrateRecord>,
}

impl PolicySet {
    /// Build a policy set seeded with the canonical built-in substrate inventory.
    ///
    /// The built-ins are intrinsic to *any* governor, so the inventory is never
    /// empty — a direct `Governor::new` embedder gets the same authoritative
    /// substrate snapshot the host `Builder` produces, without having to remember
    /// to call [`PolicySet::with_substrates`]. Deployment-specific substrates are
    /// layered on top via [`PolicySet::with_substrates`].
    pub fn new(resources: ResourceBudget, classes: BTreeMap<TaskClass, ClassPolicy>) -> Self {
        let mut substrates = BTreeMap::new();
        for record in crate::features::inventory::builtin_records() {
            // The built-in set is the canonical, valid, name-unique SSOT, so
            // registration cannot fail by construction.
            crate::features::inventory::register(&mut substrates, record)
                .expect("built-in substrate records are valid and unique");
        }
        Self {
            resources,
            classes,
            substrates,
        }
    }

    /// Layer *additional* (deployment-specific) substrates onto the built-in set,
    /// rejecting duplicates (including any attempt to shadow a built-in) and
    /// invalid records.
    pub fn with_substrates(
        mut self,
        substrates: Vec<SubstrateRecord>,
    ) -> Result<Self, taskmesh_contract::GovernorError> {
        for record in substrates {
            crate::features::inventory::register(&mut self.substrates, record)?;
        }
        Ok(self)
    }

    pub fn class(&self, class: &TaskClass) -> Option<&ClassPolicy> {
        self.classes.get(class)
    }
}

/// Classification provenance carried through admission so "why this class" stays
/// auditable in runtime state after submit, not just on the inbound `TaskSpec`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provenance {
    pub source: taskmesh_contract::PlanSource,
    pub reason: taskmesh_contract::ClassificationRationale,
}

impl Provenance {
    pub fn of(spec: &taskmesh_contract::TaskSpec) -> Self {
        Self {
            source: spec.source,
            reason: spec.reason,
        }
    }
}

/// Aggregated accounting for one root operation and its children (T06),
/// exposed for inspection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RootAttribution {
    pub child_inflight: u32,
    pub cpu_units: u32,
    pub memory_units: u32,
    pub active_stages: usize,
}

/// Result of a leak sweep over outstanding permits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeakSweepReport {
    pub reclaimed_permits: u32,
    pub suspected_leaks: u32,
}

/// The outcome of an admission decision at the engine boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionDecision {
    /// Admitted immediately; caller owns `permit_id` until release.
    Admitted { permit_id: PermitId },
    /// Enqueued; caller waits and later [`crate::Governor::claim`]s `ticket`.
    Queued { ticket: Ticket },
    /// Rejected with a terminal verdict.
    Rejected(taskmesh_contract::AdmissionVerdict),
}
