//! Driven ports: the seams adapters plug into. All are object-safe so the engine
//! and host can hold them behind `Arc<dyn _>` without committing to a runtime.

use std::time::{SystemTime, UNIX_EPOCH};

/// A monotonic-ish wall clock in milliseconds. The engine uses it for permit
/// lease timestamps and leak-sweep staleness; tests inject a controllable clock.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

/// Default `Clock` backed by the system wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

/// A fixed clock for deterministic tests.
#[derive(Debug, Default)]
pub struct ManualClock {
    now_ms: std::sync::atomic::AtomicU64,
}

impl ManualClock {
    pub fn new(now_ms: u64) -> Self {
        Self {
            now_ms: std::sync::atomic::AtomicU64::new(now_ms),
        }
    }

    pub fn set(&self, now_ms: u64) {
        self.now_ms
            .store(now_ms, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn advance(&self, by_ms: u64) {
        self.now_ms
            .fetch_add(by_ms, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.now_ms.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// The pluggable CPU execution substrate (driven port for `run_cpu`).
///
/// Object-safe by construction: it accepts an already-erased unit of work and is
/// responsible only for *running* it on a CPU-bound pool. Result delivery is the
/// caller's concern (typically a channel captured inside `work`). This lets both
/// the Tokio blocking-pool default and the Rayon adapter satisfy one `Arc<dyn _>`.
pub trait CpuExecutor: Send + Sync {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>);
}

/// Notifies a parked admission waiter that its queued request was promoted.
/// The host backs this with a runtime primitive (e.g. `tokio::sync::Notify`);
/// the engine stays runtime-agnostic and only calls `wake()` on promotion.
pub trait PermitWaker: Send + Sync {
    fn wake(&self);
}
