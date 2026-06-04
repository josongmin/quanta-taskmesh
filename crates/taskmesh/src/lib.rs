//! `taskmesh` — the governed execution control-plane, Tokio host facade.
//!
//! This crate is the host on the driving side of the hexagon. It wires the pure
//! governance engine (`taskmesh-engine`) to Tokio: it implements the [`Runtime`]
//! driving port as [`TokioRuntime`], supplies the default [`CpuExecutor`]
//! (the blocking pool) and the [`PermitWaker`] (Tokio `Notify`), and adds the
//! cancel/deadline adapter. The Rayon CPU executor plugs in via the same port
//! without this crate depending on it.
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

pub use builder::Builder;
pub use executor::{BlockingPoolCpuExecutor, SubmitOptions};
pub use runtime::TokioRuntime;

// Re-export the frozen contract surface so callers depend on one crate.
pub use taskmesh_contract::*;

// Re-export the governance engine surface callers need (advanced governance).
pub use taskmesh_engine::{
    builtin_records, AdmissionDecision, Governor, LeakSweepReport, PermitId, PolicySet, RequestKey,
    RootAttribution, Ticket, BUILTIN_SUBSTRATES, DEFAULT_LEAK_STALE_MS,
};

// The cancellation primitive used by `SubmitOptions`.
pub use tokio_util::sync::CancellationToken;
