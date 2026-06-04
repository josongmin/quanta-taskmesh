//! Driven-port adapters that bind engine ports to Tokio primitives.

pub mod permit_waker;

pub use permit_waker::TokioPermitWaker;
