//! `taskmesh-contract` — the frozen public vocabulary and port surface of
//! taskmesh.
//!
//! This crate is the boundary of the hexagon. It owns two things and nothing
//! else:
//!
//! 1. **Data contract** — product-neutral wire/domain types ([`TaskSpec`],
//!    [`ClassPolicy`], [`AdmissionVerdict`], [`Snapshot`], …). Pure data, no
//!    behavior beyond constructors and builders.
//! 2. **Port surface** — the trait seams adapters implement: the driving port
//!    [`Runtime`] and the driven ports [`Clock`], [`CpuExecutor`],
//!    [`PermitWaker`].
//!
//! The governance engine (`taskmesh-engine`) sits inside these ports; the Tokio
//! host and the Rayon executor sit outside them. Neither validation logic nor
//! runtime primitives belong here.
//!
//! ## Naming & freeze boundary
//!
//! The canonical public names are frozen: `TaskSpec`, `TaskClass`, `TaskStage`,
//! `SubstrateHint`, `AdmissionVerdict`, `GovernorError`, `RunError`, `Snapshot`,
//! `SubstrateRecord`, plus the [`Runtime`] trait. Builder/`new()` helpers are a
//! convenience layer over these frozen types, not part of the freeze target.
//!
//! ## Derive policy
//!
//! - identifier / value object: `Debug + Clone + Eq + Ord + Hash + Serde`
//! - config / policy: `Debug + Clone + Eq + Serde`
//! - runtime-only error ([`GovernorError`], [`RunError`]): not serializable

mod config;
mod policy;
mod ports;
mod resource;
mod runtime;
mod snapshot;
mod task;
mod topology;
mod verdict;

pub use config::RuntimeConfig;
pub use policy::{
    CancellationPolicy, CheckpointPolicy, ClassPolicy, DeterministicReducePolicy,
    DuplicateMergePolicy, ErrorAggregationPolicy, FairnessPolicy, MemoryOvercommitPolicy,
    MemoryPermitMode, MemoryReleasePolicy, OverflowPolicy, PartialResultOrdering, RetryAfterPolicy,
    TieBreakPolicy,
};
pub use ports::{Clock, CpuExecutor, ManualClock, PermitWaker, SystemClock};
pub use resource::{MemoryUnitScale, PermitCost, ResourceBudget};
pub use runtime::Runtime;
pub use snapshot::{ClassSnapshot, Snapshot};
pub use task::{
    ClassificationRationale, PlanSource, StageDescriptor, TaskClass, TaskScope, TaskSpec, TaskStage,
};
pub use topology::{
    CpuMode, CpuPoolConfig, SubstrateHint, SubstrateKind, SubstrateRecord, TopologyConfig,
};
pub use verdict::{AdmissionVerdict, GovernorError, RunError};
