//! Regression (audit H1): a submission whose future is dropped mid-run must
//! still release its permit — no leaked inflight/budget accounting.

use std::time::Duration;

use taskmesh::*;

fn runtime() -> TokioRuntime {
    Builder::new()
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(2)
                .max_queue_depth(8)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .unwrap()
}

fn spec(op: &str) -> TaskSpec {
    TaskSpec::io(TaskClass::new("c")).operation(op.to_string())
}

#[tokio::test]
async fn permit_released_when_run_future_is_cancelled() {
    let rt = runtime();
    let c = TaskClass::new("c");

    // run_io over a future that never resolves; cancel it via an outer timeout.
    let never = rt.run_io(spec("hang"), std::future::pending::<Result<(), ()>>());
    let timed = tokio::time::timeout(Duration::from_millis(50), never).await;
    assert!(
        timed.is_err(),
        "the submission must be cancelled by timeout"
    );

    // The permit must have been released by the RAII guard on drop.
    tokio::task::yield_now().await;
    assert_eq!(
        rt.snapshot().classes[&c].inflight,
        0,
        "cancelled submission must not leak a permit"
    );

    // And the runtime is still usable afterward.
    let out: i32 = rt
        .run_io(spec("ok"), async { Ok::<_, ()>(7) })
        .await
        .unwrap();
    assert_eq!(out, 7);
}

#[tokio::test]
async fn cancellation_frees_capacity_for_the_next_request() {
    let rt = runtime();
    let c = TaskClass::new("c");

    // Saturate both slots with hanging submissions, then cancel them.
    let h1 = rt.run_io(spec("h1"), std::future::pending::<Result<(), ()>>());
    let h2 = rt.run_io(spec("h2"), std::future::pending::<Result<(), ()>>());
    let both = async {
        let _ = tokio::join!(h1, h2);
    };
    let _ = tokio::time::timeout(Duration::from_millis(50), both).await;

    tokio::task::yield_now().await;
    assert_eq!(rt.snapshot().classes[&c].inflight, 0, "both permits freed");
}
