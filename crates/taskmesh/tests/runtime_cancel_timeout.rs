//! T07: cancellation and bounded acquire are adapter-layer concerns, surfaced as
//! typed governor-side rejections.

use std::time::Duration;

use taskmesh::ext::*;
use taskmesh::*;

fn single_slot_runtime() -> TokioRuntime {
    Builder::new()
        .topology(TopologyConfig::new().blocking_threads(1))
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

async fn wait_until_queue_depth(rt: &TokioRuntime, expected: usize) {
    for _ in 0..50 {
        if rt.snapshot().classes[&TaskClass::new("c")].queued == expected as u32 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("queue depth never reached {expected}");
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
async fn cancel_before_submit_beats_substrate_pool_contention() {
    let rt = single_slot_runtime();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();

    let holder = tokio::spawn({
        let rt = rt.clone();
        async move {
            let mut started_tx = Some(started_tx);
            rt.run_blocking_with(spec(), SubmitOptions::unbounded(), move || {
                started_tx
                    .take()
                    .expect("holder sends started once")
                    .send(())
                    .expect("receiver alive");
                std::thread::sleep(Duration::from_millis(120));
                Ok::<(), ()>(())
            })
            .await
        }
    });

    started_rx.await.expect("holder must start");

    let token = CancellationToken::new();
    token.cancel();
    let opts = SubmitOptions::unbounded()
        .with_cancel(token)
        .with_acquire_timeout(Duration::from_millis(40));
    let err = rt
        .run_blocking_with(spec(), opts, || Ok::<i32, ()>(1))
        .await
        .expect_err("pre-submit cancellation must win before substrate wait");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::CancelledBeforeSubmit
        ))
    ));

    holder.await.unwrap().expect("holder must complete");
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

#[tokio::test]
async fn acquire_timeout_is_one_budget_across_substrate_and_queue_wait() {
    let rt = single_slot_runtime();

    let occupied = match rt.governor().admit(&spec()) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admit, got {other:?}"),
    };

    let queued_holder = tokio::spawn({
        let rt = rt.clone();
        async move {
            rt.run_blocking_with(
                spec(),
                SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(60)),
                || Ok::<i32, ()>(1),
            )
            .await
        }
    });

    wait_until_queue_depth(&rt, 1).await;

    let opts = SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(60));
    let timed = tokio::time::timeout(
        Duration::from_millis(110),
        rt.run_blocking_with(spec(), opts, || Ok::<i32, ()>(1)),
    )
    .await
    .expect("acquire timeout must be end-to-end, not per-phase")
    .expect_err("acquire must still time out");
    assert!(matches!(
        timed,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::PermitAcquireTimedOut { .. }
                | AdmissionVerdict::SubstratePoolTimedOut { .. }
        ))
    ));

    let queued_err = queued_holder
        .await
        .unwrap()
        .expect_err("queued holder must time out");
    assert!(matches!(
        queued_err,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::PermitAcquireTimedOut { .. }
        ))
    ));

    rt.governor().release(occupied);
}
