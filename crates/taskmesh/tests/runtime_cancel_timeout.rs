//! T07: cancellation and bounded acquire are adapter-layer concerns, surfaced as
//! typed governor-side rejections.

use std::time::Duration;

use taskmesh::ext::*;
use taskmesh::*;

fn single_slot_runtime() -> TokioRuntime {
    Builder::new()
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(8)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .unwrap()
}

fn spec() -> TaskSpec {
    TaskSpec::blocking(TaskClass::new("c")).operation("op")
}

#[tokio::test]
async fn cancel_before_submit_is_governor_rejection() {
    let rt = single_slot_runtime();
    let token = CancellationToken::new();
    token.cancel();
    let opts = SubmitOptions::unbounded().with_cancel(token);

    let err = rt
        .run_blocking_with(spec(), opts, || Ok::<i32, ()>(1))
        .await
        .expect_err("cancelled before submit");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::CancelledBeforeSubmit
        ))
    ));
}

#[tokio::test]
async fn cancellation_token_fired_before_submit_rejects() {
    let rt = single_slot_runtime();
    let token = CancellationToken::new();
    let opts = SubmitOptions::unbounded().with_cancel(token.clone());
    token.cancel();

    let io_spec = TaskSpec::io(TaskClass::new("c")).operation("op");
    let err = rt
        .run_io_with(io_spec, opts, async { Ok::<i32, ()>(1) })
        .await
        .expect_err("token fired");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::CancelledBeforeSubmit
        ))
    ));
}

#[tokio::test]
async fn timed_acquire_yields_typed_governor_error() {
    let rt = single_slot_runtime();

    // Occupy the single slot directly via the shared governor so the next
    // submission must queue.
    let occupied = match rt.governor().admit(&spec()) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admit, got {other:?}"),
    };

    let opts = SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(50));
    let err = rt
        .run_blocking_with(spec(), opts, || Ok::<i32, ()>(1))
        .await
        .expect_err("acquire times out");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::PermitAcquireTimedOut { .. }
        ))
    ));

    rt.governor().release(occupied);
}
