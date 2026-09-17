//! The runtime builder (T02): a convenience layer that assembles a validated
//! [`RuntimeConfig`], registers substrate inventory, runs fail-closed validation,
//! and wires the default CPU executor.
//!
//! # One resolution of the machine
//!
//! Detected parallelism is read **once** here and shared. Reading it per
//! consumer invites two components to size themselves from two different
//! answers — a CPU pool built for one worker count and a capacity gate built for
//! another — and the disagreement only shows up as unexplained queueing under
//! load.
//!
//! # Topology is validated before it is resolved
//!
//! `TopologyConfig` is public, deserializable input. An inverted worker window
//! or an unrepresentable slot count is a typed `Err` from [`Builder::build`],
//! never a panic inside a clamp or a semaphore constructor.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    ClassPolicy, CpuExecutor, GovernorError, ResourceBudget, RuntimeConfig, SubstrateRecord,
    TaskClass, TopologyConfig,
};
use taskmesh_engine::{Governor, PolicySet, SystemClock};

#[cfg(not(feature = "rayon"))]
use crate::executor::BlockingPoolCpuExecutor;
use crate::runtime::TokioRuntime;

/// Fluent builder for a [`TokioRuntime`]. Invalid configurations are rejected at
/// [`Builder::build`], never deferred to a runtime admit path.
#[derive(Clone)]
pub struct Builder {
    topology: TopologyConfig,
    resources: ResourceBudget,
    classes: BTreeMap<TaskClass, ClassPolicy>,
    substrates: Vec<SubstrateRecord>,
    cpu_executor: Option<Arc<dyn CpuExecutor>>,
}

impl Default for Builder {
    fn default() -> Self {
        Self {
            topology: TopologyConfig::new(),
            resources: ResourceBudget::new(),
            classes: BTreeMap::new(),
            substrates: Vec::new(),
            cpu_executor: None,
        }
    }
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn topology(mut self, topology: TopologyConfig) -> Self {
        self.topology = topology;
        self
    }

    pub fn resources(mut self, resources: ResourceBudget) -> Self {
        self.resources = resources;
        self
    }

    pub fn class_policy(mut self, class: TaskClass, policy: ClassPolicy) -> Self {
        self.classes.insert(class, policy);
        self
    }

    /// Register an additional substrate beyond the built-in canonical set, for
    /// **inventory and governance tracking** (T08) — e.g. a `CompetingExecution`
    /// or `RetireOrMigrate` pool the deployment wants visible in the snapshot and
    /// subject to migration governance.
    ///
    /// This is *not* a new run target: tasks can only target the closed
    /// [`taskmesh_contract::SubstrateHint`] set, and the host's execution gates
    /// are wired to the built-ins. A registered extra substrate therefore appears
    /// in the inventory (governor snapshot and [`crate::RuntimeConfig`]) but is
    /// never something a `TaskSpec` schedules onto — that separation is
    /// intentional (inventory authority vs. execution targeting).
    pub fn substrate(mut self, substrate: SubstrateRecord) -> Self {
        self.substrates.push(substrate);
        self
    }

    /// Override the CPU executor (e.g. `taskmesh_rayon::RayonCpuExecutor`).
    /// Defaults to the Tokio blocking pool.
    pub fn cpu_executor(mut self, executor: Arc<dyn CpuExecutor>) -> Self {
        self.cpu_executor = Some(executor);
        self
    }

    /// Validate and build. Returns a [`GovernorError`] for any impossible budget,
    /// invalid topology, invalid memory scaling, misused degrade policy, or
    /// duplicate substrate.
    pub fn build(self) -> Result<TokioRuntime, GovernorError> {
        // Before anything is sized from it.
        self.topology.validate()?;
        let available = detected_parallelism();
        let cpu_workers = self.topology.try_resolved_cpu_workers(available)?;

        // `PolicySet::new` already seeds the canonical built-in inventory; here we
        // only layer caller additions on top. The registry rejects duplicates
        // (including any attempt to shadow a built-in) and invalid records.
        let policy = PolicySet::new(self.resources.clone(), self.classes.clone())
            .with_substrates(self.substrates)?
            .with_capability_limits(capability_limits(&self.topology, cpu_workers)?)?;

        // The config captures the *resolved* substrate inventory, so `config()`
        // is a faithful, serializable description of the built runtime.
        let config = RuntimeConfig {
            topology: self.topology.clone(),
            resources: self.resources,
            classes: self.classes,
            substrates: policy.substrates().values().cloned().collect(),
        };

        // `Governor::new` validates the policy fail-closed (impossible budgets,
        // mixed-tier fairness, invalid memory scaling, misused degrade/queue,
        // and the completeness of the substrate registry itself).
        let governor = Arc::new(Governor::new(policy, Arc::new(SystemClock))?);
        let cpu: Arc<dyn CpuExecutor> = match self.cpu_executor {
            Some(cpu) => cpu,
            None => default_cpu_executor(cpu_workers)?,
        };
        // D05: the adapter's declaration is checked against the gate built from
        // topology. A `cpu` gate wider than the workers the adapter says it has
        // would admit work the pool cannot run concurrently — the exact
        // disagreement this builder exists to prevent — so it is rejected here,
        // typed, rather than discovered as unexplained queueing under load.
        let declared = cpu.capabilities();
        if let Some(declared_workers) = declared.declared_workers {
            // Compared in `u64` so neither side is truncated: a declaration that
            // does not fit `usize` is *more* workers, not fewer.
            let resolved = u64::try_from(cpu_workers).unwrap_or(u64::MAX);
            if u64::from(declared_workers) < resolved {
                return Err(GovernorError::InvalidTopology(
                    taskmesh_contract::TopologyError::ExecutorDeclaresFewerWorkers {
                        declared: declared_workers,
                        resolved: cpu_workers,
                    },
                ));
            }
        }

        Ok(TokioRuntime::new(config, governor, cpu))
    }
}

/// Detected parallelism, read once per built runtime.
fn detected_parallelism() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

/// The capability-pool slot limits the engine enforces, derived from topology.
///
/// `0` keeps its published meaning — *no limit* — for every pool except `cpu`,
/// which is always gated to the resolved worker count so CPU topology is
/// authoritative on both host paths (Tokio blocking pool and Rayon).
fn capability_limits(
    topology: &TopologyConfig,
    cpu_workers: usize,
) -> Result<BTreeMap<String, u32>, GovernorError> {
    let mut limits = BTreeMap::new();
    let mut insert = |pool: &str, slots: usize| -> Result<(), GovernorError> {
        if slots == 0 {
            return Ok(());
        }
        let slots = u32::try_from(slots).map_err(|_too_large| {
            GovernorError::InvalidTopology(taskmesh_contract::TopologyError::SlotCountTooLarge {
                // `pool` is one of the fixed built-in names below.
                pool: builtin_pool_name(pool),
                slots,
                max: taskmesh_contract::MAX_CAPABILITY_SLOTS,
            })
        })?;
        limits.insert(pool.to_owned(), slots);
        Ok(())
    };
    insert("cpu", cpu_workers.max(1))?;
    for (pool, slots) in topology.declared_slots() {
        insert(pool, slots)?;
    }
    Ok(limits)
}

/// Map a pool name back to its `'static` built-in spelling for error reporting.
fn builtin_pool_name(pool: &str) -> &'static str {
    match pool {
        "cpu" => "cpu",
        "blocking" => "blocking",
        "large_stack" => "large_stack",
        "maintenance" => "maintenance",
        "local_runtime" => "local_runtime",
        _ => "unknown",
    }
}

/// The default CPU executor when the caller does not supply one. With the
/// `rayon` feature this is the shared Rayon pool sized from the *already
/// resolved* worker count; otherwise it is the Tokio blocking pool. Either way
/// `run_cpu` rides the `CpuExecutor` port — never a hardcoded `spawn_blocking`.
/// Fallible so a pool build failure surfaces as a `build()` error, never a panic.
#[cfg(feature = "rayon")]
fn default_cpu_executor(cpu_workers: usize) -> Result<Arc<dyn CpuExecutor>, GovernorError> {
    let pool = taskmesh_rayon::RayonCpuExecutor::try_new(cpu_workers).map_err(|e| {
        GovernorError::PolicyViolation(format!("failed to build shared CPU pool: {e}").into())
    })?;
    Ok(Arc::new(pool))
}

#[cfg(not(feature = "rayon"))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "signature mirrors the fallible rayon variant so build() is uniform"
)]
fn default_cpu_executor(_cpu_workers: usize) -> Result<Arc<dyn CpuExecutor>, GovernorError> {
    Ok(Arc::new(BlockingPoolCpuExecutor))
}
