//! The driving port: the public surface a governed runtime exposes to callers.

use std::future::Future;

use crate::snapshot::Snapshot;
use crate::task::TaskSpec;
use crate::verdict::RunError;

/// A governed execution runtime. Governor rejection and task failure are kept
/// distinct at the type level via [`RunError`].
///
/// `async fn` in trait is used deliberately; the host impl is the only intended
/// implementor of the full surface, and callers consume it through the facade.
#[allow(async_fn_in_trait)]
pub trait Runtime {
    /// Run an async, `Send` future on the async substrate.
    async fn run_io<T, E, Fut>(&self, spec: TaskSpec, fut: Fut) -> Result<T, RunError<E>>
    where
        Fut: Future<Output = Result<T, E>> + Send;

    /// Run a blocking closure on the blocking pool.
    async fn run_blocking<T, E, F>(&self, spec: TaskSpec, job: F) -> Result<T, RunError<E>>
    where
        F: FnOnce() -> Result<T, E> + Send + 'static,
        E: Send + 'static,
        T: Send + 'static;

    /// Run a CPU-bound closure through the pluggable CPU executor.
    async fn run_cpu<T, E, F>(&self, spec: TaskSpec, job: F) -> Result<T, RunError<E>>
    where
        F: FnOnce() -> Result<T, E> + Send + 'static,
        E: Send + 'static,
        T: Send + 'static;

    /// Run a non-`Send` future on the local-runtime substrate.
    async fn run_local<T, E, Fut>(&self, spec: TaskSpec, fut: Fut) -> Result<T, RunError<E>>
    where
        Fut: Future<Output = Result<T, E>> + 'static,
        T: 'static;

    /// A consistent view of governed state.
    fn snapshot(&self) -> Snapshot;
}
