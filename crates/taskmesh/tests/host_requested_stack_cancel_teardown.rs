//! Cancellation may end caller response while owned-runtime teardown retains custody.

use std::sync::mpsc;
use std::time::Duration;

use taskmesh::*;

const STACK: u64 = 2 * 1024 * 1024;

fn runtime() -> TokioRuntime {
    Builder::new()
        .topology(TopologyConfig::new().large_stack_slots(1))
        .resources(ResourceBudget::new().cpu_units(10).memory_units(10))
        .class_policy(
            TaskClass::new("cleanup"),
            ClassPolicy::new()
                .max_inflight(1)
                .cancellation_policy(CancellationPolicy::Cooperative),
        )
        .build()
        .expect("requested-stack runtime")
}

async fn wait_for_cleanup(rt: &TokioRuntime) -> bool {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let ledgers = rt.governor().permit_ledgers();
            if ledgers.len() == 1 && ledgers[0].phase == ExecutionPhase::CleanupPending {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancellation_during_teardown_answers_before_blocking_child_exits() {
    for root_result in [Ok::<i32, &'static str>(7), Err("task failed")] {
        let rt = runtime();
        let token = CancellationToken::new();
        let (child_started_tx, child_started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let submitted = rt.clone();
        let options = SubmitOptions::unbounded().with_cancel(token.clone());
        let caller = tokio::spawn(async move {
            submitted
                .run_async_with_requested_stack_with(
                    TaskSpec::base(
                        TaskClass::new("cleanup"),
                        SubstrateHint::LargeStackCapability,
                    )
                    .operation("cancel-during-teardown")
                    .stack_size_bytes(STACK),
                    options,
                    move || async move {
                        let _child = tokio::task::spawn_blocking(move || {
                            child_started_tx
                                .send(())
                                .expect("root waits for child start");
                            let _released = release_rx.recv();
                        });
                        child_started_rx.await.expect("blocking child started");
                        root_result
                    },
                )
                .await
        });
        if !wait_for_cleanup(&rt).await {
            let finished = caller.is_finished();
            drop(release_tx);
            caller.abort();
            panic!(
                "root did not enter teardown: {:?}, caller_finished={finished}",
                rt.snapshot()
            );
        }
        assert!(!caller.is_finished(), "teardown holds the normal response");
        token.cancel();
        let response = tokio::time::timeout(Duration::from_millis(500), caller).await;
        // Send before asserting so the owned runtime can still tear down on RED.
        let charged = rt.snapshot().classes[&TaskClass::new("cleanup")].inflight;
        release_tx.send(()).expect("blocking child remains held");
        assert_eq!(charged, 1, "caller cancellation cannot refund live custody");
        let result = response
            .expect("cancellation must answer before teardown")
            .expect("caller task joins");
        assert_eq!(result, Err(RunError::Governor(GovernorError::Cancelled)));
        tokio::time::timeout(Duration::from_secs(5), rt.drain(Duration::from_secs(5)))
            .await
            .expect("drain bounded")
            .expect("worker teardown releases capacity");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn normal_response_waits_for_teardown_and_observes_released_capacity() {
    let rt = runtime();
    let (child_started_tx, child_started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let submitted = rt.clone();
    let caller = tokio::spawn(async move {
        submitted
            .run_async_with_requested_stack(
                TaskSpec::base(
                    TaskClass::new("cleanup"),
                    SubstrateHint::LargeStackCapability,
                )
                .operation("normal-teardown")
                .stack_size_bytes(STACK),
                move || async move {
                    let _child = tokio::task::spawn_blocking(move || {
                        child_started_tx
                            .send(())
                            .expect("root waits for child start");
                        let _released = release_rx.recv();
                    });
                    child_started_rx.await.expect("blocking child started");
                    Ok::<_, ()>(7)
                },
            )
            .await
    });
    if !wait_for_cleanup(&rt).await {
        let finished = caller.is_finished();
        drop(release_tx);
        caller.abort();
        panic!(
            "root did not enter teardown: {:?}, caller_finished={finished}",
            rt.snapshot()
        );
    }
    assert!(!caller.is_finished(), "normal result waits for teardown");
    assert_eq!(
        rt.snapshot().classes[&TaskClass::new("cleanup")].inflight,
        1
    );
    release_tx.send(()).expect("blocking child remains held");
    let result = tokio::time::timeout(Duration::from_secs(5), caller)
        .await
        .expect("caller responds after teardown")
        .expect("caller task joins");
    assert_eq!(result, Ok(7));
    assert_eq!(
        rt.snapshot().classes[&TaskClass::new("cleanup")].inflight,
        0
    );
}
