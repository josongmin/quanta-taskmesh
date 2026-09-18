//! Graceful drain of the Tokio host (ADR 0003 D17).
//!
//! [`TokioRuntime::drain`] is one-way: it flips the runtime into a *draining*
//! state in which every submission is refused before admission, then waits
//! for the work the engine already holds to return its custody. It tears
//! nothing down — dropping the handle remains the only teardown — and a drain
//! that times out leaves the runtime draining, so a later call continues the
//! same wait instead of starting a new life.
//!
//! # Custody, not replies (D10)
//!
//! A caller's answer is not the work's end: a timed-out or cancelled caller
//! has its result while the worker still holds the execution lease. The drain
//! therefore never counts replies. It reads the engine's own `inflight` and
//! `queued` gauges — the same numbers [`Snapshot`] reports — and returns only
//! when every class shows both at zero.
//!
//! # Closed in the engine, not at the door
//!
//! The refusal is the engine's ([`Governor::close_admission`]), decided under
//! the same lock that admits. There is no host-side flag racing the engine's
//! decision: a submission is either admitted (and in the gauges) or refused,
//! and a snapshot taken after the close already contains every admission that
//! will ever happen. An embedder driving `governor()` directly is refused the
//! same way.
//!
//! # Event-driven wait
//!
//! Every host-owned return of custody pings a [`Notify`] the drain waits on: an
//! [`ExecutionLease`](super::ExecutionLease) dropping, a queued ticket abandoned,
//! and a promotion returned unstarted after its budget expired. The drain
//! re-reads the gauges when something actually changed and never polls on an
//! interval. Work admitted *outside* the host (an embedder driving
//! `governor()` directly) is counted, because the gauges are the engine's, but
//! its release does not ping; such work is seen when the timeout elapses and
//! the drain takes its final look.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use taskmesh_contract::{Snapshot, TaskClass};
#[cfg(doc)]
use taskmesh_engine::Governor;
use tokio::sync::Notify;
use tokio::time::Instant;

use super::TokioRuntime;

/// Work still charged to one class when a drain gave up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Outstanding {
    /// Requests holding capacity in any [`ExecutionPhase`](taskmesh_contract::ExecutionPhase).
    /// The caller may already have its answer (D10); the worker has not
    /// terminated.
    pub inflight: u32,
    /// Requests waiting in the class admission queue.
    pub queued: u32,
}

/// The runtime drained: every class reported `inflight == 0 && queued == 0`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DrainReport {
    /// From the call until the idle observation.
    pub elapsed: Duration,
    /// Classes whose gauges the drain observed at zero — every class the
    /// governor knows, whether or not it ever had work.
    pub classes_drained: usize,
}

/// The timeout elapsed with work still charged.
///
/// The runtime is still draining: new submissions stay refused, the listed
/// work keeps running under its lease, and a second [`TokioRuntime::drain`]
/// continues the same wait. The counts are the engine's at the moment of the
/// final look, not the caller's view of who has replied (D10).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NotDrained {
    /// Every class with `inflight > 0 || queued > 0`; classes at zero are
    /// omitted.
    pub classes: BTreeMap<TaskClass, Outstanding>,
    /// From the call until the final look; at least the requested timeout.
    pub elapsed: Duration,
}

impl fmt::Display for NotDrained {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "runtime not drained after {:?}:", self.elapsed)?;
        for (class, left) in &self.classes {
            write!(
                f,
                " {class} inflight={} queued={};",
                left.inflight, left.queued
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for NotDrained {}

/// Host-side drain state, shared by every clone of the runtime handle and by
/// every lease and ticket guard the host issues.
///
/// The *decision* to refuse lives in the engine; this holds only what the host
/// needs to wait efficiently: the wake-up, and a mirror of the closed flag
/// that lets the release hot path skip the wake-up while nothing is draining.
#[derive(Debug, Default)]
pub(super) struct DrainSignal {
    /// Mirror of the engine's closed flag, set by `drain` *before* its first
    /// look at the gauges and never cleared. Read only to gate `settled`.
    draining: AtomicBool,
    /// Pinged when host-owned custody returns to the engine.
    settled: Notify,
}

impl DrainSignal {
    fn begin(&self) {
        self.draining.store(true, Ordering::SeqCst);
    }

    /// Custody returned to the engine: wake a drain that may be waiting on it.
    ///
    /// Gated on the mirror so the release hot path pays one atomic load, not a
    /// lock, while nothing is draining. The gate cannot lose a wake-up: the
    /// engine transition that returned the custody and the drain's look at the
    /// gauges take the same mutex, so a release that read the mirror as clear
    /// happened before the drain's look and is already in the numbers, while a
    /// release after that look sees the mirror (stored before the look) and
    /// pings a `Notified` created before the look.
    pub(super) fn settled(&self) {
        if self.draining.load(Ordering::SeqCst) {
            self.settled.notify_waiters();
        }
    }
}

/// The classes with work still charged, from one authoritative snapshot.
fn outstanding(snapshot: &Snapshot) -> BTreeMap<TaskClass, Outstanding> {
    snapshot
        .classes
        .iter()
        .filter(|(_, class)| class.inflight > 0 || class.queued > 0)
        .map(|(name, class)| {
            (
                name.clone(),
                Outstanding {
                    inflight: class.inflight,
                    queued: class.queued,
                },
            )
        })
        .collect()
}

async fn drain_deadline_reached(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending::<()>().await,
    }
}

impl TokioRuntime {
    /// Whether [`Self::drain`] has been called on this runtime or any clone of
    /// it — the engine's own flag ([`Governor::admission_closed`]), so `true`
    /// means every later submission is refused. One-way: there is no resume.
    pub fn is_draining(&self) -> bool {
        self.governor.admission_closed()
    }

    /// Stop taking work and wait, up to `timeout`, for the work already
    /// admitted to finish.
    ///
    /// From the first call on, every submission path (`run_io`, `run_blocking`,
    /// `run_cpu`, `run_local`, the requested-stack paths, and their `_with`
    /// variants) is refused *before admission* with
    /// `GovernorError::Rejected(AdmissionVerdict::RuntimeUnavailable)`: nothing
    /// is queued and nothing is charged. The refusal is the engine's
    /// ([`Governor::close_admission`]), so a submission is either admitted
    /// before the close — and then counted — or refused after it; there is no
    /// third state. Work already queued or in flight is not cancelled — the
    /// drain is graceful — and a queued request is still promoted and run when
    /// capacity frees. A submission the plan refuses for its own shape
    /// (unknown class, substrate mismatch, unsupported deadline) is refused for
    /// that reason first; either way nothing is admitted.
    ///
    /// Returns `Ok` once every class reports `inflight == 0 && queued == 0` on
    /// the engine's own gauges; at that point `snapshot()` shows the same,
    /// `conservation_violation()` is `None`, and nothing can be admitted any
    /// more. Returns [`NotDrained`] with the per-class counts when `timeout`
    /// elapses first; the runtime **stays draining**, the listed work keeps
    /// running under its lease, and a later call continues waiting. Dropping
    /// the handle remains the only teardown.
    ///
    /// The wait is event-driven: each host-owned return of custody wakes it
    /// (see the module docs). A `timeout` the clock cannot represent is treated
    /// as unbounded. An idle runtime returns at once; `Duration::ZERO` takes
    /// one look and answers.
    pub async fn drain(&self, timeout: Duration) -> Result<DrainReport, NotDrained> {
        let started = Instant::now();
        // Mirror first, then close: every release after the close sees the
        // mirror, and every release before it is in the first look below.
        self.drain.begin();
        self.governor.close_admission();
        let deadline = started.checked_add(timeout);
        loop {
            // Created before the look, so a release that lands between the
            // look and the wait still completes it (`notify_waiters` wakes
            // every `Notified` in existence at the call).
            let settled = self.drain.settled.notified();
            let snapshot = self.governor.snapshot();
            let classes = outstanding(&snapshot);
            if classes.is_empty() {
                return Ok(DrainReport {
                    elapsed: started.elapsed(),
                    classes_drained: snapshot.classes.len(),
                });
            }
            let now = Instant::now();
            if deadline.is_some_and(|deadline| now >= deadline) {
                return Err(NotDrained {
                    classes,
                    elapsed: now.duration_since(started),
                });
            }
            tokio::select! {
                biased;
                () = settled => {}
                () = drain_deadline_reached(deadline) => {}
            }
        }
    }
}
