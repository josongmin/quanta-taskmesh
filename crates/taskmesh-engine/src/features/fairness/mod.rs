//! Fairness slice (T04): cross-class dispatch disciplines and retry-after hints.

pub mod retry_after;
pub mod scheduler;

pub use retry_after::compute as retry_after;
pub use scheduler::{enqueue_tags, select};
