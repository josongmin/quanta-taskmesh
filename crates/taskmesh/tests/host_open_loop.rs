use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use taskmesh::{
    Builder, CancellationPolicy, ClassPolicy, GovernorError, OverflowPolicy, ResourceBudget,
    RunError, Runtime, SubmitOptions, TaskClass, TaskSpec, TokioRuntime,
};

fn blocking(operation: &str) -> TaskSpec {
    TaskSpec::blocking(TaskClass::new("c")).operation(operation.to_owned())
}

fn runtime(depth: u32) -> TokioRuntime {
    Builder::new()
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(depth)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth)
                .cancellation_policy(CancellationPolicy::CooperativeWithDeadline),
        )
        .build()
        .expect("runtime builds")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn caller_terminal_response_and_worker_custody_are_separate_ledgers() {
    let runtime = runtime(1);
    let finished = Arc::new(AtomicBool::new(false));
    let worker_finished = finished.clone();
    let (release_tx, release_rx) = std::sync::mpsc::channel();

    let response = tokio::time::timeout(
        Duration::from_secs(5),
        runtime.run_blocking_with(
            blocking("deadline"),
            SubmitOptions::unbounded().with_deadline(Duration::from_millis(20)),
            move || {
                release_rx.recv().expect("test releases worker");
                worker_finished.store(true, Ordering::SeqCst);
                Ok::<_, ()>(7_usize)
            },
        ),
    )
    .await
    .expect("caller receives a bounded response");
    assert!(matches!(
        response,
        Err(RunError::Governor(GovernorError::DeadlineExceeded))
    ));
    assert!(!finished.load(Ordering::SeqCst));
    assert_eq!(runtime.snapshot().classes[&TaskClass::new("c")].inflight, 1);

    release_tx.send(()).expect("worker still owns custody");
    runtime
        .drain(Duration::from_secs(5))
        .await
        .expect("worker termination returns custody");
    assert!(finished.load(Ordering::SeqCst));
    assert_eq!(runtime.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}
