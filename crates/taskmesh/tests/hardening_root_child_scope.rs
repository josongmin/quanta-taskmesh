//! BG25-007: declared stages and raw spawned children have different owners.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use taskmesh::*;

const HANG: Duration = Duration::from_secs(5);

async fn bounded<F: std::future::Future>(what: &str, future: F) -> F::Output {
    let Ok(output) = tokio::time::timeout(HANG, future).await else {
        panic!("{what}: the caller was still waiting after {HANG:?}");
    };
    output
}

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

    let result: usize = bounded(
        "declared root stage",
        rt.run_io(staged, async move {
            root_count.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ()>(7)
        }),
    )
    .await
    .expect("bootstrap stage runs");
    assert_eq!(result, 7);
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    let snapshot = rt.snapshot();
    assert_eq!(snapshot.classes[&class()].admitted_total, 1);
    assert_eq!(snapshot.classes[&class()].inflight, 0);
    assert_eq!(snapshot.capabilities["cpu"].in_use, 0);

    let later_count = Arc::clone(&executions);
    let later: usize = bounded(
        "separately submitted later stage",
        rt.run_cpu(TaskSpec::cpu(class()).operation("later"), move || {
            later_count.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ()>(11)
        }),
    )
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

    bounded(
        "ambient root completion",
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
        ),
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
    let failure = bounded("panicked root join", root)
        .await
        .expect_err("root panic reaches its caller");
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
    bounded("aborted root admission", started_rx)
        .await
        .expect("root holds its permit");
    assert_eq!(rt.snapshot().classes[&class()].inflight, 1);
    root.abort();
    assert!(bounded("aborted root join", root)
        .await
        .expect_err("caller aborted")
        .is_cancelled());
    assert_eq!(rt.snapshot().classes[&class()].inflight, 0);
    rt.drain(Duration::from_secs(1))
        .await
        .expect("aborted caller releases root custody");
}

#[test]
fn local_caller_panic_and_drop_refund_the_root_without_worker_reclassification() {
    // run_local is !Send, so drive it on the caller's current-thread runtime.
    // An unwind is a caller panic, not a detached worker's WorkerPanicked.
    let tokio = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime builds");
    let rt = runtime();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tokio.block_on(
            rt.run_local(TaskSpec::local(class()).operation("local-panic"), async {
                panic!("local caller panics while it owns the root lease");
                #[allow(unreachable_code, reason = "the caller must panic")]
                Ok::<(), ()>(())
            }),
        )
    }));
    assert!(
        panic.is_err(),
        "caller panic must not be converted to RunError"
    );
    assert_eq!(rt.snapshot().classes[&class()].inflight, 0);

    tokio.block_on(async {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
        let mut root = Box::pin(rt.run_local(
            TaskSpec::local(class()).operation("local-drop"),
            async move {
                let _observer_gone = started_tx.send(());
                std::future::pending::<Result<(), ()>>().await
            },
        ));
        tokio::select! {
            result = &mut root => panic!("local root must remain pending: {result:?}"),
            ready = bounded("local root starts", started_rx) => {
                ready.expect("local root signals admission");
            }
        }
        assert_eq!(rt.snapshot().classes[&class()].inflight, 1);
        drop(root);
        assert_eq!(rt.snapshot().classes[&class()].inflight, 0);

        bounded(
            "local capacity reused after panic and drop",
            rt.run_local(TaskSpec::local(class()).operation("local-next"), async {
                Ok::<(), ()>(())
            }),
        )
        .await
        .expect("next caller admits without a leaked root permit");
        assert_eq!(rt.snapshot().classes[&class()].inflight, 0);
    });
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
    let local_completed = Arc::new(AtomicBool::new(false));
    let ambient_completed = Arc::new(AtomicBool::new(false));
    let child_dropped = Arc::clone(&dropped);
    let child_completed = Arc::clone(&local_completed);
    let ambient_child_completed = Arc::clone(&ambient_completed);
    let (local_release_tx, local_release_rx) = tokio::sync::oneshot::channel::<()>();
    let (local_started_tx, local_started_rx) = tokio::sync::oneshot::channel::<()>();
    let (ambient_release_tx, ambient_release_rx) = tokio::sync::oneshot::channel::<()>();
    let (ambient_started_tx, ambient_started_rx) = tokio::sync::oneshot::channel::<()>();
    let (ambient_done_tx, ambient_done_rx) = tokio::sync::oneshot::channel::<()>();

    bounded(
        "local root completion",
        rt.run_local(
            TaskSpec::local(class()).operation("local-root"),
            async move {
                let _unawaited = tokio::task::spawn_local(async move {
                    let _notice = DropNotice(child_dropped);
                    let _observer_gone = local_started_tx.send(());
                    let _released = local_release_rx.await;
                    child_completed.store(true, Ordering::SeqCst);
                });
                let _ambient = tokio::spawn(async move {
                    let _observer_gone = ambient_started_tx.send(());
                    ambient_release_rx
                        .await
                        .expect("test releases ambient child");
                    ambient_child_completed.store(true, Ordering::SeqCst);
                    let _observer_gone = ambient_done_tx.send(());
                });
                local_started_rx.await.expect("local child starts");
                ambient_started_rx.await.expect("ambient child starts");
                Ok::<(), ()>(())
            },
        ),
    )
    .await
    .expect("root completes");

    assert_eq!(rt.snapshot().classes[&class()].inflight, 0);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(!local_completed.load(Ordering::SeqCst));
    assert_eq!(
        local_release_tx.send(()),
        Err(()),
        "local child receiver is gone"
    );
    rt.drain(Duration::from_secs(1))
        .await
        .expect("neither detached child holds the root's governed lease");
    assert!(!ambient_completed.load(Ordering::SeqCst));
    ambient_release_tx
        .send(())
        .expect("ambient child still outlives local root and drain");
    bounded("ambient child completion", ambient_done_rx)
        .await
        .expect("ambient child reports completion");
    assert!(ambient_completed.load(Ordering::SeqCst));
}
