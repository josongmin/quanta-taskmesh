//! Audit P2: cancellation_policy is enforced, not inert. A class that opts into
//! cooperative cancellation honors a mid-run token; a PreSubmitOnly class does
//! not (the token only gates pre-submit).

use std::time::Duration;

use taskmesh::*;

/// How long a bounded-wait assertion waits before declaring a hang. Far above
/// any budget under test, far below "the test binary never finishes".
const HANG: Duration = Duration::from_secs(5);

/// Await `future` for at most [`HANG`]; a longer wait is reported as the named
/// regression, not endured. A test whose only failure mode is a hang is not a
/// test the CI runner can attribute to anything.
async fn bounded<F: std::future::Future>(what: &str, future: F) -> F::Output {
    let Ok(output) = tokio::time::timeout(HANG, future).await else {
        panic!("{what}: the caller was still waiting after {HANG:?}");
    };
    output
}

fn runtime(policy: CancellationPolicy) -> TokioRuntime {
    Builder::new()
        .resources(ResourceBudget::new().cpu_units(64).memory_units(64))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(4)
                .cpu_units(1)
                .cancellation_policy(policy),
        )
        .build()
        .unwrap()
}

fn io_spec() -> TaskSpec {
    TaskSpec::io(TaskClass::new("c")).operation("op")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cooperative_class_cancels_mid_run() {
    let rt = runtime(CancellationPolicy::Cooperative);
    let token = CancellationToken::new();
    let opts = SubmitOptions::unbounded().with_cancel(token.clone());

    // Long-running work that would never finish on its own.
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let rt2 = rt.clone();
    let handle = tokio::spawn(async move {
        rt2.run_io_with(io_spec(), opts, async {
            started_tx
                .send(())
                .expect("test observes work admission before cancellation");
            // Never completes on its own — only cooperative cancel ends it.
            std::future::pending::<()>().await;
            Ok::<i32, ()>(1)
        })
        .await
    });

    bounded(
        "cooperative_class_cancels_mid_run: work was never admitted",
        started_rx,
    )
    .await
    .expect("work start signal sender survives");
    token.cancel(); // fire mid-run

    let err = bounded(
        "cooperative_class_cancels_mid_run: the cancelled work never returned",
        handle,
    )
    .await
    .unwrap()
    .expect_err("must be cancelled mid-run");
    assert!(matches!(err, RunError::Governor(GovernorError::Cancelled)));

    // Permit released by the guard — runtime drained and usable.
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pre_submit_only_ignores_mid_run_token() {
    let rt = runtime(CancellationPolicy::PreSubmitOnly);
    let token = CancellationToken::new();
    let opts = SubmitOptions::unbounded().with_cancel(token.clone());

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
    let rt2 = rt.clone();
    let handle = tokio::spawn(async move {
        rt2.run_io_with(io_spec(), opts, async {
            started_tx
                .send(())
                .expect("test observes work admission before cancellation");
            finish_rx
                .await
                .expect("test releases the pre-submit-only work");
            Ok::<i32, ()>(42)
        })
        .await
    });

    // Fire the token mid-run; a PreSubmitOnly class must ignore it and finish.
    bounded(
        "pre_submit_only_ignores_mid_run_token: work was never admitted",
        started_rx,
    )
    .await
    .expect("work start signal sender survives");
    token.cancel();
    finish_tx
        .send(())
        .expect("work remains live after a pre-submit-only cancellation");

    let out = bounded(
        "pre_submit_only_ignores_mid_run_token: work did not finish after release",
        handle,
    )
    .await
    .unwrap()
    .expect("pre-submit-only ignores mid-run cancel");
    assert_eq!(out, 42);
}

#[tokio::test]
async fn cancel_before_submit_still_rejects_regardless_of_policy() {
    // Pre-submit cancellation is always honored (independent of the policy).
    let rt = runtime(CancellationPolicy::PreSubmitOnly);
    let token = CancellationToken::new();
    token.cancel();
    let opts = SubmitOptions::unbounded().with_cancel(token);
    let err = rt
        .run_io_with(io_spec(), opts, async { Ok::<i32, ()>(1) })
        .await
        .expect_err("cancelled before submit");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::CancelledBeforeSubmit
        ))
    ));
}
