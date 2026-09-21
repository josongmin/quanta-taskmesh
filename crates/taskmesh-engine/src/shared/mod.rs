//! Cross-slice kernel: identifiers, the resolved policy set, and the value types
//! every feature shares. No feature logic lives here.

use std::borrow::Cow;
use std::cmp::Ordering as CmpOrdering;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::num::NonZeroU32;
#[cfg(feature = "test-util")]
use std::sync::atomic::{AtomicUsize, Ordering};

use taskmesh_contract::{ClassPolicy, ResourceBudget, SubstrateRecord, TaskClass};

/// Identity of a granted permit.
pub type PermitId = u64;

/// Name of a capability pool, shared cheaply between pending requests, permits,
/// and the occupancy ledger.
pub type CapabilityName = std::sync::Arc<str>;

/// Registry-issued identity of a capability pool. The inner name is private so
/// admission cannot turn an arbitrary string into a capability handle.
#[derive(Debug)]
struct CapabilityAuthority;

#[derive(Debug, Clone)]
pub struct CapabilityId {
    name: CapabilityName,
    authority: std::sync::Arc<CapabilityAuthority>,
}

impl CapabilityId {
    pub(crate) fn as_str(&self) -> &str {
        &self.name
    }
}

impl PartialEq for CapabilityId {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && std::sync::Arc::ptr_eq(&self.authority, &other.authority)
    }
}

impl Eq for CapabilityId {}

impl PartialOrd for CapabilityId {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for CapabilityId {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        self.name.cmp(&other.name).then_with(|| {
            std::sync::Arc::as_ptr(&self.authority).cmp(&std::sync::Arc::as_ptr(&other.authority))
        })
    }
}

impl Hash for CapabilityId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        std::sync::Arc::as_ptr(&self.authority).hash(state);
    }
}

impl std::fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Capability selected by a validated execution plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedCapability {
    Ungated,
    Registered(CapabilityId),
}

/// Capacity policy attached to every registered capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityCapacity {
    Bounded(NonZeroU32),
    ExplicitUnbounded,
}

impl CapabilityCapacity {
    pub fn limit(self) -> u32 {
        match self {
            Self::Bounded(limit) => limit.get(),
            Self::ExplicitUnbounded => 0,
        }
    }
}

/// Single authority record used by name resolution, capacity, and snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityRecord {
    id: CapabilityId,
    capacity: CapabilityCapacity,
}

impl CapabilityRecord {
    pub fn id(&self) -> &CapabilityId {
        &self.id
    }

    pub fn name(&self) -> &str {
        self.id.as_str()
    }

    pub fn capacity(&self) -> CapabilityCapacity {
        self.capacity
    }
}

/// Raw pool-name resolution failed before admission state was touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityResolutionError {
    EmptyName,
    UnknownName { name: String },
    ForeignId,
    TooManyRequirements { max: usize },
}

impl std::fmt::Display for CapabilityResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyName => f.write_str("capability name must be non-empty"),
            Self::UnknownName { name } => write!(f, "unknown capability: {name}"),
            Self::ForeignId => f.write_str("capability id is not registered by this policy"),
            Self::TooManyRequirements { max } => {
                write!(
                    f,
                    "capability requirement count exceeds bounded maximum {max}"
                )
            }
        }
    }
}

impl std::error::Error for CapabilityResolutionError {}

pub const MAX_CAPABILITY_REQUIREMENTS: usize = 32;

/// Bounded, canonical set of capability slots charged atomically by one permit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CapabilityRequirementSet(Vec<CapabilityId>);

impl CapabilityRequirementSet {
    // This named constructor is intentionally identical to `Default`; exclude
    // only that mechanically equivalent whole-function replacement while still
    // mutating every non-equivalent operation in this impl.
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from_resolved(
        capabilities: impl IntoIterator<Item = ResolvedCapability>,
    ) -> Result<Self, CapabilityResolutionError> {
        let mut ids = Vec::new();
        for capability in capabilities {
            if let ResolvedCapability::Registered(id) = capability {
                match ids.binary_search(&id) {
                    Ok(_) => {}
                    Err(index) => ids.insert(index, id),
                }
                if ids.len() > MAX_CAPABILITY_REQUIREMENTS {
                    return Err(CapabilityResolutionError::TooManyRequirements {
                        max: MAX_CAPABILITY_REQUIREMENTS,
                    });
                }
            }
        }
        Ok(Self(ids))
    }

    pub fn iter(&self) -> impl Iterator<Item = &CapabilityId> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn intersects(&self, other: &Self) -> bool {
        if self.is_empty() && other.is_empty() {
            return true;
        }
        self.0.iter().any(|id| other.0.binary_search(id).is_ok())
    }
}

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
    capabilities: BTreeMap<CapabilityId, CapabilityRecord>,
    capability_authority: std::sync::Arc<CapabilityAuthority>,
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
        let capability_authority = std::sync::Arc::new(CapabilityAuthority);
        let capabilities = builtin_capability_records(&substrates, &capability_authority);
        Self {
            resources,
            classes,
            substrates,
            capabilities,
            capability_authority,
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

    /// Declare capability-pool slot limits, keyed by capability-pool name.
    ///
    /// This is the **single** physical-capacity authority: the engine checks it
    /// in the same transition as class inflight and the resource budget, so
    /// there is no second queue in front of admission for work to pile up in. A
    /// `0` is explicit-unbounded. Missing authority is never interpreted as
    /// ungated and is rejected when the policy becomes live.
    ///
    /// Rejects a limit for a pool no registered substrate provides — a limit
    /// nothing consults is a silent misconfiguration.
    pub fn with_capability_limits(
        mut self,
        limits: BTreeMap<String, u32>,
    ) -> Result<Self, taskmesh_contract::GovernorError> {
        for (pool, limit) in limits {
            if pool.trim().is_empty() {
                return Err(taskmesh_contract::GovernorError::PolicyViolation(
                    "capability name must be non-empty".into(),
                ));
            }
            let executing = self.substrates.values().any(|record| {
                record.kind != taskmesh_contract::SubstrateKind::AuthorityOnly
                    && record.capability_pool.as_deref() == Some(pool.as_str())
            });
            if !executing {
                return Err(taskmesh_contract::GovernorError::PolicyViolation(
                    format!("capability limit for unregistered pool: {pool}").into(),
                ));
            }
            let id = self
                .capabilities
                .keys()
                .find(|id| id.as_str() == pool)
                .cloned()
                .unwrap_or_else(|| CapabilityId {
                    name: CapabilityName::from(pool.as_str()),
                    authority: std::sync::Arc::clone(&self.capability_authority),
                });
            let capacity = NonZeroU32::new(limit).map_or(
                CapabilityCapacity::ExplicitUnbounded,
                CapabilityCapacity::Bounded,
            );
            self.capabilities
                .insert(id.clone(), CapabilityRecord { id, capacity });
        }
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
        self.substrates = substrates;
        self
    }

    /// The interned name for an authority-backed capability pool.
    pub fn capability_name(&self, pool: &str) -> Option<CapabilityName> {
        self.capabilities
            .keys()
            .find(|id| id.as_str() == pool)
            .map(|id| std::sync::Arc::clone(&id.name))
    }

    /// Resolve a raw name before admission. Unknown and empty names never
    /// allocate ids or touch governed state.
    pub fn resolve_capability(
        &self,
        pool: &str,
    ) -> Result<ResolvedCapability, CapabilityResolutionError> {
        if pool.trim().is_empty() {
            return Err(CapabilityResolutionError::EmptyName);
        }
        self.capabilities
            .keys()
            .find(|id| id.as_str() == pool)
            .cloned()
            .map(ResolvedCapability::Registered)
            .ok_or_else(|| CapabilityResolutionError::UnknownName {
                name: pool.to_owned(),
            })
    }

    pub(crate) fn validate_resolved_capability(
        &self,
        capability: &ResolvedCapability,
    ) -> Result<(), CapabilityResolutionError> {
        match capability {
            ResolvedCapability::Ungated => Ok(()),
            ResolvedCapability::Registered(id) if self.capabilities.contains_key(id) => Ok(()),
            ResolvedCapability::Registered(_) => Err(CapabilityResolutionError::ForeignId),
        }
    }

    pub(crate) fn validate_requirements(
        &self,
        requirements: &CapabilityRequirementSet,
    ) -> Result<(), CapabilityResolutionError> {
        if requirements
            .iter()
            .all(|id| self.capabilities.contains_key(id))
        {
            Ok(())
        } else {
            Err(CapabilityResolutionError::ForeignId)
        }
    }

    pub fn resolved_capability_name<'a>(
        &'a self,
        capability: &ResolvedCapability,
    ) -> Option<&'a str> {
        match capability {
            ResolvedCapability::Ungated => None,
            ResolvedCapability::Registered(id) => {
                self.capabilities.get(id).map(CapabilityRecord::name)
            }
        }
    }

    pub(crate) fn capability_record(&self, id: &CapabilityId) -> Option<&CapabilityRecord> {
        self.capabilities.get(id)
    }

    pub fn capability_records(&self) -> impl Iterator<Item = &CapabilityRecord> {
        self.capabilities.values()
    }

    pub fn class(&self, class: &TaskClass) -> Option<&ClassPolicy> {
        self.classes.get(class)
    }

    /// The registered substrate inventory, keyed by name.
    pub fn substrates(&self) -> &BTreeMap<String, SubstrateRecord> {
        &self.substrates
    }

    /// Missing is distinct from [`CapabilityCapacity::ExplicitUnbounded`].
    pub fn capability_capacity(&self, pool: &str) -> Option<CapabilityCapacity> {
        self.capabilities
            .values()
            .find(|record| record.name() == pool)
            .map(CapabilityRecord::capacity)
    }
}

fn builtin_capability_records(
    substrates: &BTreeMap<String, SubstrateRecord>,
    authority: &std::sync::Arc<CapabilityAuthority>,
) -> BTreeMap<CapabilityId, CapabilityRecord> {
    let mut capabilities = BTreeMap::new();
    for record in substrates.values() {
        let Some(pool) = record.capability_pool.as_deref() else {
            continue;
        };
        let id = CapabilityId {
            name: CapabilityName::from(pool),
            authority: std::sync::Arc::clone(authority),
        };
        capabilities.entry(id.clone()).or_insert(CapabilityRecord {
            id,
            capacity: CapabilityCapacity::ExplicitUnbounded,
        });
    }
    capabilities
}

/// Classification provenance carried through admission so "why this class" stays
/// auditable in runtime state after submit, not just on the inbound `TaskSpec`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    pub source: taskmesh_contract::PlanSource,
    pub reason: taskmesh_contract::ClassificationRationale,
}

impl Provenance {
    pub fn of(spec: &taskmesh_contract::TaskSpec) -> Self {
        Self {
            source: spec.source.clone(),
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

#[cfg(test)]
mod mutation_semantics {
    use super::*;
    use std::collections::BTreeMap;
    use std::hash::{Hash, Hasher};

    struct FixedHasher(u64);

    impl Default for FixedHasher {
        fn default() -> Self {
            Self(0xcbf2_9ce4_8422_2325)
        }
    }

    impl Hasher for FixedHasher {
        fn finish(&self) -> u64 {
            self.0
        }

        fn write(&mut self, bytes: &[u8]) {
            for byte in bytes {
                self.0 ^= u64::from(*byte);
                self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }

    fn policy_with_pools(count: usize) -> PolicySet {
        let records = (0..count)
            .map(|index| {
                let name = format!("executor-{index}");
                let pool = format!("pool-{index}");
                SubstrateRecord::new(
                    name,
                    taskmesh_contract::SubstrateKind::CompetingExecution,
                    Some(pool),
                )
            })
            .collect::<Vec<_>>();
        let limits = (0..count)
            .map(|index| (format!("pool-{index}"), 1))
            .collect();
        PolicySet::new(ResourceBudget::new(), BTreeMap::new())
            .with_substrates(records)
            .expect("unique executing substrates")
            .with_capability_limits(limits)
            .expect("every capability has an executing substrate")
    }

    fn hash(value: &CapabilityId) -> u64 {
        let mut hasher = FixedHasher::default();
        value.hash(&mut hasher);
        hasher.finish()
    }

    fn registered(capability: ResolvedCapability) -> Option<CapabilityId> {
        match capability {
            ResolvedCapability::Registered(id) => Some(id),
            ResolvedCapability::Ungated => None,
        }
    }

    #[test]
    fn capability_identity_binds_name_and_issuing_authority() {
        let first_policy = PolicySet::new(ResourceBudget::new(), BTreeMap::new());
        let second_policy = PolicySet::new(ResourceBudget::new(), BTreeMap::new());
        let first = registered(first_policy.resolve_capability("cpu").expect("built-in"))
            .expect("cpu is registered");
        let clone = first.clone();
        let foreign = registered(second_policy.resolve_capability("cpu").expect("built-in"))
            .expect("cpu is registered");

        assert_eq!(first, clone);
        assert_eq!(first.partial_cmp(&clone), Some(CmpOrdering::Equal));
        assert_eq!(hash(&first), hash(&clone));
        assert_ne!(first, foreign);
        assert_ne!(hash(&first), hash(&foreign));
        assert_eq!(first.to_string(), "cpu");
    }

    #[test]
    fn requirement_bound_is_inclusive_and_foreign_ids_fail_closed() {
        let policy = policy_with_pools(MAX_CAPABILITY_REQUIREMENTS + 1);
        let resolved: Vec<_> = (0..=MAX_CAPABILITY_REQUIREMENTS)
            .map(|index| {
                policy
                    .resolve_capability(&format!("pool-{index}"))
                    .expect("registered")
            })
            .collect();
        let maximum = CapabilityRequirementSet::from_resolved(
            resolved
                .get(..MAX_CAPABILITY_REQUIREMENTS)
                .expect("fixture contains the documented maximum")
                .iter()
                .cloned(),
        )
        .expect("the documented maximum is admissible");
        assert_eq!(maximum.iter().count(), MAX_CAPABILITY_REQUIREMENTS);
        assert_eq!(policy.validate_requirements(&maximum), Ok(()));
        assert_eq!(
            CapabilityRequirementSet::from_resolved(resolved),
            Err(CapabilityResolutionError::TooManyRequirements {
                max: MAX_CAPABILITY_REQUIREMENTS,
            })
        );

        let foreign_policy = policy_with_pools(1);
        let foreign = CapabilityRequirementSet::from_resolved([foreign_policy
            .resolve_capability("pool-0")
            .expect("registered")])
        .expect("one requirement");
        assert_eq!(
            policy.validate_requirements(&foreign),
            Err(CapabilityResolutionError::ForeignId)
        );
    }

    #[test]
    fn request_key_accessors_preserve_the_authoritative_root() {
        let key = RequestKey::from_root("root:42");
        assert_eq!(key.as_str(), "root:42");
        assert_eq!(key.to_string(), "root:42");
    }

    #[test]
    fn capability_resolution_errors_render_exact_context() {
        assert_eq!(
            CapabilityResolutionError::EmptyName.to_string(),
            "capability name must be non-empty"
        );
        assert_eq!(
            CapabilityResolutionError::UnknownName {
                name: "gpu".to_owned(),
            }
            .to_string(),
            "unknown capability: gpu"
        );
        assert_eq!(
            CapabilityResolutionError::ForeignId.to_string(),
            "capability id is not registered by this policy"
        );
        assert_eq!(
            CapabilityResolutionError::TooManyRequirements { max: 32 }.to_string(),
            "capability requirement count exceeds bounded maximum 32"
        );
    }
}
