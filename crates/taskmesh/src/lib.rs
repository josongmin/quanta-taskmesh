//! `taskmesh` — the governed execution control-plane, Tokio host facade.
//!
//! This crate is the host on the driving side of the hexagon. It wires the pure
//! governance engine (`taskmesh-engine`) to Tokio: it implements the [`Runtime`]
//! driving port as [`TokioRuntime`], supplies the default [`ext::CpuExecutor`]
//! (the blocking pool) and the [`ext::PermitWaker`] (Tokio `Notify`), and adds
//! the cancel/deadline adapter. The Rayon CPU executor plugs in via the same
//! port without this crate depending on it.
//!
//! ```no_run
//! use taskmesh::{Builder, ClassPolicy, ResourceBudget, Runtime, TaskClass, TaskSpec, TopologyConfig};
//!
//! # async fn demo() {
//! let runtime = Builder::new()
//!     .topology(TopologyConfig::new().cpu_auto().reserve_cores(1).blocking_threads(8))
//!     .resources(ResourceBudget::new().cpu_units(64).memory_units(256))
//!     .class_policy(
//!         TaskClass::new("retrieval"),
//!         ClassPolicy::new().max_inflight(32).max_queue_depth(128).cpu_units(1).memory_units(2),
//!     )
//!     .build()
//!     .expect("runtime must build");
//!
//! let spec = TaskSpec::blocking(TaskClass::new("retrieval")).operation("search:repo:123");
//! let out: String = runtime
//!     .run_blocking(spec, || Ok::<_, std::convert::Infallible>("ok".to_string()))
//!     .await
//!     .expect("governor and task must succeed");
//! assert_eq!(out, "ok");
//! # }
//! ```

mod adapters;
mod builder;
mod executor;
mod runtime;

// ---- public SDK surface ---------------------------------------------------
//
// The everyday surface: the builder, the runtime handle, the `Runtime` driving
// port, the frozen task/policy/topology vocabulary, verdicts, and observation
// types. One crate to depend on; engine internals live under `ext`.

pub use builder::Builder;
pub use executor::{SubmissionDeadline, SubmitOptions};
pub use runtime::TokioRuntime;
pub use tokio_util::sync::CancellationToken;

pub use taskmesh_contract::{
    // driving port, outcomes, observation, resolved config
    AdmissionVerdict,
    // semantic policy
    CancellationPolicy,
    CheckpointPolicy,
    ClassPolicy,
    ClassSnapshot,
    // task description (product-neutral)
    ClassificationRationale,
    // worker governance (topology / resources)
    CpuMode,
    CpuPoolConfig,
    DeterministicReducePolicy,
    DuplicateMergePolicy,
    ErrorAggregationPolicy,
    FairnessPolicy,
    GovernorError,
    MemoryOvercommitPolicy,
    MemoryPermitMode,
    MemoryReleasePolicy,
    MemoryUnitScale,
    OverflowPolicy,
    PartialResultOrdering,
    PermitCost,
    PlanSource,
    ResourceBudget,
    RetryAfterPolicy,
    RunError,
    Runtime,
    RuntimeConfig,
    Snapshot,
    StageDescriptor,
    SubstrateHint,
    SubstrateKind,
    SubstrateRecord,
    TaskClass,
    TaskScope,
    TaskSpec,
    TaskStage,
    TieBreakPolicy,
    TopologyConfig,
};

// ---- ext: advanced integrator surface -------------------------------------

/// Extension points for advanced integrators.
///
/// Everyday SDK usage never needs this module; it exposes the driven ports (to
/// write custom adapters), the default host CPU executor, and direct access to
/// the governance engine.
pub mod ext {
    /// Driven ports: implement these to plug in a custom substrate or clock.
    pub use taskmesh_contract::{Clock, CpuExecutor, ManualClock, PermitWaker, SystemClock};

    /// The default host CPU executor adapter (Tokio blocking pool).
    pub use crate::executor::BlockingPoolCpuExecutor;

    /// The governance engine and its admission primitives, for embedding in a
    /// non-Tokio host or driving admission directly.
    pub use taskmesh_engine::{
        builtin_records, AdmissionDecision, Governor, LeakSweepReport, PermitId, PolicySet,
        Provenance, RootAttribution, Ticket, BUILTIN_SUBSTRATES, DEFAULT_LEAK_STALE_MS,
    };
}
