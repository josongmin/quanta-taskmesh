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
use taskmesh_contract::{CpuExecutor, TopologyConfig};

/// A `CpuExecutor` backed by one shared Rayon thread pool.
#[derive(Clone)]
pub struct RayonCpuExecutor {
    pool: Arc<ThreadPool>,
}

impl RayonCpuExecutor {
    /// Build a pool with an explicit worker count (clamped to at least 1).
    pub fn new(workers: usize) -> Self {
        let pool = ThreadPoolBuilder::new()
            .num_threads(workers.max(1))
            .thread_name(|i| format!("taskmesh-cpu-{i}"))
            .build()
            .expect("rayon pool must build");
        Self {
            pool: Arc::new(pool),
        }
    }

    /// Build a pool whose worker count is derived from the runtime topology:
    /// `available_parallelism() - reserve_cores`, clamped to
    /// `[min_workers, max_workers]` (T09).
    pub fn from_topology(topology: &TopologyConfig) -> Self {
        let available = available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        Self::new(topology.resolved_cpu_workers(available))
    }

    /// Wrap an already-built shared pool.
    pub fn with_pool(pool: Arc<ThreadPool>) -> Self {
        Self { pool }
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
}
