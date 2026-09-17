//! Cross-slice kernel: identifiers, the resolved policy set, and the value types
//! every feature shares. No feature logic lives here.

use std::borrow::Cow;
use std::collections::BTreeMap;
#[cfg(feature = "test-util")]
use std::sync::atomic::{AtomicUsize, Ordering};

use taskmesh_contract::{ClassPolicy, ResourceBudget, SubstrateRecord, TaskClass};

/// Identity of a granted permit.
pub type PermitId = u64;

/// Name of a capability pool, shared cheaply between pending requests, permits,
/// and the occupancy ledger.
pub type CapabilityName = std::sync::Arc<str>;

/// Identity of a queued request awaiting promotion.
pub type Ticket = u64;

/// Monotonic admission sequence number (also used as the FIFO arrival key).
pub type Seq = u64;

/// Admission key, derived from the root operation id.
///
/// There is no caller-supplied key. An earlier surface accepted one, but it
/// carried no behavior — admission never read it — so it was a misleading
/// input; same-key dedupe remains out of scope for the startup set.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestKey(Cow<'static, str>);

#[cfg(feature = "test-util")]
static REQUEST_KEY_DERIVE_COUNT: AtomicUsize = AtomicUsize::new(0);

impl RequestKey {
    /// Derive the admission key from a spec's root operation id (the only source
    /// of authority).
    pub fn from_root(root_operation_id: &str) -> Self {
        #[cfg(feature = "test-util")]
        REQUEST_KEY_DERIVE_COUNT.fetch_add(1, Ordering::Relaxed);
        Self(Cow::Owned(root_operation_id.to_owned()))
    }

    /// The key as text (the root operation id it was derived from). Read-only:
    /// a key is never a caller input.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RequestKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(feature = "test-util")]
#[doc(hidden)]
pub fn request_key_derive_count() -> usize {
    REQUEST_KEY_DERIVE_COUNT.load(Ordering::Relaxed)
}

#[cfg(feature = "test-util")]
#[doc(hidden)]
pub fn reset_request_key_derive_count() {
    REQUEST_KEY_DERIVE_COUNT.store(0, Ordering::Relaxed);
}

/// The concrete resource a permit reserves, after mode resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResolvedCost {
    pub cpu_units: u32,
    pub memory_units: u32,
}

/// The validated, resolved policy a [`crate::Governor`] runs against. Substrates
/// are kept in a registry for explicit, duplicate-checked inventory (T08).
///
/// # Why the registry is private
///
/// The substrate registry and the capability limits are *validated* state, and
/// a public field is a hole straight through the validator: a caller could
/// `clear()` the inventory, insert a record under a key that disagrees with its
/// own name, or build the whole struct with a literal and skip
/// [`PolicySet::new`] entirely. They are reached through constructors and
/// accessors so [`crate::Governor::new`] is the only way state becomes live, and
/// it re-checks the registry it is handed regardless of how it was built.
#[derive(Debug, Clone)]
pub struct PolicySet {
    pub resources: ResourceBudget,
    pub classes: BTreeMap<TaskClass, ClassPolicy>,
    substrates: BTreeMap<String, SubstrateRecord>,
    capability_limits: BTreeMap<String, u32>,
    /// Every registered capability-pool name, interned once. Admission clones
    /// an `Arc` from here instead of allocating a fresh name per request: the
    /// per-op allocation count is a gated metric, and resolving a capability
    /// must not cost one.
    capability_names: BTreeMap<String, CapabilityName>,
}

impl Default for PolicySet {
    /// Delegates to the canonical [`PolicySet::new`], so a defaulted policy set
    /// carries the built-in inventory like every other one. An empty-inventory
    /// governor is not a thing that can be constructed.
    fn default() -> Self {
        Self::new(ResourceBudget::default(), BTreeMap::new())
    }
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
        let capability_names = intern_capability_names(&substrates);
        Self {
            resources,
            classes,
            substrates,
            capability_limits: BTreeMap::new(),
            capability_names,
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
        self.capability_names = intern_capability_names(&self.substrates);
        Ok(self)
    }

    /// Declare capability-pool slot limits, keyed by capability-pool name.
    ///
    /// This is the **single** physical-capacity authority: the engine checks it
    /// in the same transition as class inflight and the resource budget, so
    /// there is no second queue in front of admission for work to pile up in. A
    /// missing or `0` limit means the pool is ungated, matching the published
    /// `0 == no limit` convention.
    ///
    /// Rejects a limit for a pool no registered substrate provides — a limit
    /// nothing consults is a silent misconfiguration.
    pub fn with_capability_limits(
        mut self,
        limits: BTreeMap<String, u32>,
    ) -> Result<Self, taskmesh_contract::GovernorError> {
        for pool in limits.keys() {
            if !self
                .substrates
                .values()
                .any(|record| record.capability_pool.as_deref() == Some(pool.as_str()))
            {
                return Err(taskmesh_contract::GovernorError::PolicyViolation(
                    format!("capability limit for unregistered pool: {pool}").into(),
                ));
            }
        }
        self.capability_limits = limits;
        Ok(self)
    }

    /// Replace the substrate registry wholesale, **without** re-seeding the
    /// built-ins or checking key/record agreement.
    ///
    /// This exists only so the registry validator can be tested against
    /// registries the safe API cannot produce — an empty inventory, a key that
    /// disagrees with the record it holds, a built-in rebound to the wrong pool.
    /// Gated behind `test-util` so the production surface keeps no way to build
    /// one; [`crate::Governor::new`] rejects it regardless of how it arose.
    #[cfg(feature = "test-util")]
    #[doc(hidden)]
    pub fn forge_substrates(mut self, substrates: BTreeMap<String, SubstrateRecord>) -> Self {
        self.capability_names = intern_capability_names(&substrates);
        self.substrates = substrates;
        self
    }

    /// The interned name for a capability pool, if a registered substrate
    /// provides it. Cloning the returned `Arc` does not allocate.
    pub fn capability_name(&self, pool: &str) -> Option<CapabilityName> {
        self.capability_names.get(pool).cloned()
    }

    pub fn class(&self, class: &TaskClass) -> Option<&ClassPolicy> {
        self.classes.get(class)
    }

    /// The registered substrate inventory, keyed by name.
    pub fn substrates(&self) -> &BTreeMap<String, SubstrateRecord> {
        &self.substrates
    }

    /// Declared capability-pool limits, keyed by capability-pool name.
    pub fn capability_limits(&self) -> &BTreeMap<String, u32> {
        &self.capability_limits
    }

    /// The configured slot limit for `pool`; `0` means ungated.
    pub fn capability_limit(&self, pool: &str) -> u32 {
        self.capability_limits.get(pool).copied().unwrap_or(0)
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
    /// Exact CPU units attributed to this root's children.
    pub cpu_units: u128,
    /// Exact memory units attributed to this root's children.
    pub memory_units: u128,
    pub active_stages: usize,
}

/// Result of a leak sweep over outstanding permits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeakSweepReport {
    /// Permits whose capacity the sweep actually returned.
    pub reclaimed_permits: u32,
    /// Stale, leak-detecting permits the sweep observed — reclaimed or not.
    pub suspected_leaks: u32,
    /// Suspected permits left charged because their work had already reached an
    /// executor. Stale is not dead: reclaiming these would double-book capacity
    /// a live worker still holds.
    pub retained_active: u32,
}

fn intern_capability_names(
    substrates: &BTreeMap<String, SubstrateRecord>,
) -> BTreeMap<String, CapabilityName> {
    substrates
        .values()
        .filter_map(|record| record.capability_pool.as_deref())
        .map(|pool| (pool.to_owned(), CapabilityName::from(pool)))
        .collect()
}

/// What a stage-boundary memory release did.
///
/// A rejected release is reported, not folded into "freed 0 units": a caller
/// cannot tell an enforced policy from a permit that happened to hold nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[must_use = "a refused or stale stage release is not a release; check the outcome"]
pub enum StageReleaseOutcome {
    /// Units were returned to the pool.
    Released { freed_units: u32 },
    /// The class's [`taskmesh_contract::MemoryReleasePolicy`] does not permit a
    /// stage-boundary release (D02).
    PolicyForbids {
        policy: taskmesh_contract::MemoryReleasePolicy,
    },
    /// No such live permit.
    UnknownPermit,
    /// A stage event with this sequence (or a newer one) was already applied.
    StaleSequence { current_sequence: u64 },
}

impl StageReleaseOutcome {
    /// Units actually returned; `0` for every non-release outcome.
    pub fn freed_units(self) -> u32 {
        match self {
            Self::Released { freed_units } => freed_units,
            _ => 0,
        }
    }

    pub fn is_released(self) -> bool {
        matches!(self, Self::Released { .. })
    }
}

/// The outcome of an admission decision at the engine boundary.
///
/// `#[must_use]`: a dropped `Admitted` is a permit nobody will ever release.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "an Admitted permit that is dropped is leaked capacity; handle every arm"]
pub enum AdmissionDecision {
    /// Admitted immediately; caller owns `permit_id` until release.
    Admitted { permit_id: PermitId },
    /// Enqueued; caller waits and later [`crate::Governor::claim`]s `ticket`.
    Queued { ticket: Ticket },
    /// Rejected with a terminal verdict.
    Rejected(taskmesh_contract::AdmissionVerdict),
}
