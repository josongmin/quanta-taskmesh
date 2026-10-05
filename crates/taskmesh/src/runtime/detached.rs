//! Detached synchronous worker settlement and caller response.

use super::{
    finalize_released_response, oneshot, panic, AssertUnwindSafe, BlockingJobV1, CancellationToken,
    Duration, ExecutionLease, ExecutionPhase, GovernorError, Instant, RunError,
};

/// What a detached worker sends back when it finishes.
pub(super) struct DetachedOutcome<T, E> {
    result: Result<T, RunError<E>>,
    started_at: std::time::Instant,
    completed_at: std::time::Instant,
    /// Custody travels with the result, so a caller that observes completion has
    /// already released the capacity.
    lease: ExecutionLease,
}

/// The shared worker body for every detached synchronous dispatch.
///
/// One wrapper so the start timestamp, the phase transitions, the panic
/// boundary, and the lease handoff are identical on the blocking pool, the CPU
/// executor port, and a dedicated stack thread. The differences between those
/// three are *where* the closure runs, not what governance it owes.
pub(super) fn run_detached_job<T, E>(
    job: BlockingJobV1<T, E>,
    lease: ExecutionLease,
    started_tx: oneshot::Sender<std::time::Instant>,
    done_tx: oneshot::Sender<DetachedOutcome<T, E>>,
    context: &'static str,
    response_by: Option<std::time::Instant>,
) {
    // The worker's own clock is the authority for a relative run budget. Timing
    // from submission instead would charge the job for queue time it did not
    // spend running — and would miss the case where the adapter runs it inline,
    // where "submitted" and "finished" are the same instant.
    let started_at = std::time::Instant::now();
    let mut lease = lease;
    // A lease the engine no longer backs must not run: the sweep reclaimed it
    // while the job sat in the executor's queue. Report, do not execute.
    let result = if response_by.is_some_and(|deadline| started_at >= deadline) {
        Err(RunError::Governor(GovernorError::DeadlineExceeded))
    } else if let Err(error) = lease.advance(ExecutionPhase::Running) {
        Err(RunError::Governor(error))
    } else {
        // The caller may already have stopped waiting; that is expected, and
        // the completion message below is what actually carries custody back.
        let _undelivered = started_tx.send(started_at);
        let outcome = panic::catch_unwind(AssertUnwindSafe(job));
        match (outcome, lease.advance(ExecutionPhase::CleanupPending)) {
            (Ok(Ok(value)), Ok(())) => Ok(value),
            (Ok(Err(error)), Ok(())) => Err(RunError::Task(error)),
            (Err(_panic), Ok(())) => Err(RunError::Governor(GovernorError::WorkerPanicked {
                context: context.into(),
            })),
            // Unreachable once `Running` was accepted (nothing but this lease
            // can end the permit), reported anyway rather than pretending the
            // gauges are right.
            (_, Err(error)) => Err(RunError::Governor(error)),
        }
    };
    let completed_at = std::time::Instant::now();
    // A failed send means the caller stopped waiting (timeout, cancel, drop).
    // That is a normal, expected delivery outcome — not a lost message — and
    // this worker is then the owner that releases the lease, which happens when
    // the returned payload is dropped here.
    let _undelivered = done_tx.send(DetachedOutcome {
        result,
        started_at,
        completed_at,
        lease,
    });
}

/// Await a detached worker, racing its completion against cancellation and the
/// run budget.
///
/// `RunFor` is anchored to the worker's own start. A separate `response_by`
/// covers acquisition and caller response; a synchronous worker keeps custody
/// after that response expires. Cooperative `CompleteBy` cannot reach here.
/// Completion at either boundary is a deadline miss, including an inline
/// executor result delivered before the timer is polled.
pub(super) async fn await_detached<T, E>(
    started_rx: oneshot::Receiver<std::time::Instant>,
    done_rx: oneshot::Receiver<DetachedOutcome<T, E>>,
    cancel: Option<CancellationToken>,
    run_budget: Option<Duration>,
    context: &'static str,
    response_by: Option<std::time::Instant>,
) -> Result<T, RunError<E>> {
    let cancelled = async {
        match &cancel {
            Some(token) => token.cancelled().await,
            None => std::future::pending::<()>().await,
        }
    };
    let expiry = async {
        match run_budget {
            None => std::future::pending::<()>().await,
            Some(budget) => match started_rx.await {
                // The budget starts when the work does.
                Ok(started_at) => match started_at.checked_add(budget) {
                    Some(expires_at) => {
                        tokio::time::sleep_until(Instant::from_std(expires_at)).await;
                    }
                    None => std::future::pending::<()>().await,
                },
                // The worker never started and never will; completion decides.
                Err(_) => std::future::pending::<()>().await,
            },
        }
    };
    let mut done_rx = done_rx;
    tokio::select! {
        biased;
        () = cancelled, if cancel.is_some() => Err(RunError::Governor(GovernorError::Cancelled)),
        () = async {
            match response_by {
                Some(deadline) => tokio::time::sleep_until(Instant::from_std(deadline)).await,
                None => std::future::pending::<()>().await,
            }
        }, if response_by.is_some() => Err(RunError::Governor(GovernorError::DeadlineExceeded)),
        () = expiry, if run_budget.is_some() => Err(RunError::Governor(GovernorError::DeadlineExceeded)),
        received = &mut done_rx => match received {
            Ok(outcome) => {
                let run_expired = run_budget.is_some_and(|budget| {
                    outcome
                        .started_at
                        .checked_add(budget)
                        .is_some_and(|expires_at| outcome.completed_at >= expires_at)
                });
                let DetachedOutcome { result, lease, .. } = outcome;
                // Release fence: drop custody before the caller is answered.
                drop(lease);
                let result = if run_expired {
                    Err(RunError::Governor(GovernorError::DeadlineExceeded))
                } else {
                    result
                };
                finalize_released_response(result, cancel.as_ref(), response_by)
            }
            Err(_) => Err(RunError::Governor(GovernorError::JobAbandoned {
                context: context.into(),
            })),
        },
    }
}
