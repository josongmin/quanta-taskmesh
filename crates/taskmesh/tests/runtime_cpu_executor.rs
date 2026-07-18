//! T09: run_cpu rides the executor abstraction; the Rayon adapter plugs in and
//! the large-stack/blocking path stays off the shared CPU pool.

use std::sync::Arc;

use taskmesh::*;
use taskmesh_rayon::RayonCpuExecutor;

struct AsyncFutureDropSignalV1(Option<tokio::sync::oneshot::Sender<()>>);

impl Drop for AsyncFutureDropSignalV1 {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

fn runtime_with_rayon(workers: usize) -> (TokioRuntime, usize) {
    let topology = TopologyConfig::new().cpu_fixed(workers);
    let rayon = RayonCpuExecutor::from_topology(&topology);
    let count = rayon.worker_count();
    let rt = Builder::new()
        .topology(topology)
        .resources(ResourceBudget::new().cpu_units(100).memory_units(100))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new().max_inflight(8).cpu_units(1),
        )
        .cpu_executor(Arc::new(rayon))
        .build()
        .unwrap();
    (rt, count)
}

#[tokio::test]
async fn cpu_work_executes_through_rayon_abstraction() {
    let (rt, workers) = runtime_with_rayon(3);
    assert_eq!(workers, 3, "worker count respects topology clamp");

    let spec = TaskSpec::cpu(TaskClass::new("c")).operation("op");
    let thread_name: String = rt
        .run_cpu(spec, || {
            Ok::<_, ()>(
                std::thread::current()
                    .name()
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .await
        .expect("cpu work runs");
    assert!(
        thread_name.starts_with("taskmesh-cpu-"),
        "cpu work ran on the shared rayon pool, got {thread_name:?}"
    );
}

#[tokio::test]
async fn large_stack_blocking_path_avoids_shared_cpu_pool() {
    let (rt, _) = runtime_with_rayon(2);

    // A large-stack-classified job goes through run_blocking, NOT the CPU
    // executor, so it must not land on a "taskmesh-cpu-*" rayon worker.
    let spec = TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
        .operation("big-stack");
    let thread_name: String = rt
        .run_blocking(spec, || {
            Ok::<_, ()>(
                std::thread::current()
                    .name()
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .await
        .expect("blocking work runs");
    assert!(
        !thread_name.starts_with("taskmesh-cpu-"),
        "large-stack work must not be routed to the shared CPU pool, got {thread_name:?}"
    );
}

#[tokio::test]
async fn large_stack_blocking_path_honors_requested_stack_contract_v1() {
    let (rt, _) = runtime_with_rayon(2);

    let spec = TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
        .operation("big-stack")
        .stack_size_bytes(2 * 1024 * 1024);
    let thread_name: String = rt
        .run_blocking(spec, || {
            Ok::<_, ()>(
                std::thread::current()
                    .name()
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .await
        .expect("large-stack work runs");
    assert_eq!(
        thread_name,
        "taskmesh.large_stack:c",
        "requested stack-size contract must route through the host-managed dedicated large-stack thread"
    );
}

#[tokio::test]
async fn large_stack_blocking_path_enters_current_tokio_handle_v1() {
    let (rt, _) = runtime_with_rayon(2);

    let spec = TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
        .operation("big-stack-reactor")
        .stack_size_bytes(2 * 1024 * 1024);
    let has_tokio_context: bool = rt
        .run_blocking(spec, || {
            Ok::<_, ()>(tokio::runtime::Handle::try_current().is_ok())
        })
        .await
        .expect("large-stack work runs");
    assert!(
        has_tokio_context,
        "requested large-stack thread must enter the caller Tokio handle before running the job"
    );
}

#[tokio::test]
async fn requested_stack_async_path_owns_root_and_spawned_child_runtime_v1() {
    let (rt, _) = runtime_with_rayon(2);

    let spec = TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
        .operation("big-stack-async")
        .stack_size_bytes(2 * 1024 * 1024);
    let (factory_thread, factory_has_context, root_thread, child_thread): (
        String,
        bool,
        String,
        String,
    ) = rt
        .run_async_with_requested_stack(spec, || {
            let factory_thread = std::thread::current()
                .name()
                .unwrap_or_default()
                .to_string();
            let factory_has_context = tokio::runtime::Handle::try_current().is_ok();
            let child = tokio::spawn(async {
                tokio::task::yield_now().await;
                std::thread::current()
                    .name()
                    .unwrap_or_default()
                    .to_string()
            });
            async move {
                let root_thread = std::thread::current()
                    .name()
                    .unwrap_or_default()
                    .to_string();
                let child_thread = child.await.map_err(|error| error.to_string())?;
                Ok::<_, String>((
                    factory_thread,
                    factory_has_context,
                    root_thread,
                    child_thread,
                ))
            }
        })
        .await
        .expect("requested-stack async work runs");

    let expected = "taskmesh.large_stack:c";
    assert_eq!(factory_thread, expected);
    assert!(factory_has_context);
    assert_eq!(root_thread, expected);
    assert_eq!(child_thread, expected);
}

#[tokio::test]
async fn requested_stack_async_path_requires_explicit_stack_bytes_v1() {
    let (rt, _) = runtime_with_rayon(2);
    let spec = TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
        .operation("missing-stack-async");

    let error = rt
        .run_async_with_requested_stack(spec, || async { Ok::<_, ()>(()) })
        .await
        .expect_err("async large-stack work without requested bytes must fail closed");

    assert!(matches!(
        error,
        RunError::Governor(GovernorError::PolicyViolation(message))
            if message.contains("requested stack size missing")
    ));
}

#[tokio::test]
async fn requested_stack_async_path_projects_factory_panic_as_governor_error_v1() {
    let (rt, _) = runtime_with_rayon(2);
    let spec = TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
        .operation("panic-stack-async")
        .stack_size_bytes(2 * 1024 * 1024);

    let error = rt
        .run_async_with_requested_stack(spec, || -> std::future::Ready<Result<(), ()>> {
            panic!("factory panic")
        })
        .await
        .expect_err("requested-stack async factory panic must fail closed");

    assert!(matches!(
        error,
        RunError::Governor(GovernorError::PolicyViolation(message))
            if message.contains("requested-stack async worker panicked")
    ));
}

#[tokio::test]
async fn requested_stack_async_path_supports_worker_local_non_send_future_v1() {
    let (rt, _) = runtime_with_rayon(2);
    let spec = TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
        .operation("non-send-stack-async")
        .stack_size_bytes(2 * 1024 * 1024);

    let value = rt
        .run_async_with_requested_stack(spec, || {
            let value = std::rc::Rc::new(7_u32);
            async move {
                tokio::task::yield_now().await;
                Ok::<_, ()>(*value)
            }
        })
        .await
        .expect("worker-local future need not be Send");

    assert_eq!(value, 7);
}

#[tokio::test]
async fn requested_stack_async_path_preserves_task_error_v1() {
    let (rt, _) = runtime_with_rayon(2);
    let spec = TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
        .operation("task-error-stack-async")
        .stack_size_bytes(2 * 1024 * 1024);

    let error = rt
        .run_async_with_requested_stack(spec, || async { Err::<(), _>("task-failed") })
        .await
        .expect_err("task error must remain distinct from governor failure");

    assert!(matches!(error, RunError::Task("task-failed")));
}

#[tokio::test]
async fn requested_stack_async_path_rejects_wrong_substrate_v1() {
    let (rt, _) = runtime_with_rayon(2);
    let spec = TaskSpec::blocking(TaskClass::new("c"))
        .operation("wrong-substrate-stack-async")
        .stack_size_bytes(2 * 1024 * 1024);

    let error = rt
        .run_async_with_requested_stack(spec, || async { Ok::<_, ()>(()) })
        .await
        .expect_err("requested-stack async work requires the large-stack capability");

    assert!(matches!(
        error,
        RunError::Governor(GovernorError::Rejected(AdmissionVerdict::SubstrateMismatch))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requested_stack_async_caller_abort_drops_worker_future_v1() {
    let (rt, _) = runtime_with_rayon(2);
    let spec = TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
        .operation("caller-abort-stack-async")
        .stack_size_bytes(2 * 1024 * 1024);
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let worker = tokio::spawn({
        let rt = rt.clone();
        async move {
            rt.run_async_with_requested_stack(spec, move || {
                let _ = started_tx.send(());
                let drop_signal = AsyncFutureDropSignalV1(Some(dropped_tx));
                async move {
                    let _drop_signal = drop_signal;
                    std::future::pending::<Result<(), ()>>().await
                }
            })
            .await
        }
    });

    started_rx.await.expect("worker future must start");
    worker.abort();
    let _ = worker.await;
    tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
        .await
        .expect("worker future must be dropped after caller abort")
        .expect("drop signal sender must complete");
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while rt.snapshot().classes[&TaskClass::new("c")].inflight != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("permit must release after the worker terminates");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requested_stack_async_caller_abort_retains_permit_until_sync_poll_terminates_v1() {
    let (rt, _) = runtime_with_rayon(2);
    let spec = TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
        .operation("caller-abort-long-poll-stack-async")
        .stack_size_bytes(2 * 1024 * 1024);
    let (poll_started_tx, poll_started_rx) = tokio::sync::oneshot::channel();
    let (release_poll_tx, release_poll_rx) = std::sync::mpsc::channel();
    let submission = tokio::spawn({
        let rt = rt.clone();
        async move {
            rt.run_async_with_requested_stack(spec, move || {
                let mut poll_started_tx = Some(poll_started_tx);
                std::future::poll_fn(move |_| {
                    if let Some(started) = poll_started_tx.take() {
                        let _ = started.send(());
                    }
                    release_poll_rx
                        .recv()
                        .expect("test controls synchronous poll termination");
                    std::task::Poll::Ready(Ok::<(), ()>(()))
                })
            })
            .await
        }
    });

    poll_started_rx
        .await
        .expect("requested-stack worker must enter the synchronous poll");
    submission.abort();
    let abort = submission
        .await
        .expect_err("caller submission must observe task abort");
    assert!(abort.is_cancelled());
    assert_eq!(
        rt.snapshot().classes[&TaskClass::new("c")].inflight,
        1,
        "caller abort must not release a permit still owned by the worker"
    );

    release_poll_tx
        .send(())
        .expect("worker must still own the poll receiver");
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while rt.snapshot().classes[&TaskClass::new("c")].inflight != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("permit must release after the worker terminates");
}

#[tokio::test]
async fn default_runtime_works_without_rayon() {
    // Without an injected executor, run_cpu still works (blocking-pool default).
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(10))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new().max_inflight(4).cpu_units(1),
        )
        .build()
        .unwrap();
    let spec = TaskSpec::cpu(TaskClass::new("c")).operation("op");
    let out: i32 = rt.run_cpu(spec, || Ok::<_, ()>(5)).await.unwrap();
    assert_eq!(out, 5);
}
