//! Observable runtime state.
//!
//! # Exactness (D06)
//!
//! Held-resource aggregates are **exact**, not narrowed. A per-request cost is a
//! `u32`, but a class or a whole runtime may legitimately hold more than `u32`
//! units at once (an unlimited budget is written as `0`), so the aggregate view
//! is a `u128`. Narrowing it to `u32` on the wire would report *less* resource
//! than is actually held — the precise failure that lets an over-budget
//! admission look healthy — so the wire form is an exact decimal string instead.
//!
//! # Phases
//!
//! A live request occupies exactly one *ownership phase*
//! ([`ExecutionPhase`]). `inflight` is the sum of the phase gauges; it never
//! double-counts, and a caller-visible response (a deadline reply, a cancel)
//! does not move a request out of its phase — only the execution owner does.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::task::TaskClass;
use crate::topology::SubstrateRecord;

/// Wire schema version of [`Snapshot`]. Bumped whenever the meaning or the
/// representation of a field changes, so a consumer can refuse a snapshot it
/// does not understand instead of misreading it.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 2;

/// The ownership phase of one live governed request.
///
/// Phases are **monotonic**: a request advances through them and never goes
/// back. They describe who owns the execution, which is independent of what the
/// caller has already been told — a timed-out caller has its answer while the
/// work is still `Running` or `CleanupPending`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ExecutionPhase {
    /// Admitted and holding capacity; not yet handed to an executor.
    DispatchReserved,
    /// An executor accepted custody but the work has not started. Accounted
    /// separately so an adapter's internal start latency cannot hide as
    /// "pending".
    Accepted,
    /// The work is executing.
    Running,
    /// The work finished but its owner (worker runtime, child teardown) has not
    /// yet released custody. Still charged.
    CleanupPending,
}

impl ExecutionPhase {
    /// Phase order used to enforce monotonic advancement.
    pub fn rank(self) -> u8 {
        match self {
            Self::DispatchReserved => 0,
            Self::Accepted => 1,
            Self::Running => 2,
            Self::CleanupPending => 3,
        }
    }

    /// Whether reaching this phase means the work actually started executing.
    pub fn has_started(self) -> bool {
        matches!(self, Self::Running | Self::CleanupPending)
    }
}

impl fmt::Display for ExecutionPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DispatchReserved => "dispatch_reserved",
            Self::Accepted => "accepted",
            Self::Running => "running",
            Self::CleanupPending => "cleanup_pending",
        })
    }
}

/// Exact-width resource aggregate. Serialized as a decimal **string** so no JSON
/// consumer silently rounds it through an `f64`.
mod exact_units {
    use serde::{de::Error as _, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u128, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u128, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse::<u128>()
            .map_err(|error| D::Error::custom(format!("invalid exact unit value {raw:?}: {error}")))
    }
}

/// Per-class accounting at a point in time.
///
/// Conservation, within one snapshot and one class:
///
/// - `inflight == dispatch_reserved + accepted + running + cleanup_pending`
/// - `admitted_total == inflight as u128 + terminated_total`
/// - `started_total >= (running + cleanup_pending) as u128`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ClassSnapshot {
    /// Live requests holding capacity, in any [`ExecutionPhase`].
    pub inflight: u32,
    /// Requests waiting in the class admission queue (not yet admitted).
    pub queued: u32,
    /// Exact CPU units held by `inflight` requests.
    #[serde(with = "exact_units")]
    pub cpu_units_held: u128,
    /// Exact memory units held by `inflight` requests.
    #[serde(with = "exact_units")]
    pub memory_units_held: u128,
    /// Phase gauges. They partition `inflight`.
    pub dispatch_reserved: u32,
    pub accepted: u32,
    pub running: u32,
    pub cleanup_pending: u32,
    /// Cumulative permits granted for this class. Decimal string on the wire
    /// for the same reason as the held units (D06): a JSON consumer that reads
    /// numbers as `f64` would round a large counter.
    #[serde(with = "exact_units")]
    pub admitted_total: u128,
    /// Cumulative permits that reached [`ExecutionPhase::Running`].
    #[serde(with = "exact_units")]
    pub started_total: u128,
    /// Cumulative permits released (execution terminated).
    #[serde(with = "exact_units")]
    pub terminated_total: u128,
}

impl ClassSnapshot {
    /// Check this class's conservation identities. Returns the first violated
    /// identity, or `None` when the projection is consistent.
    pub fn conservation_violation(&self) -> Option<String> {
        let phases = u128::from(self.dispatch_reserved)
            + u128::from(self.accepted)
            + u128::from(self.running)
            + u128::from(self.cleanup_pending);
        if phases != u128::from(self.inflight) {
            return Some(format!("inflight {} != phase sum {phases}", self.inflight));
        }
        if self.admitted_total != u128::from(self.inflight) + self.terminated_total {
            return Some(format!(
                "admitted_total {} != inflight {} + terminated_total {}",
                self.admitted_total, self.inflight, self.terminated_total
            ));
        }
        let started_live = u128::from(self.running) + u128::from(self.cleanup_pending);
        if self.started_total < started_live {
            return Some(format!(
                "started_total {} < live started {started_live}",
                self.started_total
            ));
        }
        None
    }
}

/// A consistent view of governed state: per-class accounting plus the substrate
/// inventory and capability-pool occupancy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Wire schema version; see [`SNAPSHOT_SCHEMA_VERSION`].
    pub schema_version: u32,
    pub classes: BTreeMap<TaskClass, ClassSnapshot>,
    pub substrates: Vec<SubstrateRecord>,
    /// Capability-pool occupancy by pool name: `(in_use, limit)`. A limit of `0`
    /// means the pool is ungated.
    pub capabilities: BTreeMap<String, CapabilityUsage>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            classes: BTreeMap::new(),
            substrates: Vec::new(),
            capabilities: BTreeMap::new(),
        }
    }
}

impl Snapshot {
    /// Check every class's conservation identities plus capability occupancy.
    /// Returns the first violation found.
    pub fn conservation_violation(&self) -> Option<String> {
        for (class, class_snapshot) in &self.classes {
            if let Some(violation) = class_snapshot.conservation_violation() {
                return Some(format!("class {class}: {violation}"));
            }
        }
        for (pool, usage) in &self.capabilities {
            if usage.limit != 0 && usage.in_use > usage.limit {
                return Some(format!(
                    "capability {pool}: in_use {} exceeds limit {}",
                    usage.in_use, usage.limit
                ));
            }
        }
        None
    }
}

/// Occupancy of one capability pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CapabilityUsage {
    pub in_use: u32,
    /// Configured slot count; `0` means ungated.
    pub limit: u32,
}
