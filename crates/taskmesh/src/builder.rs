//! The runtime builder (T02): a convenience layer that assembles a validated
//! [`RuntimeConfig`], registers substrate inventory, runs fail-closed validation,
//! and wires the default CPU executor.

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
    /// invalid memory scaling, misused degrade policy, or duplicate substrate.
    pub fn build(self) -> Result<TokioRuntime, GovernorError> {
        // `PolicySet::new` already seeds the canonical built-in inventory; here we
        // only layer caller additions on top. The registry rejects duplicates
        // (including any attempt to shadow a built-in) and invalid records.
        let policy = PolicySet::new(self.resources.clone(), self.classes.clone())
            .with_substrates(self.substrates)?;

        // The config captures the *resolved* substrate inventory, so `config()`
        // is a faithful, serializable description of the built runtime.
        let config = RuntimeConfig {
            topology: self.topology.clone(),
            resources: self.resources,
            classes: self.classes,
            substrates: policy.substrates.values().cloned().collect(),
        };

        // `Governor::new` validates the policy fail-closed (impossible budgets,
        // mixed-tier fairness, invalid memory scaling, misused degrade/queue).
        let governor = Arc::new(Governor::new(policy, Arc::new(SystemClock))?);
        let cpu: Arc<dyn CpuExecutor> = match self.cpu_executor {
            Some(cpu) => cpu,
            None => default_cpu_executor(&self.topology)?,
        };

        Ok(TokioRuntime::new(config, governor, cpu))
    }
}

/// The default CPU executor when the caller does not supply one. With the
/// `rayon` feature this is the shared Rayon pool sized from the topology;
/// otherwise it is the Tokio blocking pool. Either way `run_cpu` rides the
/// `CpuExecutor` port — never a hardcoded `spawn_blocking`. Fallible so a pool
/// build failure surfaces as a `build()` error, never a panic (fail-closed).
#[cfg(feature = "rayon")]
fn default_cpu_executor(topology: &TopologyConfig) -> Result<Arc<dyn CpuExecutor>, GovernorError> {
    let pool = taskmesh_rayon::RayonCpuExecutor::try_from_topology(topology).map_err(|e| {
        GovernorError::PolicyViolation(format!("failed to build shared CPU pool: {e}").into())
    })?;
    Ok(Arc::new(pool))
}

#[cfg(not(feature = "rayon"))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "signature mirrors the fallible rayon variant so build() is uniform"
)]
fn default_cpu_executor(_topology: &TopologyConfig) -> Result<Arc<dyn CpuExecutor>, GovernorError> {
    Ok(Arc::new(BlockingPoolCpuExecutor))
}
