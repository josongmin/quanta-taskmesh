//! Worker topology and substrate (capability-pool) vocabulary.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

/// Governance disposition of a substrate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SubstrateKind {
    AuthorityOnly,
    CompetingExecution,
    MaintenanceOnly,
    RetireOrMigrate,
}

/// Where a stage wants to run. Product-neutral; the adapter layer owns mapping
/// these to concrete pools.
///
/// Governance note: in the current Tokio host, `BlockingPool`,
/// `LargeStackCapability`, and `BackgroundOnly` all *execute* on Tokio's blocking
/// pool — they are distinguished by **separate capability pools** (independent
/// topology-sized concurrency gates), not by distinct executors. A dedicated
/// large-stack/background thread pool is a future host concern; the contract
/// already names the capability so callers and governance can treat them
/// separately today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SubstrateHint {
    AsyncIo,
    BlockingPool,
    SharedCpuExecutor,
    LargeStackCapability,
    LocalRuntime,
    BackgroundOnly,
}

/// A registered substrate. `capability_pool` is required for every kind except
/// `AuthorityOnly`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubstrateRecord {
    pub name: Cow<'static, str>,
    pub kind: SubstrateKind,
    pub capability_pool: Option<Cow<'static, str>>,
}

impl SubstrateRecord {
    pub fn new(
        name: impl Into<Cow<'static, str>>,
        kind: SubstrateKind,
        capability_pool: Option<impl Into<Cow<'static, str>>>,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            capability_pool: capability_pool.map(Into::into),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CpuMode {
    Auto,
    Fixed(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuPoolConfig {
    pub mode: CpuMode,
    pub reserve_cores: usize,
    pub min_workers: usize,
    pub max_workers: usize,
}

impl Default for CpuPoolConfig {
    fn default() -> Self {
        Self {
            mode: CpuMode::Auto,
            reserve_cores: 0,
            min_workers: 1,
            max_workers: usize::MAX,
        }
    }
}

/// `cpu` defaults via [`CpuPoolConfig::default`]; all slot counts default to 0.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyConfig {
    pub cpu: CpuPoolConfig,
    pub blocking_threads: usize,
    pub large_stack_slots: usize,
    pub maintenance_workers: usize,
    pub local_runtime_slots: usize,
}

impl TopologyConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cpu_auto(mut self) -> Self {
        self.cpu.mode = CpuMode::Auto;
        self
    }

    pub fn cpu_fixed(mut self, value: usize) -> Self {
        self.cpu.mode = CpuMode::Fixed(value);
        self
    }

    pub fn reserve_cores(mut self, value: usize) -> Self {
        self.cpu.reserve_cores = value;
        self
    }

    pub fn min_workers(mut self, value: usize) -> Self {
        self.cpu.min_workers = value;
        self
    }

    pub fn max_workers(mut self, value: usize) -> Self {
        self.cpu.max_workers = value;
        self
    }

    pub fn blocking_threads(mut self, value: usize) -> Self {
        self.blocking_threads = value;
        self
    }

    pub fn large_stack_slots(mut self, value: usize) -> Self {
        self.large_stack_slots = value;
        self
    }

    pub fn maintenance_workers(mut self, value: usize) -> Self {
        self.maintenance_workers = value;
        self
    }

    pub fn local_runtime_slots(mut self, value: usize) -> Self {
        self.local_runtime_slots = value;
        self
    }

    /// Resolve the worker count for the shared CPU pool, honoring reserve cores
    /// and the `[min_workers, max_workers]` clamp. `available` is the detected
    /// parallelism (callers pass `available_parallelism()`).
    pub fn resolved_cpu_workers(&self, available: usize) -> usize {
        let base = match self.cpu.mode {
            CpuMode::Fixed(n) => n,
            CpuMode::Auto => available.saturating_sub(self.cpu.reserve_cores),
        };
        base.clamp(self.cpu.min_workers.max(1), self.cpu.max_workers.max(1))
    }
}
