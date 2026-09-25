//! The default `CpuExecutor`: the Tokio blocking pool. This is the residual
//! `spawn_blocking` path, now living *behind* the executor port rather than
//! hardcoded into `run_cpu` (T09). Swapping in `taskmesh-rayon` replaces only
//! this object.

use taskmesh_contract::{CpuExecutor, ExecutorCapabilities, PHYSICAL_SHARED_BLOCKING};

/// Runs CPU work on Tokio's blocking pool. Always available without extra crates.
#[derive(Debug, Clone, Copy)]
pub struct BlockingPoolCpuExecutor {
    physical_workers: u32,
}

impl BlockingPoolCpuExecutor {
    pub fn new(physical_workers: std::num::NonZeroU32) -> Self {
        Self {
            physical_workers: physical_workers.get(),
        }
    }
}

impl CpuExecutor for BlockingPoolCpuExecutor {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        // The work closure owns its own result channel; we discard the handle.
        tokio::task::spawn_blocking(work);
    }

    /// What this adapter can honestly say (D05): `spawn_blocking` returns
    /// without running the job inline; the pool is Tokio's, shared with every
    /// other `spawn_blocking` user on the runtime. The declared worker count is
    /// the builder-resolved Taskmesh bound on submissions to that shared domain;
    /// it does not claim that Tokio's ambient pool has the same size.
    fn capabilities(&self) -> ExecutorCapabilities {
        ExecutorCapabilities::legacy()
            .nonblocking_submit(true)
            .declared_workers(self.physical_workers)
            .exclusive_pool(false)
            .physical_domain(PHYSICAL_SHARED_BLOCKING)
            .requires_tokio_context(true)
    }
}
