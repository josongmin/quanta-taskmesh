//! The default `CpuExecutor`: the Tokio blocking pool. This is the residual
//! `spawn_blocking` path, now living *behind* the executor port rather than
//! hardcoded into `run_cpu` (T09). Swapping in `taskmesh-rayon` replaces only
//! this object.

use taskmesh_contract::CpuExecutor;

/// Runs CPU work on Tokio's blocking pool. Always available without extra crates.
#[derive(Debug, Default, Clone, Copy)]
pub struct BlockingPoolCpuExecutor;

impl CpuExecutor for BlockingPoolCpuExecutor {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        // The work closure owns its own result channel; we discard the handle.
        tokio::task::spawn_blocking(work);
    }
}
