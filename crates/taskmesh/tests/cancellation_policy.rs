//! Audit P2: cancellation_policy is enforced, not inert. A class that opts into
//! cooperative cancellation honors a mid-run token; a PreSubmitOnly class does
//! not (the token only gates pre-submit).

use std::time::Duration;

use taskmesh::*;

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
    let rt2 = rt.clone();
    let handle = tokio::spawn(async move {
        rt2.run_io_with(io_spec(), opts, async {
            // Never completes on its own — only cooperative cancel ends it.
            std::future::pending::<()>().await;
            Ok::<i32, ()>(1)
        })
        .await
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    token.cancel(); // fire mid-run

    let err = handle
        .await
        .unwrap()
        .expect_err("must be cancelled mid-run");
    assert!(matches!(err, RunError::Governor(GovernorError::Cancelled)));

    // Permit released by the guard — runtime drained and usable.
    tokio::task::yield_now().await;
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pre_submit_only_ignores_mid_run_token() {
    let rt = runtime(CancellationPolicy::PreSubmitOnly);
    let token = CancellationToken::new();
    let opts = SubmitOptions::unbounded().with_cancel(token.clone());

    let rt2 = rt.clone();
    let handle = tokio::spawn(async move {
        rt2.run_io_with(io_spec(), opts, async {
            tokio::time::sleep(Duration::from_millis(80)).await;
            Ok::<i32, ()>(42)
        })
        .await
    });

    // Fire the token mid-run; a PreSubmitOnly class must ignore it and finish.
    tokio::time::sleep(Duration::from_millis(20)).await;
    token.cancel();

    let out = handle
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
