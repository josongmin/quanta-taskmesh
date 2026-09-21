//! Worker topology and substrate (capability-pool) vocabulary.

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize};

/// The largest capability-pool slot count taskmesh governs.
///
/// Capability occupancy is accounted in `u32` by the governance engine, so a
/// declared slot count above this cannot be represented and is rejected at
/// construction instead of being silently clamped. On 64-bit hosts this is also
/// comfortably below `tokio::sync::Semaphore::MAX_PERMITS`, so a topology that
/// validates here can never trip a host semaphore construction panic either.
#[allow(
    clippy::as_conversions,
    reason = "u32::MAX -> usize is exact on every supported target; const context has no fallible alternative"
)]
// `u32::MAX` always fits `usize` on every target taskmesh supports (32-bit and
// wider), so this widening is exact.
pub const MAX_CAPABILITY_SLOTS: usize = u32::MAX as usize;

/// Why a [`TopologyConfig`] is not constructible.
///
/// Topology is public, deserializable input, so every impossible shape is a
/// typed rejection at validation time — never a panic inside a resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TopologyError {
    /// `cpu.min_workers > cpu.max_workers`: the clamp window is empty.
    InvertedCpuWorkerBounds {
        min_workers: usize,
        max_workers: usize,
    },
    /// A declared slot count exceeds [`MAX_CAPABILITY_SLOTS`].
    SlotCountTooLarge {
        pool: &'static str,
        slots: usize,
        max: usize,
    },
    /// `CpuMode::Fixed(0)` requests a pool that can never run anything.
    ZeroFixedCpuWorkers,
    /// The supplied [`crate::CpuExecutor`] declares fewer workers than the
    /// resolved CPU topology, so the `cpu` gate would admit more concurrent
    /// work than the executor can run. The two must be sized from one answer.
    ExecutorDeclaresFewerWorkers { declared: u32, resolved: usize },
    /// An installable executor may execute `spawn` inline and therefore create
    /// an unbounded competing path on the runtime worker that submitted it.
    ExecutorSubmissionMayBlock,
    /// The executor did not declare the finite physical domain it occupies.
    ExecutorPhysicalDomainUnknown,
    /// The executor did not declare the finite worker count of its domain.
    ExecutorWorkerCountUnknown,
    /// The executor named a domain the host does not own.
    UnknownExecutorPhysicalDomain { domain: &'static str },
    /// A fixed physical worker domain cannot contain zero workers.
    ZeroFixedPhysicalDomain { domain: &'static str },
    /// The executor's declared workers and its configured physical domain must
    /// be the same number; wider and narrower declarations are both dishonest.
    ExecutorWorkerCountMismatch {
        declared: u32,
        resolved: usize,
        domain: &'static str,
    },
}

impl fmt::Display for TopologyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvertedCpuWorkerBounds {
                min_workers,
                max_workers,
            } => write!(
                f,
                "topology cpu.min_workers ({min_workers}) exceeds cpu.max_workers ({max_workers})"
            ),
            Self::SlotCountTooLarge { pool, slots, max } => write!(
                f,
                "topology pool {pool} declares {slots} slots, above the governed maximum {max}"
            ),
            Self::ExecutorDeclaresFewerWorkers { declared, resolved } => write!(
                f,
                "cpu executor declares {declared} worker(s) but the topology resolved {resolved}; the cpu gate cannot exceed the executor"
            ),
            Self::ZeroFixedCpuWorkers => {
                f.write_str("topology cpu.mode = Fixed(0) cannot execute any work")
            }
            Self::ExecutorSubmissionMayBlock => {
                f.write_str("cpu executor submission must be nonblocking")
            }
            Self::ExecutorPhysicalDomainUnknown => {
                f.write_str("cpu executor must declare its physical domain")
            }
            Self::ExecutorWorkerCountUnknown => {
                f.write_str("cpu executor must declare its physical worker count")
            }
            Self::UnknownExecutorPhysicalDomain { domain } => {
                write!(f, "cpu executor declares unknown physical domain {domain}")
            }
            Self::ZeroFixedPhysicalDomain { domain } => {
                write!(f, "physical domain {domain} cannot resolve to zero workers")
            }
            Self::ExecutorWorkerCountMismatch {
                declared,
                resolved,
                domain,
            } => write!(
                f,
                "cpu executor declares {declared} worker(s) for physical domain {domain}, but the domain resolved {resolved}"
            ),
        }
    }
}

impl std::error::Error for TopologyError {}

/// Governance disposition of a substrate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SubstrateKind {
    AuthorityOnly,
    CompetingExecution,
    MaintenanceOnly,
    RetireOrMigrate,
}

/// Where a stage wants to run. Product-neutral; the adapter layer owns mapping
/// these to concrete pools.
///
/// Governance note: in the current Tokio host, `BlockingPool`,
/// `LargeStackCapability`, and `BackgroundOnly` are all governed as blocking-family
/// substrates. `BlockingPool` and `BackgroundOnly` execute on Tokio's blocking
/// pool. `LargeStackCapability` can either use that pool or, when the submitted
/// [`crate::TaskSpec`] carries an explicit stack-size request, execute on a
/// host-managed dedicated thread for that one task. They are distinguished by
/// **separate capability pools** (independent topology-sized concurrency gates),
/// not by a permanently separate executor fleet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SubstrateHint {
    AsyncIo,
    BlockingPool,
    SharedCpuExecutor,
    LargeStackCapability,
    LocalRuntime,
    BackgroundOnly,
}

impl SubstrateHint {
    /// The built-in capability pool this hint dispatches to, or `None` for an
    /// ungated substrate (`AsyncIo` — async concurrency is unbounded by design).
    ///
    /// This is the authoritative hint→pool mapping: the host derives its worker
    /// gates from the registered inventory by this name, rather than hardcoding
    /// the relationship. The names match [`crate::SubstrateRecord::capability_pool`]
    /// of the built-in set.
    pub fn capability_pool(self) -> Option<&'static str> {
        match self {
            Self::AsyncIo => None,
            Self::BlockingPool => Some("blocking"),
            Self::SharedCpuExecutor => Some("cpu"),
            Self::LargeStackCapability => Some("large_stack"),
            Self::LocalRuntime => Some("local_runtime"),
            Self::BackgroundOnly => Some("maintenance"),
        }
    }
}

/// A registered substrate. `capability_pool` is required for every kind except
/// `AuthorityOnly`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubstrateRecord {
    pub name: Cow<'static, str>,
    pub kind: SubstrateKind,
    pub capability_pool: Option<Cow<'static, str>>,
}

impl SubstrateRecord {
    pub fn new(
        name: impl Into<Cow<'static, str>>,
        kind: SubstrateKind,
        capability_pool: Option<impl Into<Cow<'static, str>>>,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            capability_pool: capability_pool.map(Into::into),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CpuMode {
    Auto,
    Fixed(usize),
}

/// Capacity source for a host-owned physical worker domain.
///
/// This is versioned separately from the legacy role-slot fields below. Their
/// published `0 = explicitly unbounded` meaning remains unchanged; physical
/// worker domains are always finite and nonzero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhysicalDomainMode {
    Auto,
    Fixed(usize),
}

/// Stable, product-neutral identifiers for the worker domains of the Tokio host.
///
/// These are capability-pool names too: one engine transition atomically
/// charges a physical domain and any semantic role pool.
pub const PHYSICAL_SHARED_BLOCKING: &str = "physical.shared_blocking";
pub const PHYSICAL_CPU: &str = "physical.cpu";
pub const PHYSICAL_DEDICATED: &str = "physical.dedicated";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalDomainTopology {
    pub shared_blocking: PhysicalDomainMode,
    pub dedicated: PhysicalDomainMode,
}

impl Default for PhysicalDomainTopology {
    fn default() -> Self {
        Self {
            shared_blocking: PhysicalDomainMode::Auto,
            dedicated: PhysicalDomainMode::Auto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuPoolConfig {
    pub mode: CpuMode,
    pub reserve_cores: usize,
    pub min_workers: usize,
    pub max_workers: usize,
}

impl Default for CpuPoolConfig {
    fn default() -> Self {
        Self {
            mode: CpuMode::Auto,
            reserve_cores: 0,
            min_workers: 1,
            max_workers: usize::MAX,
        }
    }
}

/// `cpu` defaults via [`CpuPoolConfig::default`]; all slot counts default to 0.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyConfig {
    pub cpu: CpuPoolConfig,
    pub blocking_threads: usize,
    pub large_stack_slots: usize,
    pub maintenance_workers: usize,
    pub local_runtime_slots: usize,
    /// Finite physical worker ownership. Defaults are machine-derived once by
    /// the host builder; they are never represented by the legacy zero sentinel.
    #[serde(default)]
    pub physical_domains: PhysicalDomainTopology,
}

impl TopologyConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cpu_auto(mut self) -> Self {
        self.cpu.mode = CpuMode::Auto;
        self
    }

    pub fn cpu_fixed(mut self, value: usize) -> Self {
        self.cpu.mode = CpuMode::Fixed(value);
        self
    }

    pub fn reserve_cores(mut self, value: usize) -> Self {
        self.cpu.reserve_cores = value;
        self
    }

    pub fn min_workers(mut self, value: usize) -> Self {
        self.cpu.min_workers = value;
        self
    }

    pub fn max_workers(mut self, value: usize) -> Self {
        self.cpu.max_workers = value;
        self
    }

    pub fn blocking_threads(mut self, value: usize) -> Self {
        self.blocking_threads = value;
        self
    }

    pub fn large_stack_slots(mut self, value: usize) -> Self {
        self.large_stack_slots = value;
        self
    }

    pub fn maintenance_workers(mut self, value: usize) -> Self {
        self.maintenance_workers = value;
        self
    }

    pub fn local_runtime_slots(mut self, value: usize) -> Self {
        self.local_runtime_slots = value;
        self
    }

    pub fn shared_blocking_domain(mut self, mode: PhysicalDomainMode) -> Self {
        self.physical_domains.shared_blocking = mode;
        self
    }

    pub fn dedicated_domain(mut self, mode: PhysicalDomainMode) -> Self {
        self.physical_domains.dedicated = mode;
        self
    }

    /// Fail-closed structural validation. Every constructor that turns a topology
    /// into real worker capacity calls this **before** resolving anything, so an
    /// impossible topology is a typed `Err`, never a panic in a clamp.
    pub fn validate(&self) -> Result<(), TopologyError> {
        let min = self.cpu.min_workers.max(1);
        let max = self.cpu.max_workers.max(1);
        if min > max {
            return Err(TopologyError::InvertedCpuWorkerBounds {
                min_workers: self.cpu.min_workers,
                max_workers: self.cpu.max_workers,
            });
        }
        if self.cpu.mode == CpuMode::Fixed(0) {
            return Err(TopologyError::ZeroFixedCpuWorkers);
        }
        for (domain, mode) in [
            (
                PHYSICAL_SHARED_BLOCKING,
                self.physical_domains.shared_blocking,
            ),
            (PHYSICAL_DEDICATED, self.physical_domains.dedicated),
        ] {
            if let PhysicalDomainMode::Fixed(workers) = mode {
                if workers == 0 {
                    return Err(TopologyError::ZeroFixedPhysicalDomain { domain });
                }
                if workers > MAX_CAPABILITY_SLOTS {
                    return Err(TopologyError::SlotCountTooLarge {
                        pool: domain,
                        slots: workers,
                        max: MAX_CAPABILITY_SLOTS,
                    });
                }
            }
        }
        for (pool, slots) in self.declared_slots() {
            if slots > MAX_CAPABILITY_SLOTS {
                return Err(TopologyError::SlotCountTooLarge {
                    pool,
                    slots,
                    max: MAX_CAPABILITY_SLOTS,
                });
            }
        }
        Ok(())
    }

    /// Resolve one physical domain from the same machine answer used for every
    /// other Auto capacity. Validation makes the result finite and nonzero.
    pub fn try_resolved_physical_domain(
        &self,
        domain: &'static str,
        available: usize,
    ) -> Result<usize, TopologyError> {
        self.validate()?;
        if domain == PHYSICAL_CPU {
            return self.try_resolved_cpu_workers(available);
        }
        let mode = match domain {
            PHYSICAL_SHARED_BLOCKING => self.physical_domains.shared_blocking,
            PHYSICAL_DEDICATED => self.physical_domains.dedicated,
            _ => return Err(TopologyError::UnknownExecutorPhysicalDomain { domain }),
        };
        Ok(match mode {
            PhysicalDomainMode::Auto => available.max(1),
            PhysicalDomainMode::Fixed(workers) => workers,
        })
    }

    /// The explicitly-declared slot counts, by capability-pool name. The `cpu`
    /// pool is resolved separately (it depends on detected parallelism).
    ///
    /// `0` keeps its published meaning: *no limit for this pool*.
    pub fn declared_slots(&self) -> [(&'static str, usize); 4] {
        [
            ("blocking", self.blocking_threads),
            ("large_stack", self.large_stack_slots),
            ("maintenance", self.maintenance_workers),
            ("local_runtime", self.local_runtime_slots),
        ]
    }

    /// Resolve the worker count for the shared CPU pool, honoring reserve cores
    /// and the `[min_workers, max_workers]` clamp. `available` is the detected
    /// parallelism (callers pass `available_parallelism()` **once** and share the
    /// result, so every gate and executor agrees on one number).
    ///
    /// Returns the validation error rather than clamping an impossible window.
    pub fn try_resolved_cpu_workers(&self, available: usize) -> Result<usize, TopologyError> {
        self.validate()?;
        let workers = self.resolve_cpu_workers_unchecked(available);
        if workers > MAX_CAPABILITY_SLOTS {
            return Err(TopologyError::SlotCountTooLarge {
                pool: "cpu",
                slots: workers,
                max: MAX_CAPABILITY_SLOTS,
            });
        }
        Ok(workers)
    }

    /// Resolve the worker count for the shared CPU pool.
    ///
    /// Total by construction: an inverted `[min, max]` window resolves to the
    /// minimum rather than panicking. Prefer [`TopologyConfig::try_resolved_cpu_workers`]
    /// on any construction path — it reports the inverted window instead of
    /// silently picking a value for it.
    pub fn resolved_cpu_workers(&self, available: usize) -> usize {
        self.resolve_cpu_workers_unchecked(available)
    }

    fn resolve_cpu_workers_unchecked(&self, available: usize) -> usize {
        let base = match self.cpu.mode {
            CpuMode::Fixed(n) => n,
            CpuMode::Auto => available.saturating_sub(self.cpu.reserve_cores),
        };
        let lo = self.cpu.min_workers.max(1);
        let hi = self.cpu.max_workers.max(1);
        // `clamp` panics when lo > hi; ordering the two operations explicitly
        // keeps this total for every input, including an inverted window.
        base.max(lo).min(hi.max(lo))
    }
}
