//! `taskmesh-engine` — the governance core of taskmesh.
//!
//! This crate is the inside of the hexagon: a pure, runtime-agnostic governed
//! execution state machine. It depends only on `taskmesh-contract` (the port
//! surface) and `parking_lot` (state mutex). It contains no Tokio primitives, no
//! I/O, and no product-local taxonomy.
//!
//! Under `--cfg loom` / `--cfg shuttle` the mutex and atomics come from the
//! model checker instead (see `sync`), so the concurrency proofs in `tests/`
//! run against this crate's real transitions.
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
mod sync;

pub use engine::governor::{PendingView, PermitLedgerView, PROMOTION_BUDGET};
pub use engine::state::{
    CapacityBlock, ClaimOutcome, ReleaseOutcome, TerminalReason, MAX_TERMINAL_TICKETS,
};
pub use engine::Governor;
#[cfg(feature = "test-util")]
pub use shared::{request_key_derive_count, reset_request_key_derive_count};
pub use shared::{
    AdmissionDecision, CapabilityName, LeakSweepReport, PermitId, PolicySet, Provenance,
    RequestKey, ResolvedCost, RootAttribution, Seq, StageReleaseOutcome, Ticket,
};

/// Outcome of a memory reconcile (T05).
pub use features::memory::ReconcileOutcome;

/// Default staleness window used by [`Governor::reap_leaks`].
pub use features::memory::DEFAULT_LEAK_STALE_MS;

/// The fixed canonical built-in substrate set (T08).
pub use features::inventory::{builtin_records, BUILTIN_SUBSTRATES};

// Re-export the driven clock ports for convenience at the engine boundary.
pub use taskmesh_contract::{Clock, ExecutionPhase, ManualClock, SystemClock};
