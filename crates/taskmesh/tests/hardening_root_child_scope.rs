//! BG25-007: declared stages and raw spawned children have different owners.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use taskmesh::*;

fn class() -> TaskClass {
    TaskClass::new("c")
}

fn runtime() -> TokioRuntime {
    Builder::new()
        .resources(ResourceBudget::new().cpu_units(8).memory_units(8))
        .class_policy(class(), ClassPolicy::new().max_inflight(2).cpu_units(1))
        .build()
        .expect("valid runtime")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declared_later_stages_and_reduce_do_not_execute_without_caller_submission() {
    let rt = runtime();
    let executions = Arc::new(AtomicUsize::new(0));
    let root_count = Arc::clone(&executions);
    let staged = TaskSpec::io(class())
        .operation("root")
        .stage(TaskStage::new("rank"), SubstrateHint::SharedCpuExecutor)
        .reduce_stage(
            TaskStage::new("merge"),
            SubstrateHint::SharedCpuExecutor,
            DeterministicReducePolicy::keyed("stable_id"),
        );

    let result: usize = rt
        .run_io(staged, async move {
            root_count.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ()>(7)
        })
        .await
        .expect("bootstrap stage runs");
    assert_eq!(result, 7);
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    let snapshot = rt.snapshot();
    assert_eq!(snapshot.classes[&class()].admitted_total, 1);
    assert_eq!(snapshot.classes[&class()].inflight, 0);
    assert_eq!(snapshot.capabilities["cpu"].in_use, 0);

    let later_count = Arc::clone(&executions);
    let later: usize = rt
        .run_cpu(TaskSpec::cpu(class()).operation("later"), move || {
            later_count.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ()>(11)
        })
        .await
        .expect("caller submits later work separately");
    assert_eq!(later, 11);
    assert_eq!(executions.load(Ordering::SeqCst), 2);
    assert_eq!(rt.snapshot().classes[&class()].admitted_total, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ambient_tokio_child_outlives_governed_root_and_drain() {
    let rt = runtime();
    let completed = Arc::new(AtomicBool::new(false));
    let child_completed = Arc::clone(&completed);
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();

    rt.run_io(
        TaskSpec::io(class()).operation("ambient-root"),
        async move {
            let _detached = tokio::spawn(async move {
                let _observer_gone = started_tx.send(());
                release_rx.await.expect("test releases ambient child");
                child_completed.store(true, Ordering::SeqCst);
                let _observer_gone = done_tx.send(());
            });
            started_rx.await.expect("ambient child starts");
            Ok::<(), ()>(())
        },
    )
    .await
    .expect("root completes independently");

    assert_eq!(rt.snapshot().classes[&class()].inflight, 0);
    rt.drain(Duration::from_secs(1))
        .await
        .expect("drain does not own ambient child");
    assert!(!completed.load(Ordering::SeqCst));
    release_tx.send(()).expect("ambient child remains alive");
    tokio::time::timeout(Duration::from_secs(1), done_rx)
        .await
        .expect("ambient child completes after release")
        .expect("ambient child sends completion");
    assert!(completed.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn panicked_root_refunds_its_lease_but_not_ambient_child_custody() {
    let rt = runtime();
    let host = rt.clone();
    let completed = Arc::new(AtomicBool::new(false));
    let child_completed = Arc::clone(&completed);
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();

    let root = tokio::spawn(async move {
        host.run_io(
            TaskSpec::io(class()).operation("panicked-root"),
            async move {
                let _detached = tokio::spawn(async move {
                    let _observer_gone = started_tx.send(());
                    release_rx.await.expect("test releases ambient child");
                    child_completed.store(true, Ordering::SeqCst);
                    let _observer_gone = done_tx.send(());
                });
                started_rx.await.expect("ambient child starts");
                panic!("root panic is contained by its caller task");
                #[allow(unreachable_code, reason = "the root must panic")]
                Ok::<(), ()>(())
            },
        )
        .await
    });
    let failure = root.await.expect_err("root panic reaches its caller");
    assert!(failure.is_panic());
    assert_eq!(rt.snapshot().classes[&class()].inflight, 0);
    rt.drain(Duration::from_secs(1))
        .await
        .expect("root panic does not retain a governed lease");
    assert!(!completed.load(Ordering::SeqCst));
    release_tx.send(()).expect("ambient child remains alive");
    tokio::time::timeout(Duration::from_secs(1), done_rx)
        .await
        .expect("ambient child completes after release")
        .expect("ambient child sends completion");
    assert!(completed.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aborted_caller_root_refunds_only_its_own_lease() {
    let rt = runtime();
    let host = rt.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let (_release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let root = tokio::spawn(async move {
        host.run_io(
            TaskSpec::io(class()).operation("aborted-root"),
            async move {
                let _observer_gone = started_tx.send(());
                release_rx.await.expect("caller aborts before release");
                Ok::<(), ()>(())
            },
        )
        .await
    });
    started_rx.await.expect("root holds its permit");
    assert_eq!(rt.snapshot().classes[&class()].inflight, 1);
    root.abort();
    assert!(root.await.expect_err("caller aborted").is_cancelled());
    assert_eq!(rt.snapshot().classes[&class()].inflight, 0);
    rt.drain(Duration::from_secs(1))
        .await
        .expect("aborted caller releases root custody");
}

struct DropNotice(Arc<AtomicUsize>);

impl Drop for DropNotice {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unawaited_local_child_is_dropped_with_the_root_local_set() {
    let rt = runtime();
    let dropped = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicBool::new(false));
    let child_dropped = Arc::clone(&dropped);
    let child_completed = Arc::clone(&completed);
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();

    rt.run_local(
        TaskSpec::local(class()).operation("local-root"),
        async move {
            let _unawaited = tokio::task::spawn_local(async move {
                let _notice = DropNotice(child_dropped);
                let _observer_gone = started_tx.send(());
                let _released = release_rx.await;
                child_completed.store(true, Ordering::SeqCst);
            });
            started_rx.await.expect("local child starts");
            Ok::<(), ()>(())
        },
    )
    .await
    .expect("root completes");

    assert_eq!(rt.snapshot().classes[&class()].inflight, 0);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(!completed.load(Ordering::SeqCst));
    assert!(release_tx.send(()).is_err(), "local child receiver is gone");
    rt.drain(Duration::from_secs(1))
        .await
        .expect("dropped local child has no governed lease");
}
