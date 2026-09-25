use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use taskmesh::{
    AdmissionVerdict, Builder, CancellationPolicy, ClassPolicy, GovernorError, OverflowPolicy,
    ResourceBudget, RunError, Runtime, SubmitOptions, TaskClass, TaskSpec, TokioRuntime,
};

fn spec(operation: &str) -> TaskSpec {
    TaskSpec::io(TaskClass::new("c")).operation(operation.to_owned())
}

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
async fn offered_terminal_unanswered_and_execution_counts_close_exactly() {
    const FOLLOWERS: usize = 8;
    let runtime = runtime(2);
    let executed = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let holder_runtime = runtime.clone();
    let holder_executed = executed.clone();
    let holder = tokio::spawn(async move {
        holder_runtime
            .run_blocking(blocking("holder"), move || {
                holder_executed.fetch_add(1, Ordering::SeqCst);
                let _ = started_tx.send(());
                release_rx.recv().expect("test releases holder");
                Ok::<_, ()>(0_usize)
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), started_rx)
        .await
        .expect("holder starts")
        .expect("holder reports start");

    let mut followers = Vec::new();
    for index in 0..FOLLOWERS {
        let runtime = runtime.clone();
        let executed = executed.clone();
        followers.push(tokio::spawn(async move {
            runtime
                .run_io(spec(&format!("follower-{index}")), async move {
                    executed.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, ()>(index)
                })
                .await
        }));
    }

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let class = &runtime.snapshot().classes[&TaskClass::new("c")];
            if class.queued == 2 && followers.iter().filter(|task| task.is_finished()).count() == 6
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("bounded producer reaches queue and rejection terminal states");

    release_tx.send(()).expect("holder waits");
    assert_eq!(holder.await.expect("holder task"), Ok(0));

    let mut succeeded = 0_usize;
    let mut rejected = 0_usize;
    for task in followers {
        match task.await.expect("follower task") {
            Ok(_) => succeeded += 1,
            Err(RunError::Governor(GovernorError::Rejected(AdmissionVerdict::QueueFull {
                ..
            }))) => rejected += 1,
            other => panic!("unexpected terminal result: {other:?}"),
        }
    }
    let offered = 1 + FOLLOWERS;
    let terminal = 1 + succeeded + rejected;
    let unanswered = offered - terminal;
    assert_eq!((offered, terminal, unanswered), (9, 9, 0));
    assert_eq!((succeeded, rejected), (2, 6));
    assert_eq!(executed.load(Ordering::SeqCst), 3);
    let final_state = runtime.snapshot();
    assert_eq!(
        (
            final_state.classes[&TaskClass::new("c")].inflight,
            final_state.classes[&TaskClass::new("c")].queued,
        ),
        (0, 0)
    );
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
