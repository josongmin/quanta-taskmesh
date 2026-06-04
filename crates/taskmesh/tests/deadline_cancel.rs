//! Regression tests for the audit-driven cancellation/deadline hardening (host):
//! `CooperativeWithDeadline` is real, cooperative cancel covers run_io/run_local,
//! and run_cpu can escape a stalled CpuExecutor.

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
