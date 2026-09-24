//! Host acquisition lifecycle regressions using the real governor and builder.

use super::*;
use std::future::poll_fn;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::Poll;
use std::time::Duration;
use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, ManualClock, MemoryReleasePolicy, OverflowPolicy,
    ResourceBudget, TaskClass,
};
use taskmesh_engine::{AdvanceOutcome, ReleaseOutcome};

fn runtime() -> (TokioRuntime, Arc<ManualClock>, TaskSpec) {
    let mut runtime = crate::Builder::new()
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(
            TaskClass::new("claim-regression"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(8)
                .cpu_units(1)
                .memory_units(10)
                .memory_release_policy(MemoryReleasePolicy::LeakDetecting)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .expect("validated host configuration");
    // Replace only the clock port; retain the builder's actual policy/inventory.
    let clock = Arc::new(ManualClock::new(1_000));
    let policy = runtime.governor.policy().clone();
    runtime.governor = Arc::new(
        Governor::new(policy, clock.clone()).expect("same validated policy with controlled clock"),
    );
    let spec = TaskSpec::io(TaskClass::new("claim-regression")).operation("claim-lifecycle");
    (runtime, clock, spec)
}

static NEXT_OPERATION: AtomicUsize = AtomicUsize::new(1);

fn unique_spec(spec: &TaskSpec, prefix: &str) -> TaskSpec {
    let id = NEXT_OPERATION.fetch_add(1, Ordering::SeqCst);
    spec.clone().operation(format!("{prefix}-{id}"))
}

fn occupy(runtime: &TokioRuntime, spec: &TaskSpec) -> PermitId {
    match runtime.governor.admit(&unique_spec(spec, "holder")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected the first permit, got {other:?}"),
    }
}

fn validated_plan(
    runtime: &TokioRuntime,
    spec: &TaskSpec,
    opts: &SubmitOptions,
) -> ValidatedDispatchPlan {
    runtime
        .plan(
            unique_spec(spec, "waiter"),
            opts,
            &[SubstrateHint::AsyncIo],
            DispatchKind::CallerFuture,
        )
        .expect("test task is a valid IO dispatch")
}

fn arbiter(opts: &SubmitOptions) -> AcquisitionArbiter {
    AcquisitionArbiter::new(opts, None).expect("test acquisition budget is representable")
}

async fn park<F: Future>(mut future: Pin<&mut F>) {
    poll_fn(|cx| {
        assert!(
            future.as_mut().poll(cx).is_pending(),
            "waiter must first park"
        );
        Poll::Ready(())
    })
    .await;
}

async fn finish<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(1), future)
        .await
        .expect("a terminal claim must not park forever")
}

fn assert_accounting(runtime: &TokioRuntime, inflight: u32, queued: u32) {
    let snapshot = runtime.governor.snapshot();
    let class = &snapshot.classes[&TaskClass::new("claim-regression")];
    assert_eq!((class.inflight, class.queued), (inflight, queued));
    assert_eq!(class.cpu_units_held, u128::from(inflight));
    assert_eq!(class.memory_units_held, u128::from(inflight) * 10);
    assert_eq!(class.conservation_violation(), None);
}

#[tokio::test]
async fn ready_and_pending_promotion_transfer_ownership_once_v1() {
    let (runtime, _, spec) = runtime();
    let opts = SubmitOptions::unbounded();
    let first_plan = validated_plan(&runtime, &spec, &opts);
    let first_arbiter = arbiter(&opts);
    let first = runtime
        .acquire(&first_plan, &first_arbiter)
        .await
        .expect("ready");
    let second_plan = validated_plan(&runtime, &spec, &opts);
    let second_arbiter = arbiter(&opts);
    let mut waiter = Box::pin(runtime.acquire(&second_plan, &second_arbiter));
    park(waiter.as_mut()).await;
    assert_accounting(&runtime, 1, 1);
    assert_eq!(runtime.governor.release(first), ReleaseOutcome::Released);
    let second = finish(waiter).await.expect("promoted permit");
    assert_ne!(first, second);
    assert_accounting(&runtime, 1, 0);
    assert_eq!(runtime.governor.release(second), ReleaseOutcome::Released);
    assert_accounting(&runtime, 0, 0);
}

#[tokio::test]
async fn reclaimed_promotion_returns_public_typed_error_and_releases_guard_v1() {
    let (runtime, clock, spec) = runtime();
    let first = occupy(&runtime, &spec);
    let executed = AtomicBool::new(false);
    let mut waiter = Box::pin(
        runtime.run_io_with(spec, SubmitOptions::unbounded(), async {
            executed.store(true, Ordering::SeqCst);
            Ok::<(), ()>(())
        }),
    );
    park(waiter.as_mut()).await;
    assert_eq!(runtime.governor.release(first), ReleaseOutcome::Released);
    clock.advance(11);
    assert_eq!(runtime.governor.reap_leaks_with(10).reclaimed_permits, 1);
    let error = finish(waiter).await.expect_err("reclaimed promotion");
    assert!(
        !executed.load(Ordering::SeqCst),
        "reclaimed work must not execute"
    );
    let RunError::Governor(GovernorError::TicketClaimTerminated {
        ticket,
        reason: taskmesh_contract::TerminalReason::Reclaimed,
    }) = error
    else {
        panic!("must retain the terminal reason: {error:?}");
    };
    assert_eq!(runtime.governor.claim(ticket), ClaimOutcome::Invalid);
    assert_accounting(&runtime, 0, 0);
}

#[tokio::test]
async fn invalid_ticket_without_deadline_returns_instead_of_parking_v1() {
    let (runtime, _, _) = runtime();
    let waker = TokioPermitWaker::new();
    let opts = SubmitOptions::unbounded();
    let arbiter = arbiter(&opts);
    let error = finish(runtime.await_promotion(u64::MAX, &waker, &arbiter))
        .await
        .expect_err("never-issued ticket");
    assert_eq!(
        error,
        GovernorError::InvalidTicketClaim { ticket: u64::MAX }
    );
    assert_accounting(&runtime, 0, 0);
}

#[tokio::test]
async fn pending_timeout_removes_queued_ticket_without_releasing_holder_v1() {
    let (runtime, _, spec) = runtime();
    let first = occupy(&runtime, &spec);
    let opts = SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(10));
    let plan = validated_plan(&runtime, &spec, &opts);
    let arbiter = arbiter(&opts);
    let mut waiter = Box::pin(runtime.acquire(&plan, &arbiter));
    park(waiter.as_mut()).await;
    let error = finish(waiter).await.expect_err("queue deadline");
    assert_eq!(
        error,
        GovernorError::Rejected(AdmissionVerdict::PermitAcquireTimedOut {
            retry_after_ms: None
        }),
    );
    assert_accounting(&runtime, 1, 0);
    assert_eq!(runtime.governor.release(first), ReleaseOutcome::Released);
    assert_accounting(&runtime, 0, 0);
}

#[tokio::test]
async fn cancel_beats_claim_of_promoted_permit_and_guard_releases_it_v1() {
    let (runtime, _, spec) = runtime();
    let first = occupy(&runtime, &spec);
    let token = CancellationToken::new();
    let opts = SubmitOptions::unbounded().with_cancel(token.clone());
    let plan = validated_plan(&runtime, &spec, &opts);
    let arbiter = arbiter(&opts);
    let mut waiter = Box::pin(runtime.acquire(&plan, &arbiter));
    park(waiter.as_mut()).await;
    assert_eq!(runtime.governor.release(first), ReleaseOutcome::Released);
    token.cancel();
    let error = finish(waiter).await.expect_err("cancellation wins");
    assert_eq!(
        error,
        GovernorError::Rejected(AdmissionVerdict::CancelledBeforeSubmit)
    );
    assert_accounting(&runtime, 0, 0);
}

#[test]
fn cancel_between_immediate_admit_and_handoff_returns_unstarted_once_v1() {
    let (runtime, _, spec) = runtime();
    let permit = occupy(&runtime, &spec);
    let token = CancellationToken::new();
    let opts = SubmitOptions::unbounded().with_cancel(token.clone());
    let arbiter = arbiter(&opts);
    token.cancel();

    let error = runtime
        .finalize_acquired(permit, &arbiter, false, None)
        .expect_err("cancel at the handoff boundary wins");
    assert_eq!(
        error,
        GovernorError::Rejected(AdmissionVerdict::CancelledBeforeSubmit)
    );
    assert_accounting(&runtime, 0, 0);
    assert_eq!(
        runtime.governor.release(permit),
        ReleaseOutcome::UnknownPermit,
        "the unstarted permit was returned exactly once"
    );
}

#[tokio::test]
async fn dropped_acquisition_abandons_queued_and_promoted_tickets_v1() {
    for promote in [false, true] {
        let (runtime, _, spec) = runtime();
        let first = occupy(&runtime, &spec);
        let opts = SubmitOptions::unbounded();
        let plan = validated_plan(&runtime, &spec, &opts);
        let arbiter = arbiter(&opts);
        let mut waiter = Box::pin(runtime.acquire(&plan, &arbiter));
        park(waiter.as_mut()).await;
        if promote {
            assert_eq!(runtime.governor.release(first), ReleaseOutcome::Released);
        }
        drop(waiter);
        assert_accounting(&runtime, u32::from(!promote), 0);
        if !promote {
            assert_eq!(runtime.governor.release(first), ReleaseOutcome::Released);
        }
        assert_accounting(&runtime, 0, 0);
    }
}

fn queue_without_notification(runtime: &TokioRuntime, spec: &TaskSpec) -> u64 {
    match runtime.governor.admit(&unique_spec(spec, "queued")) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected a queue ticket, got {other:?}"),
    }
}

/// D09 on the queued path, *loop* branch: the waiter is woken (its promotion
/// already landed) but the budget is already spent when it looks. The `biased`
/// select prefers the wake over the timer, so this — not the timeout branch —
/// is the path a late promotion takes to start work. It must not.
#[tokio::test]
async fn a_promotion_found_on_wake_after_the_budget_is_spent_is_unwound_v1() {
    let (runtime, _, spec) = runtime();
    let first = occupy(&runtime, &spec);
    let ticket = queue_without_notification(&runtime, &spec);
    // Promote *before* the waiter ever polls: its first look finds `Ready`
    // through the loop path, with a budget that expired in the past.
    assert_eq!(runtime.governor.release(first), ReleaseOutcome::Released);
    assert!(matches!(
        runtime.governor.ticket_status(ticket),
        ClaimOutcome::Ready(_)
    ));
    let waker = TokioPermitWaker::new();
    let opts = SubmitOptions::unbounded().with_acquire_timeout(Duration::ZERO);
    let arbiter = arbiter(&opts);
    let error = finish(runtime.await_promotion(ticket, &waker, &arbiter))
        .await
        .expect_err("a promotion found after the budget is spent must not start");
    assert_eq!(
        error,
        GovernorError::Rejected(AdmissionVerdict::PermitAcquireTimedOut {
            retry_after_ms: None
        })
    );
    assert_eq!(runtime.governor.claim(ticket), ClaimOutcome::Invalid);
    assert_accounting(&runtime, 0, 0);
    let next = occupy(&runtime, &spec);
    assert_eq!(runtime.governor.release(next), ReleaseOutcome::Released);
    assert_accounting(&runtime, 0, 0);
}

/// D09 on the queued path, *timeout* branch: a promotion that lands after the
/// acquisition budget is spent is returned unstarted, exactly as an immediate
/// `Admitted` past the budget is on the fast path. The caller gets the timeout,
/// the permit goes straight back to the pool, and nothing is stranded
/// promoted-but-unclaimed.
#[tokio::test]
async fn timeout_after_racing_promotion_unwinds_instead_of_starting_v1() {
    let (runtime, _, spec) = runtime();
    let first = occupy(&runtime, &spec);
    let ticket = queue_without_notification(&runtime, &spec);
    let waker = TokioPermitWaker::new();
    let opts = SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(10));
    let arbiter = arbiter(&opts);
    let mut waiter = Box::pin(runtime.await_promotion(ticket, &waker, &arbiter));
    // No governor waker is registered for this ticket: only the deadline resumes
    // this suspended select, so what it does with the racing promotion is the
    // timeout branch's own decision, not the loop's.
    park(waiter.as_mut()).await;
    assert_eq!(runtime.governor.release(first), ReleaseOutcome::Released);
    assert!(
        matches!(
            runtime.governor.ticket_status(ticket),
            ClaimOutcome::Ready(_)
        ),
        "the release must have promoted the ticket before the timer fires"
    );
    let error = finish(waiter)
        .await
        .expect_err("budget spent: the promotion is not started");
    assert_eq!(
        error,
        GovernorError::Rejected(AdmissionVerdict::PermitAcquireTimedOut {
            retry_after_ms: None
        })
    );
    // The promoted permit was returned by the waiter itself, not left for a
    // guard: the ticket is gone and the class is empty again.
    assert_eq!(runtime.governor.claim(ticket), ClaimOutcome::Invalid);
    assert_accounting(&runtime, 0, 0);
    // And the capacity is genuinely back: the next admit succeeds immediately.
    let next = occupy(&runtime, &spec);
    assert_eq!(runtime.governor.release(next), ReleaseOutcome::Released);
    assert_accounting(&runtime, 0, 0);
}

#[tokio::test]
async fn timeout_last_chance_claim_preserves_reclaimed_terminal_reason_v1() {
    let (runtime, clock, spec) = runtime();
    let first = occupy(&runtime, &spec);
    let ticket = queue_without_notification(&runtime, &spec);
    let waker = TokioPermitWaker::new();
    let opts = SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(10));
    let arbiter = arbiter(&opts);
    let mut waiter = Box::pin(runtime.await_promotion(ticket, &waker, &arbiter));
    park(waiter.as_mut()).await;
    assert_eq!(runtime.governor.release(first), ReleaseOutcome::Released);
    clock.advance(11);
    assert_eq!(runtime.governor.reap_leaks_with(10).reclaimed_permits, 1);
    let error = finish(waiter).await.expect_err("terminal beats timeout");
    assert_eq!(
        error,
        GovernorError::TicketClaimTerminated {
            ticket,
            reason: taskmesh_contract::TerminalReason::Reclaimed,
        }
    );
    runtime.governor.abandon(ticket);
    assert_accounting(&runtime, 0, 0);
}

#[tokio::test]
async fn timeout_last_chance_claim_reports_invalid_instead_of_generic_timeout_v1() {
    let (runtime, _, spec) = runtime();
    let first = occupy(&runtime, &spec);
    let ticket = queue_without_notification(&runtime, &spec);
    let waker = TokioPermitWaker::new();
    let opts = SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(10));
    let arbiter = arbiter(&opts);
    let mut waiter = Box::pin(runtime.await_promotion(ticket, &waker, &arbiter));
    park(waiter.as_mut()).await;
    runtime.governor.abandon(ticket);
    let error = finish(waiter).await.expect_err("invalid beats timeout");
    assert_eq!(error, GovernorError::InvalidTicketClaim { ticket });
    assert_accounting(&runtime, 1, 0);
    assert_eq!(runtime.governor.release(first), ReleaseOutcome::Released);
    assert_accounting(&runtime, 0, 0);
}

#[tokio::test]
async fn a_lease_the_sweep_reclaimed_before_dispatch_refuses_to_advance_v1() {
    // The only way a granted permit disappears under its holder: the leak
    // sweep reclaims a `DispatchReserved` lease that went stale before the host
    // declared it `Accepted`. The lease must then refuse to advance (typed),
    // disarm itself, and release nothing on drop — never run work on capacity
    // the engine already handed to someone else, never double-release.
    let (runtime, clock, spec) = runtime();
    let permit = occupy(&runtime, &spec);
    let mut lease = ExecutionLease::reserved(Arc::clone(&runtime.governor), permit);
    clock.advance(11);
    assert_eq!(runtime.governor.reap_leaks_with(10).reclaimed_permits, 1);
    assert_accounting(&runtime, 0, 0);

    assert_eq!(
        lease.advance(ExecutionPhase::Accepted),
        Err(GovernorError::LeaseReclaimed { permit_id: permit })
    );
    // The answer is stable, and the drop releases nothing. (Internally the
    // lease disarms itself; that is defensive — with the permit gone the
    // engine answers `UnknownPermit` either way — and is not what this test
    // can observe. What it observes is that the work is refused and nothing
    // is double-counted.)
    assert_eq!(
        lease.advance(ExecutionPhase::Running),
        Err(GovernorError::LeaseReclaimed { permit_id: permit })
    );
    drop(lease);
    assert_accounting(&runtime, 0, 0);

    // Control: once `Accepted`, the sweep leaves the lease alone and the drop
    // is the one release.
    let permit = occupy(&runtime, &spec);
    let mut lease = ExecutionLease::reserved(Arc::clone(&runtime.governor), permit);
    assert_eq!(lease.advance(ExecutionPhase::Accepted), Ok(()));
    clock.advance(11);
    let report = runtime.governor.reap_leaks_with(10);
    assert_eq!((report.reclaimed_permits, report.retained_active), (0, 1));
    assert_accounting(&runtime, 1, 0);
    drop(lease);
    assert_accounting(&runtime, 0, 0);
}

#[tokio::test]
async fn a_phase_that_does_not_advance_is_reported_not_ignored_v1() {
    let (runtime, _, spec) = runtime();
    let permit = occupy(&runtime, &spec);
    let mut lease = ExecutionLease::reserved(Arc::clone(&runtime.governor), permit);
    assert_eq!(lease.advance(ExecutionPhase::Running), Ok(()));
    // Backwards, and repeated: both are host programming errors and both are
    // said out loud, while the permit stays live and owned.
    for phase in [ExecutionPhase::Accepted, ExecutionPhase::Running] {
        let error = lease
            .advance(phase)
            .expect_err("a non-advancing phase is refused");
        assert!(
            matches!(&error, GovernorError::PolicyViolation(message) if message.contains("does not advance")),
            "got {error:?}"
        );
    }
    assert_accounting(&runtime, 1, 0);
    drop(lease);
    assert_accounting(&runtime, 0, 0);
}

#[tokio::test]
async fn a_lease_taken_through_ext_is_refused_not_run_v1() {
    // Before dispatch the permit is not the runtime lease's alone (D14): an
    // `ext` caller can advance it and take the lease the engine mints on that
    // first advance. The runtime lease then cannot release the permit, so it
    // must not start work under it either — the work would run on capacity
    // this lease can never return. It refuses, typed, and its drop releases
    // nothing; the permit belongs to whoever holds the token.
    let (runtime, _, spec) = runtime();
    let permit = occupy(&runtime, &spec);
    let mut lease = ExecutionLease::reserved(Arc::clone(&runtime.governor), permit);
    let AdvanceOutcome::Leased(taken) = runtime
        .governor
        .advance_phase(permit, ExecutionPhase::Accepted)
    else {
        panic!("the first advance of a reserved permit leases it");
    };

    let error = lease
        .advance(ExecutionPhase::Running)
        .expect_err("a runtime lease must not run under a lease it does not hold");
    assert!(
        matches!(&error, GovernorError::PolicyViolation(message) if message.contains("leased outside its runtime lease")),
        "got {error:?}"
    );
    // Phase declarations are not token-gated — only the release is — so the
    // engine did record the move; the capacity is what stays with the token.
    assert_eq!(
        runtime.governor.phase(permit),
        Some(ExecutionPhase::Running)
    );
    drop(lease);
    assert_accounting(&runtime, 1, 0);

    assert_eq!(
        runtime.governor.release_leased(taken),
        ReleaseOutcome::Released
    );
    assert_accounting(&runtime, 0, 0);
}
