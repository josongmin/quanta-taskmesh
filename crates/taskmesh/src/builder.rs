//! The runtime builder (T02): a convenience layer that assembles a validated
//! [`RuntimeConfig`], registers substrate inventory, runs fail-closed validation,
//! and wires the default CPU executor.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    ClassPolicy, CpuExecutor, GovernorError, ResourceBudget, RuntimeConfig, SubstrateRecord,
    TaskClass, TopologyConfig,
};
use taskmesh_engine::{builtin_records, Governor, PolicySet, SystemClock};

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

    /// Register an additional substrate beyond the built-in canonical set.
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
        let config = RuntimeConfig {
            topology: self.topology.clone(),
            resources: self.resources.clone(),
            classes: self.classes.clone(),
        };

        // Built-in canonical inventory first, then caller additions; the registry
        // rejects duplicates and invalid records.
        let mut substrates = builtin_records();
        substrates.extend(self.substrates);

        let policy = PolicySet::new(self.resources, self.classes).with_substrates(substrates)?;
        // `Governor::new` validates the policy fail-closed (impossible budgets,
        // mixed-tier fairness, invalid memory scaling, misused degrade/queue).
        let governor = Arc::new(Governor::new(policy, Arc::new(SystemClock))?);
        let cpu: Arc<dyn CpuExecutor> = self
            .cpu_executor
            .unwrap_or_else(|| default_cpu_executor(&self.topology));

        Ok(TokioRuntime::new(config, governor, cpu))
    }
}

/// The default CPU executor when the caller does not supply one. With the
/// `rayon` feature this is the shared Rayon pool sized from the topology;
/// otherwise it is the Tokio blocking pool. Either way `run_cpu` rides the
/// `CpuExecutor` port — never a hardcoded `spawn_blocking`.
#[cfg(feature = "rayon")]
fn default_cpu_executor(topology: &TopologyConfig) -> Arc<dyn CpuExecutor> {
    Arc::new(taskmesh_rayon::RayonCpuExecutor::from_topology(topology))
}

#[cfg(not(feature = "rayon"))]
fn default_cpu_executor(_topology: &TopologyConfig) -> Arc<dyn CpuExecutor> {
    Arc::new(BlockingPoolCpuExecutor)
}
