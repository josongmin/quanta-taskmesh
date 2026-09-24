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

use std::collections::{btree_map::Entry, BTreeMap};
use std::sync::Arc;

use taskmesh_contract::{
    ClassPolicy, CpuExecutor, GovernorError, ResourceBudget, RuntimeConfig, SettlementWaker,
    SubstrateKind, SubstrateRecord, TaskClass, TopologyConfig, TopologyError, PHYSICAL_CPU,
    PHYSICAL_DEDICATED, PHYSICAL_SHARED_BLOCKING,
};
use taskmesh_engine::{Governor, PolicySet, SystemClock, BUILTIN_SUBSTRATES};

#[cfg(not(feature = "rayon"))]
use crate::executor::BlockingPoolCpuExecutor;
use crate::runtime::{DrainSignal, TokioRuntime};

/// Fluent builder for a [`TokioRuntime`]. Invalid configurations are rejected at
/// [`Builder::build`], never deferred to a runtime admit path.
#[derive(Clone)]
pub struct Builder {
    topology: TopologyConfig,
    resources: ResourceBudget,
    classes: BTreeMap<TaskClass, ClassPolicy>,
    substrates: Vec<SubstrateRecord>,
    capability_limits: BTreeMap<String, u32>,
    registration_error: Option<GovernorError>,
    cpu_executor: Option<Arc<dyn CpuExecutor>>,
}

impl std::fmt::Debug for Builder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Builder")
            .field("topology", &self.topology)
            .field("resources", &self.resources)
            .field("classes", &self.classes)
            .field("substrates", &self.substrates)
            .field("capability_limits", &self.capability_limits)
            .field("registration_error", &self.registration_error)
            .field(
                "cpu_executor",
                &self.cpu_executor.as_ref().map(|cpu| cpu.capabilities()),
            )
            .finish()
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self {
            topology: TopologyConfig::new(),
            resources: ResourceBudget::new(),
            classes: BTreeMap::new(),
            substrates: Vec::new(),
            capability_limits: BTreeMap::new(),
            registration_error: None,
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

    /// Register a class policy once. Repeating a class records a deterministic
    /// error returned by [`Self::build`], even when both policies are equal.
    pub fn class_policy(mut self, class: TaskClass, policy: ClassPolicy) -> Self {
        match self.classes.entry(class) {
            Entry::Occupied(existing) => {
                self.registration_error.get_or_insert_with(|| {
                    GovernorError::PolicyViolation(
                        format!("duplicate class policy registration: {}", existing.key()).into(),
                    )
                });
            }
            Entry::Vacant(vacant) => {
                vacant.insert(policy);
            }
        }
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

    /// Declare capacity authority for an additional substrate pool. `0` is an
    /// explicit unbounded declaration; omission is rejected by the engine.
    /// Repeating a pool records an error returned by [`Self::build`]. Built-in
    /// semantic and physical pools are topology-owned and cannot be registered
    /// here, including when their topology limit is unbounded.
    pub fn capability_limit(mut self, pool: impl Into<String>, slots: u32) -> Self {
        match self.capability_limits.entry(pool.into()) {
            Entry::Occupied(existing) => {
                self.registration_error.get_or_insert_with(|| {
                    GovernorError::PolicyViolation(
                        format!(
                            "duplicate capability limit registration: {}",
                            existing.key()
                        )
                        .into(),
                    )
                });
            }
            Entry::Vacant(vacant) => {
                vacant.insert(slots);
            }
        }
        self
    }

    /// Override the CPU executor (e.g. `taskmesh_rayon::RayonCpuExecutor`).
    /// Defaults to the Tokio blocking pool.
    pub fn cpu_executor(mut self, executor: Arc<dyn CpuExecutor>) -> Self {
        self.cpu_executor = Some(executor);
        self
    }

    /// Validate and build. Returns a [`GovernorError`] for any impossible budget,
    /// invalid topology, invalid memory scaling, misused degrade policy,
    /// malformed class name, or duplicate class, substrate, or capability pool.
    pub fn build(self) -> Result<TokioRuntime, GovernorError> {
        if let Some(error) = self.registration_error {
            return Err(error);
        }
        // Before anything is sized from it.
        self.topology.validate()?;
        let available = detected_parallelism();
        let cpu_workers = self.topology.try_resolved_cpu_workers(available)?;
        // `try_resolved_cpu_workers` has already validated the complete CPU
        // window against `MAX_CAPABILITY_SLOTS`; repeating that predicate here
        // created an unreachable second authority for the same invariant.
        let shared_blocking_workers = self
            .topology
            .try_resolved_physical_domain(PHYSICAL_SHARED_BLOCKING, available)?;
        let dedicated_workers = self
            .topology
            .try_resolved_physical_domain(PHYSICAL_DEDICATED, available)?;

        // Construct and validate the installed executor before creating any
        // governed state. Installed executors are submission protocols, not
        // advisory metadata: inline/legacy submission is rejected.
        let cpu: Arc<dyn CpuExecutor> = match self.cpu_executor {
            Some(cpu) => cpu,
            None => default_cpu_executor(cpu_workers, shared_blocking_workers)?,
        };
        let descriptor = cpu.capabilities();
        if !descriptor.nonblocking_submit {
            return Err(GovernorError::InvalidTopology(
                TopologyError::ExecutorSubmissionMayBlock,
            ));
        }
        let domain = descriptor
            .physical_domain
            .ok_or(GovernorError::InvalidTopology(
                TopologyError::ExecutorPhysicalDomainUnknown,
            ))?;
        let expected_workers = match domain {
            PHYSICAL_SHARED_BLOCKING => shared_blocking_workers,
            PHYSICAL_CPU => cpu_workers,
            _ => {
                return Err(GovernorError::InvalidTopology(
                    TopologyError::UnknownExecutorPhysicalDomain { domain },
                ));
            }
        };
        let declared_workers =
            descriptor
                .declared_workers
                .ok_or(GovernorError::InvalidTopology(
                    TopologyError::ExecutorWorkerCountUnknown,
                ))?;
        if u64::from(declared_workers) != u64::try_from(expected_workers).unwrap_or(u64::MAX) {
            return Err(GovernorError::InvalidTopology(
                TopologyError::ExecutorWorkerCountMismatch {
                    declared: declared_workers,
                    resolved: expected_workers,
                    domain,
                },
            ));
        }

        let mut substrates = self.substrates;
        substrates.extend([
            physical_domain_record(PHYSICAL_SHARED_BLOCKING),
            physical_domain_record(PHYSICAL_CPU),
            physical_domain_record(PHYSICAL_DEDICATED),
        ]);

        // `PolicySet::new` already seeds the canonical built-in inventory; here we
        // only layer caller additions on top. The registry rejects duplicates
        // (including any attempt to shadow a built-in) and invalid records.
        let policy = PolicySet::new(self.resources.clone(), self.classes.clone())
            .with_substrates(substrates)?
            .with_capability_limits(capability_limits(
                &self.topology,
                cpu_workers,
                shared_blocking_workers,
                dedicated_workers,
                self.capability_limits,
            )?)?;

        // Keep the caller's portable topology declaration while recording the
        // resolved substrate inventory. Machine-specific capacity belongs to
        // the governor snapshot; duplicating it here would create a second
        // runtime-state authority.
        let config = RuntimeConfig {
            topology: self.topology.clone(),
            resources: self.resources,
            classes: self.classes,
            substrates: policy.substrates().values().cloned().collect(),
        };

        // `Governor::new` validates the policy fail-closed (impossible budgets,
        // mixed-tier fairness, invalid memory scaling, misused degrade/queue,
        // and the completeness of the substrate registry itself).
        let drain = Arc::new(DrainSignal::default());
        // `Arc::clone` fixes the expected type before unsizing. The method form
        // clones the concrete pointer and then coerces it to the driven port.
        #[allow(
            clippy::clone_on_ref_ptr,
            reason = "Arc::clone cannot unsize concrete Arc<T> to Arc<dyn _>"
        )]
        let settlement_waker: Arc<dyn SettlementWaker> = drain.clone();
        let governor = Arc::new(Governor::new_with_settlement_waker(
            policy,
            Arc::new(SystemClock),
            settlement_waker,
        )?);

        Ok(TokioRuntime::new(config, governor, cpu, drain))
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
    shared_blocking_workers: usize,
    dedicated_workers: usize,
    mut limits: BTreeMap<String, u32>,
) -> Result<BTreeMap<String, u32>, GovernorError> {
    if let Some(pool) = limits.keys().find(|pool| builder_owned_pool(pool.as_str())) {
        return Err(GovernorError::PolicyViolation(
            format!("capability limit for built-in pool {pool} cannot be overridden").into(),
        ));
    }
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
        if limits.insert(pool.to_owned(), slots).is_some() {
            return Err(GovernorError::PolicyViolation(
                format!("capability limit for built-in pool {pool} cannot be overridden").into(),
            ));
        }
        Ok(())
    };
    insert("cpu", cpu_workers.max(1))?;
    insert(PHYSICAL_CPU, cpu_workers.max(1))?;
    insert(PHYSICAL_SHARED_BLOCKING, shared_blocking_workers)?;
    insert(PHYSICAL_DEDICATED, dedicated_workers)?;
    for (pool, slots) in topology.declared_slots() {
        insert(pool, slots)?;
    }
    Ok(limits)
}

fn builder_owned_pool(pool: &str) -> bool {
    BUILTIN_SUBSTRATES.contains(&pool)
        || matches!(
            pool,
            PHYSICAL_CPU | PHYSICAL_SHARED_BLOCKING | PHYSICAL_DEDICATED
        )
}

fn physical_domain_record(domain: &'static str) -> SubstrateRecord {
    SubstrateRecord::new(
        format!("host-domain:{domain}"),
        SubstrateKind::CompetingExecution,
        Some(domain),
    )
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
// cargo-mutants scans cfg-disabled bodies. This adapter has mutually exclusive
// Rayon/Tokio implementations, so ordinary mutation would always report the
// inactive body as missed. The two feature lanes test the capability contract.
fn default_cpu_executor(
    cpu_workers: usize,
    _shared_blocking_workers: usize,
) -> Result<Arc<dyn CpuExecutor>, GovernorError> {
    let pool = taskmesh_rayon::RayonCpuExecutor::try_new(cpu_workers).map_err(|e| {
        GovernorError::PolicyViolation(format!("failed to build shared CPU pool: {e}").into())
    })?;
    Ok(Arc::new(pool))
}

#[cfg(not(feature = "rayon"))]
fn default_cpu_executor(
    _cpu_workers: usize,
    shared_blocking_workers: usize,
) -> Result<Arc<dyn CpuExecutor>, GovernorError> {
    let workers = u32::try_from(shared_blocking_workers).map_err(|_conversion_error| {
        GovernorError::InvalidTopology(TopologyError::SlotCountTooLarge {
            pool: PHYSICAL_SHARED_BLOCKING,
            slots: shared_blocking_workers,
            max: taskmesh_contract::MAX_CAPABILITY_SLOTS,
        })
    })?;
    let workers = std::num::NonZeroU32::new(workers)
        .expect("validated physical domain is finite and nonzero");
    Ok(Arc::new(BlockingPoolCpuExecutor::new(workers)))
}

#[cfg(test)]
mod mutation_semantics {
    use super::*;

    #[test]
    fn builtin_pool_names_are_stable_and_unknown_is_explicit() {
        for (input, expected) in [
            ("cpu", "cpu"),
            ("blocking", "blocking"),
            ("large_stack", "large_stack"),
            ("maintenance", "maintenance"),
            ("local_runtime", "local_runtime"),
            ("other", "unknown"),
        ] {
            assert_eq!(builtin_pool_name(input), expected);
        }
    }

    #[cfg(not(feature = "rayon"))]
    #[test]
    fn default_executor_declares_the_resolved_shared_domain() {
        let executor = default_cpu_executor(3, 4).expect("default executor");
        let capabilities = executor.capabilities();
        assert_eq!(capabilities.declared_workers, Some(4));
        assert_eq!(capabilities.physical_domain, Some(PHYSICAL_SHARED_BLOCKING));
        assert!(capabilities.nonblocking_submit);
        assert!(!capabilities.exclusive_pool);
    }

    #[cfg(feature = "rayon")]
    #[test]
    fn default_rayon_executor_declares_the_resolved_cpu_domain() {
        let executor = default_cpu_executor(3, 4).expect("default executor");
        let capabilities = executor.capabilities();
        assert_eq!(capabilities.declared_workers, Some(3));
        assert_eq!(capabilities.physical_domain, Some(PHYSICAL_CPU));
        assert!(capabilities.nonblocking_submit);
        assert!(capabilities.exclusive_pool);
    }
}
