//! `taskmesh-rayon` — a shared-CPU executor adapter for the taskmesh
//! [`CpuExecutor`] port (T09).
//!
//! This crate is a driven adapter on the outside of the hexagon. It implements
//! exactly one port and knows nothing about Tokio, the governor, or the host
//! facade — it only runs erased CPU work on a single shared `rayon::ThreadPool`.
//! The host bridges the result back to async via the channel captured inside the
//! work closure, so this stays runtime-agnostic.

use std::sync::Arc;
use std::thread::available_parallelism;

use rayon::{ThreadPool, ThreadPoolBuilder};
use taskmesh_contract::{CpuExecutor, ExecutorCapabilities, TopologyConfig};

/// A `CpuExecutor` backed by one shared Rayon thread pool.
#[derive(Clone)]
pub struct RayonCpuExecutor {
    pool: Arc<ThreadPool>,
    /// Whether this adapter built the pool for taskmesh alone. A pool handed in
    /// through [`RayonCpuExecutor::with_pool`] may have other users, and the
    /// adapter will not claim exclusivity it cannot see.
    exclusive: bool,
}

impl RayonCpuExecutor {
    /// Build a pool with an explicit worker count (clamped to at least 1).
    /// Returns an error instead of panicking if the OS cannot create the pool,
    /// so a host `Builder` can stay fail-closed.
    pub fn try_new(workers: usize) -> Result<Self, rayon::ThreadPoolBuildError> {
        let pool = ThreadPoolBuilder::new()
            .num_threads(workers.max(1))
            .thread_name(|i| format!("taskmesh-cpu-{i}"))
            .build()?;
        Ok(Self {
            pool: Arc::new(pool),
            exclusive: true,
        })
    }

    /// Build a pool with an explicit worker count, panicking on failure. Prefer
    /// [`RayonCpuExecutor::try_new`] on a fail-closed construction path.
    pub fn new(workers: usize) -> Self {
        Self::try_new(workers).expect("rayon pool must build")
    }

    /// Fallible topology-derived constructor (see [`RayonCpuExecutor::try_new`]).
    pub fn try_from_topology(
        topology: &TopologyConfig,
    ) -> Result<Self, rayon::ThreadPoolBuildError> {
        let available = available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        Self::try_new(topology.resolved_cpu_workers(available))
    }

    /// Build a pool whose worker count is derived from the runtime topology:
    /// `available_parallelism() - reserve_cores`, clamped to
    /// `[min_workers, max_workers]` (T09). Panics on pool-build failure; prefer
    /// [`RayonCpuExecutor::try_from_topology`] on a fail-closed path.
    pub fn from_topology(topology: &TopologyConfig) -> Self {
        Self::try_from_topology(topology).expect("rayon pool must build")
    }

    /// Wrap an already-built shared pool. The adapter declares the pool as
    /// *shared* (D05): it cannot know who else submits to it, so the host will
    /// govern only taskmesh's own submissions and say so.
    pub fn with_pool(pool: Arc<ThreadPool>) -> Self {
        Self {
            pool,
            exclusive: false,
        }
    }

    /// The number of worker threads in the shared pool.
    pub fn worker_count(&self) -> usize {
        self.pool.current_num_threads()
    }
}

impl CpuExecutor for RayonCpuExecutor {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        self.pool.spawn(work);
    }

    /// The declaration the host checks its `cpu` gate against (D05):
    /// `ThreadPool::spawn` never runs the job inline, and the worker count is
    /// the pool's real thread count — read from the pool, not re-derived from
    /// the machine, so the gate and the executor cannot be sized from two
    /// different answers.
    fn capabilities(&self) -> ExecutorCapabilities {
        let workers = u32::try_from(self.worker_count()).unwrap_or(u32::MAX);
        ExecutorCapabilities::legacy()
            .nonblocking_submit(true)
            .declared_workers(workers)
            .exclusive_pool(self.exclusive)
    }
}
