//! Regression tests for the audit-driven cancellation/deadline hardening (host):
//! `CooperativeWithDeadline` is real, cooperative cancel covers run_io/run_local,
//! and run_cpu can escape a stalled CpuExecutor.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use taskmesh::ext::*;
use taskmesh::*;

fn rt(policy: CancellationPolicy) -> TokioRuntime {
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

fn single_slot_deadline_rt() -> TokioRuntime {
    Builder::new()
        .resources(ResourceBudget::new().cpu_units(64).memory_units(64))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(8)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth)
                .cancellation_policy(CancellationPolicy::CooperativeWithDeadline),
        )
        .build()
        .unwrap()
}

// ---- F: run deadline is real for CooperativeWithDeadline -------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deadline_cancels_cooperative_with_deadline() {
    let rt = rt(CancellationPolicy::CooperativeWithDeadline);
    let spec = TaskSpec::io(TaskClass::new("c")).operation("op");
    let opts = SubmitOptions::unbounded().with_deadline(Duration::from_millis(40));
    let err = rt
        .run_io_with(spec, opts, std::future::pending::<Result<(), ()>>())
        .await
        .expect_err("deadline must fire");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::DeadlineExceeded)
    ));
    tokio::task::yield_now().await;
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deadline_ignored_for_plain_cooperative() {
    // A plain Cooperative class ignores `deadline` (only CooperativeWithDeadline
    // honors it) — the work runs to completion.
    let rt = rt(CancellationPolicy::Cooperative);
    let spec = TaskSpec::io(TaskClass::new("c")).operation("op");
    let opts = SubmitOptions::unbounded().with_deadline(Duration::from_millis(20));
    let out: i32 = rt
        .run_io_with(spec, opts, async {
            tokio::time::sleep(Duration::from_millis(60)).await;
            Ok::<_, ()>(7)
        })
        .await
        .expect("plain cooperative ignores deadline");
    assert_eq!(out, 7);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requested_stack_async_runtime_honors_cooperative_deadline_v1() {
    let rt = rt(CancellationPolicy::CooperativeWithDeadline);
    let spec = TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
        .operation("requested-stack-deadline")
        .stack_size_bytes(2 * 1024 * 1024);
    let opts = SubmitOptions::unbounded().with_deadline(Duration::from_millis(40));

    let error = rt
        .run_async_with_requested_stack_with(spec, opts, || async {
            std::future::pending::<Result<(), ()>>().await
        })
        .await
        .expect_err("requested-stack async deadline must fire on the owned runtime");

    assert!(matches!(
        error,
        RunError::Governor(GovernorError::DeadlineExceeded)
    ));
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn absolute_deadline_is_one_budget_across_queue_and_execution_v1() {
    let rt = single_slot_deadline_rt();
    let spec = TaskSpec::io(TaskClass::new("c")).operation("absolute-deadline");
    let occupied = match rt.governor().admit(&spec) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected occupied permit, got {other:?}"),
    };
    let releaser = tokio::spawn({
        let rt = rt.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            rt.governor().release(occupied);
        }
    });
    let started = Arc::new(AtomicBool::new(false));
    let worker_started = Arc::clone(&started);
    let options = SubmitOptions::unbounded()
        .with_absolute_deadline(std::time::Instant::now() + Duration::from_millis(250));

    let error = rt
        .run_io_with(spec, options, async move {
            worker_started.store(true, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(300)).await;
            Ok::<(), ()>(())
        })
        .await
        .expect_err("queue wait and execution must share one absolute deadline");

    releaser.await.expect("permit releaser completes");
    assert!(
        started.load(Ordering::SeqCst),
        "work must start after admission"
    );
    assert!(matches!(
        error,
        RunError::Governor(GovernorError::DeadlineExceeded)
    ));
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requested_stack_absolute_deadline_drops_owned_root_v1() {
    struct DropSignal(Arc<AtomicBool>);
    impl Drop for DropSignal {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let rt = rt(CancellationPolicy::CooperativeWithDeadline);
    let spec = TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
        .operation("requested-stack-absolute-deadline")
        .stack_size_bytes(2 * 1024 * 1024);
    let dropped = Arc::new(AtomicBool::new(false));
    let drop_state = Arc::clone(&dropped);
    let options = SubmitOptions::unbounded()
        .with_absolute_deadline(std::time::Instant::now() + Duration::from_millis(50));

    let error = rt
        .run_async_with_requested_stack_with(spec, options, move || {
            let signal = DropSignal(drop_state);
            async move {
                let _signal = signal;
                std::future::pending::<Result<(), ()>>().await
            }
        })
        .await
        .expect_err("absolute deadline must cancel the owned requested-stack root");

    assert!(matches!(
        error,
        RunError::Governor(GovernorError::DeadlineExceeded)
    ));
    for _ in 0..50 {
        if dropped.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}

#[tokio::test]
async fn elapsed_absolute_deadline_rejects_before_factory_v1() {
    let rt = rt(CancellationPolicy::CooperativeWithDeadline);
    let spec = TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
        .operation("elapsed-absolute-deadline")
        .stack_size_bytes(2 * 1024 * 1024);
    let invoked = Arc::new(AtomicBool::new(false));
    let factory_invoked = Arc::clone(&invoked);
    let options = SubmitOptions::unbounded()
        .with_absolute_deadline(std::time::Instant::now() - Duration::from_millis(1));

    let error = rt
        .run_async_with_requested_stack_with(spec, options, move || {
            factory_invoked.store(true, Ordering::SeqCst);
            async { Ok::<(), ()>(()) }
        })
        .await
        .expect_err("elapsed absolute deadline must fail before admission and factory creation");

    assert!(matches!(
        error,
        RunError::Governor(GovernorError::DeadlineExceeded)
    ));
    assert!(!invoked.load(Ordering::SeqCst));
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}

#[tokio::test]
async fn absolute_deadline_rejects_non_cancellable_blocking_work_v1() {
    let rt = rt(CancellationPolicy::CooperativeWithDeadline);
    let invoked = Arc::new(AtomicBool::new(false));
    let job_invoked = Arc::clone(&invoked);
    let options = SubmitOptions::unbounded()
        .with_absolute_deadline(std::time::Instant::now() + Duration::from_secs(1));

    let error = rt
        .run_blocking_with(
            TaskSpec::blocking(TaskClass::new("c")).operation("blocking-absolute-deadline"),
            options,
            move || {
                job_invoked.store(true, Ordering::SeqCst);
                Ok::<(), ()>(())
            },
        )
        .await
        .expect_err("blocking work cannot promise cancellation at an absolute deadline");

    assert!(matches!(
        error,
        RunError::Governor(GovernorError::PolicyViolation(message))
            if message.contains("drop-cancellable async work")
    ));
    assert!(!invoked.load(Ordering::SeqCst));
}

#[tokio::test]
async fn absolute_deadline_rejects_non_cancellable_cpu_work_v1() {
    let rt = rt(CancellationPolicy::CooperativeWithDeadline);
    let invoked = Arc::new(AtomicBool::new(false));
    let job_invoked = Arc::clone(&invoked);
    let options = SubmitOptions::unbounded()
        .with_absolute_deadline(std::time::Instant::now() + Duration::from_secs(1));

    let error = rt
        .run_cpu_with(
            TaskSpec::cpu(TaskClass::new("c")).operation("cpu-absolute-deadline"),
            options,
            move || {
                job_invoked.store(true, Ordering::SeqCst);
                Ok::<(), ()>(())
            },
        )
        .await
        .expect_err("CPU work cannot promise cancellation at an absolute deadline");

    assert!(matches!(
        error,
        RunError::Governor(GovernorError::PolicyViolation(message))
            if message.contains("drop-cancellable async work")
    ));
    assert!(!invoked.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requested_stack_async_runtime_honors_cooperative_cancel_v1() {
    let rt = rt(CancellationPolicy::Cooperative);
    let token = CancellationToken::new();
    let spec = TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
        .operation("requested-stack-cancel")
        .stack_size_bytes(2 * 1024 * 1024);
    let opts = SubmitOptions::unbounded().with_cancel(token.clone());
    let worker = tokio::spawn({
        let rt = rt.clone();
        async move {
            rt.run_async_with_requested_stack_with(spec, opts, || async {
                std::future::pending::<Result<(), ()>>().await
            })
            .await
        }
    });

    while rt.snapshot().classes[&TaskClass::new("c")].inflight != 1 {
        tokio::task::yield_now().await;
    }
    token.cancel();
    let error = worker
        .await
        .expect("caller task must complete")
        .expect_err("requested-stack async cancellation must stop the owned root");

    assert!(matches!(
        error,
        RunError::Governor(GovernorError::Cancelled)
    ));
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}

// ---- F: run_local honors cooperative cancel (was previously excluded) ------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_local_honors_cooperative_cancel() {
    let rt = rt(CancellationPolicy::Cooperative);
    let token = CancellationToken::new();
    let opts = SubmitOptions::unbounded().with_cancel(token.clone());
    let spec = TaskSpec::local(TaskClass::new("c")).operation("op");

    // run_local's future is !Send, so it stays on this task; a spawned timer
    // fires the token concurrently.
    let firer = {
        let token = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            token.cancel();
        })
    };
    let err = rt
        .run_local_with(spec, opts, async {
            let _rc = std::rc::Rc::new(()); // !Send, proves the local path
            std::future::pending::<()>().await;
            Ok::<i32, ()>(1)
        })
        .await
        .expect_err("run_local must cancel mid-run");
    firer.await.unwrap();
    assert!(matches!(err, RunError::Governor(GovernorError::Cancelled)));
}

// ---- F: run_cpu can escape a stalled CpuExecutor via cancel ----------------

/// A CpuExecutor that never runs (nor drops) the work — simulates a stalled or
/// saturated custom pool. Holding the box keeps the `oneshot` sender alive, so
/// `rx.await` would hang forever without cooperative cancel.
struct StallExecutor {
    held: Mutex<Vec<Box<dyn FnOnce() + Send + 'static>>>,
}

impl CpuExecutor for StallExecutor {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        self.held.lock().unwrap().push(work);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_cpu_escapes_stalled_executor_via_cancel() {
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(64).memory_units(64))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(4)
                .cpu_units(1)
                .cancellation_policy(CancellationPolicy::Cooperative),
        )
        .cpu_executor(Arc::new(StallExecutor {
            held: Mutex::new(Vec::new()),
        }))
        .build()
        .unwrap();

    let token = CancellationToken::new();
    let opts = SubmitOptions::unbounded().with_cancel(token.clone());
    let spec = TaskSpec::cpu(TaskClass::new("c")).operation("op");
    let rt2 = rt.clone();
    let handle =
        tokio::spawn(async move { rt2.run_cpu_with(spec, opts, || Ok::<i32, ()>(1)).await });

    tokio::time::sleep(Duration::from_millis(40)).await;
    token.cancel();
    let err = handle
        .await
        .unwrap()
        .expect_err("must escape the stalled executor");
    assert!(matches!(err, RunError::Governor(GovernorError::Cancelled)));
    // Permit + cpu gate released despite the executor still holding the work.
    tokio::task::yield_now().await;
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}
