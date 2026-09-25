use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use taskmesh::{
    AdmissionVerdict, Builder, CancellationPolicy, ClassPolicy, GovernorError, OverflowPolicy,
    ResourceBudget, RunError, Runtime, SubmitOptions, TaskClass, TaskSpec, TokioRuntime,
};
use tokio_util::sync::CancellationToken;

const HANG: Duration = Duration::from_secs(5);

struct ReleaseOnDrop(Option<std::sync::mpsc::Sender<()>>);

impl ReleaseOnDrop {
    fn release(mut self) {
        self.0
            .take()
            .expect("holder release is available")
            .send(())
            .expect("worker still waits");
    }
}

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
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

#[derive(Debug, Clone, Copy)]
enum CallerExit {
    Deadline,
    Cancel,
    Drop,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_caller_keeps_worker_charged_through_pre_drain_queue() {
    for exit in [CallerExit::Deadline, CallerExit::Cancel, CallerExit::Drop] {
        let runtime = runtime(1);
        let cancelled = CancellationToken::new();
        let worker_finished = Arc::new(AtomicBool::new(false));
        let finished_in_worker = Arc::clone(&worker_finished);
        let second_started = Arc::new(AtomicBool::new(false));
        let started_in_second = Arc::clone(&second_started);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release = ReleaseOnDrop(Some(release_tx));

        let holder_runtime = runtime.clone();
        let holder_cancel = cancelled.clone();
        let mut holder = tokio::spawn(async move {
            let opts = match exit {
                CallerExit::Deadline => {
                    SubmitOptions::unbounded().with_deadline(Duration::from_millis(20))
                }
                CallerExit::Cancel => SubmitOptions::unbounded().with_cancel(holder_cancel),
                CallerExit::Drop => SubmitOptions::unbounded(),
            };
            holder_runtime
                .run_blocking_with(blocking("holder"), opts, move || {
                    started_tx.send(()).expect("start observer survives");
                    release_rx.recv().expect("holder is explicitly released");
                    finished_in_worker.store(true, Ordering::SeqCst);
                    Ok::<_, ()>(7_usize)
                })
                .await
        });
        tokio::time::timeout(HANG, started_rx)
            .await
            .expect("holder starts within bound")
            .expect("holder reports start");

        match exit {
            CallerExit::Deadline => assert!(matches!(
                tokio::time::timeout(HANG, &mut holder)
                    .await
                    .expect("deadline response is bounded")
                    .expect("holder caller joins"),
                Err(RunError::Governor(GovernorError::DeadlineExceeded))
            )),
            CallerExit::Cancel => {
                cancelled.cancel();
                assert!(matches!(
                    tokio::time::timeout(HANG, &mut holder)
                        .await
                        .expect("cancel response is bounded")
                        .expect("holder caller joins"),
                    Err(RunError::Governor(GovernorError::Cancelled))
                ));
            }
            CallerExit::Drop => {
                holder.abort();
                assert!(tokio::time::timeout(HANG, &mut holder)
                    .await
                    .expect("dropped caller settles")
                    .expect_err("caller receives no response")
                    .is_cancelled());
            }
        }

        let class = TaskClass::new("c");
        let after_response = runtime.snapshot();
        assert_eq!(after_response.classes[&class].inflight, 1, "{exit:?}");
        assert_eq!(after_response.classes[&class].running, 1, "{exit:?}");
        assert_eq!(after_response.classes[&class].cpu_units_held, 1, "{exit:?}");
        assert_eq!(
            after_response.capabilities["blocking"].in_use, 1,
            "{exit:?}"
        );
        assert!(!worker_finished.load(Ordering::SeqCst), "{exit:?}");

        let second_runtime = runtime.clone();
        let second = tokio::spawn(async move {
            second_runtime
                .run_io(
                    TaskSpec::io(TaskClass::new("c")).operation("second"),
                    async move {
                        started_in_second.store(true, Ordering::SeqCst);
                        Ok::<_, ()>(9_usize)
                    },
                )
                .await
        });
        tokio::time::timeout(HANG, async {
            while runtime.snapshot().classes[&class].queued != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("second request queues before drain");
        assert!(!second_started.load(Ordering::SeqCst), "{exit:?}");

        let outstanding = runtime
            .drain(Duration::ZERO)
            .await
            .expect_err("live worker and pre-drain queue forbid drain Ok");
        let observed = &outstanding.classes[&class];
        assert_eq!((observed.inflight, observed.queued), (1, 1), "{exit:?}");
        assert!(matches!(
            runtime
                .run_io(
                    TaskSpec::io(class.clone()).operation("after-drain"),
                    async { Ok::<_, ()>(()) }
                )
                .await,
            Err(RunError::Governor(GovernorError::Rejected(
                AdmissionVerdict::RuntimeUnavailable
            )))
        ));
        assert!(!second_started.load(Ordering::SeqCst), "{exit:?}");
        assert_eq!(runtime.snapshot().classes[&class].inflight, 1, "{exit:?}");

        release.release();
        assert_eq!(
            tokio::time::timeout(HANG, second)
                .await
                .expect("queued request completes within bound")
                .expect("queued caller joins"),
            Ok(9),
            "{exit:?}"
        );
        runtime
            .drain(HANG)
            .await
            .expect("drain succeeds after both workers settle");
        assert!(worker_finished.load(Ordering::SeqCst), "{exit:?}");
        assert!(second_started.load(Ordering::SeqCst), "{exit:?}");
        let final_snapshot = runtime.snapshot();
        let final_class = &final_snapshot.classes[&class];
        assert_eq!(
            (final_class.inflight, final_class.queued),
            (0, 0),
            "{exit:?}"
        );
        assert_eq!(final_class.admitted_total, 2, "{exit:?}");
        assert_eq!(final_class.terminated_total, 2, "{exit:?}");
        assert_eq!(
            final_snapshot.capabilities["blocking"].in_use, 0,
            "{exit:?}"
        );
    }
}
