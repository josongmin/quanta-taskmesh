//! `taskmesh-engine` — the governance core of taskmesh.
//!
//! This crate is the inside of the hexagon: a pure, runtime-agnostic governed
//! execution state machine. It depends only on `taskmesh-contract` (the port
//! surface) and `parking_lot` (state mutex). It contains no Tokio primitives, no
//! I/O, and no product-local taxonomy.
//!
//! ## Layout
//!
//! - `shared` — cross-slice kernel: ids, [`PolicySet`], value types.
//! - `engine` — composition root: the [`Governor`] and its `GovernedState`.
//! - `features` — vertical slices, each with pure `domain` rules and a stateful
//!   `service`:
//!   - `admission` (T03) — bounded, fail-closed intake.
//!   - `fairness` (T04) — cross-class dispatch + retry-after.
//!   - `memory` (T05) — permit sizing, overcommit, leak sweep.
//!   - `composite` (T06) — root attribution, recursion guard, reduce validation.
//!   - `inventory` (T08) — substrate registry SSOT.

mod engine;
mod features;
mod shared;

pub use engine::Governor;
pub use shared::{
    AdmissionDecision, LeakSweepReport, PermitId, PolicySet, Provenance, ResolvedCost,
    RootAttribution, Seq, Ticket,
};

/// Default staleness window used by [`Governor::reap_leaks`].
pub use features::memory::DEFAULT_LEAK_STALE_MS;

/// The fixed canonical built-in substrate set (T08).
pub use features::inventory::{builtin_records, BUILTIN_SUBSTRATES};

// Re-export the driven clock ports for convenience at the engine boundary.
pub use taskmesh_contract::{Clock, ManualClock, SystemClock};
