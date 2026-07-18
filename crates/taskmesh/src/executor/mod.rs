//! Execution adapters: the CPU executor port's default impl plus the
//! cancel/deadline submission options.

pub mod cancel;
pub mod tokio_exec;

pub use cancel::{SubmissionDeadline, SubmitOptions};
pub use tokio_exec::BlockingPoolCpuExecutor;
