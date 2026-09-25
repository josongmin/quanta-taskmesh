//! Driven ports: the seams adapters plug into. All are object-safe so the engine
//! and host can hold them behind `Arc<dyn _>` without committing to a runtime.

use std::time::{SystemTime, UNIX_EPOCH};

/// A wall clock in milliseconds. The engine uses it for permit lease timestamps
/// and leak-sweep staleness; tests inject a controllable clock.
///
/// # Lock discipline (D07)
///
/// `now_ms` is **never** called while the engine holds its state mutex. A driven
/// port is host code: it may block, re-enter the governor, or panic, and none of
/// those may happen under the transition lock. The engine therefore samples the
/// clock *before* taking the lock.
///
/// # Monotonic commit time (D07)
///
/// Because the sample happens before the lock, an arbitrarily long pause can sit
/// between "read the clock" and "commit the state change". The engine closes
/// that gap with a **commit watermark**: the time recorded on a committed
/// transition is `max(sample, watermark)`, and the watermark then advances. So a
/// delayed sample can never stamp a lease as older than one already committed,
/// and a late heartbeat can never move a lease's activity backwards.
///
/// The clamped value is a *logical* commit time. It is monotonic by
/// construction; it is not a measurement of elapsed wall time across the lock
/// wait, and this trait does not promise that `now_ms` itself is monotonic.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

/// Default `Clock` backed by the system wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        u64::try_from(millis).unwrap_or(u64::MAX)
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

    /// What this adapter guarantees about submission and pool ownership.
    ///
    /// The default is [`ExecutorCapabilities::legacy`] so old adapters remain
    /// source-compatible. The host rejects that profile at installation: an
    /// executor must explicitly declare nonblocking submission, its finite
    /// worker count, and its registered physical domain. Once accepted, the
    /// host freezes this value and does not query the adapter during dispatch.
    fn capabilities(&self) -> ExecutorCapabilities {
        ExecutorCapabilities::legacy()
    }
}

/// The declared guarantees of a [`CpuExecutor`] adapter.
///
/// These are *declarations*, not measurements. The host reads and validates the
/// declaration once at build time, then freezes that accepted value for runtime
/// planning and observability. It rejects incomplete or inconsistent adapters.
/// `exclusive_pool=false` remains observable: taskmesh bounds its own atomic
/// submissions to the shared physical domain but does not claim authority over
/// ambient users.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExecutorCapabilities {
    /// `spawn` returns without running the work to completion on the caller
    /// thread. When `false`, the host must assume `spawn` may execute the job
    /// inline and therefore judges run deadlines by the worker's own start and
    /// completion timestamps rather than by when `spawn` returned. The host
    /// rejects `false` at installation.
    pub nonblocking_submit: bool,
    /// Finite Taskmesh submission capacity declared for this physical domain.
    /// For an exclusive pool this is its worker count. For a shared pool this is
    /// Taskmesh's own admission bound, not a claim about ambient pool users.
    /// `None` means unknown and is never upgraded to a capacity guarantee.
    pub declared_workers: Option<u32>,
    /// The adapter's pool runs taskmesh work only. When `false`, ambient users
    /// share it and taskmesh governs only its own submissions.
    pub exclusive_pool: bool,
    /// Registered physical-domain capability this executor occupies. The host
    /// resolves and freezes this once at build time; submission and promotion
    /// never query the adapter again.
    pub physical_domain: Option<&'static str>,
}

impl ExecutorCapabilities {
    /// The conservative profile assumed for an adapter that declares nothing:
    /// submission may block, worker count unknown, pool shared with ambient work.
    pub const fn legacy() -> Self {
        Self {
            nonblocking_submit: false,
            declared_workers: None,
            exclusive_pool: false,
            physical_domain: None,
        }
    }

    pub const fn nonblocking_submit(mut self, value: bool) -> Self {
        self.nonblocking_submit = value;
        self
    }

    pub const fn declared_workers(mut self, value: u32) -> Self {
        self.declared_workers = Some(value);
        self
    }

    pub const fn exclusive_pool(mut self, value: bool) -> Self {
        self.exclusive_pool = value;
        self
    }

    pub const fn physical_domain(mut self, value: &'static str) -> Self {
        self.physical_domain = Some(value);
        self
    }
}

/// Notifies a parked admission waiter that its queued request was promoted.
///
/// The host backs this with a runtime primitive (e.g. `tokio::sync::Notify`);
/// the engine stays runtime-agnostic and only calls `wake()` on promotion.
pub trait PermitWaker: Send + Sync {
    fn wake(&self);
}

/// Notifies a host that an engine transition may have reduced outstanding
/// custody.
///
/// The engine invokes this port only after releasing its state mutex. Hosts
/// must re-read the authoritative snapshot rather than treating a wake as
/// proof that the system is idle.
pub trait SettlementWaker: Send + Sync {
    fn wake(&self);
}
