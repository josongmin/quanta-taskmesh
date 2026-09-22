//! Regression tests for the audit-driven cancellation/deadline hardening (host):
//! `CooperativeWithDeadline` is real, cooperative cancel covers run_io/run_local,
//! and run_cpu can escape a stalled CpuExecutor.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use taskmesh::ext::*;
use taskmesh::*;

/// How long a bounded-wait assertion waits before declaring a hang. Far above
/// any budget under test, far below "the test binary never finishes".
const HANG: Duration = Duration::from_secs(5);

/// Await `future` for at most [`HANG`]; a longer wait is reported as the named
/// regression, not endured. A test whose only failure mode is a hang is not a
/// test the CI runner can attribute to anything.
async fn bounded<F: std::future::Future>(what: &str, future: F) -> F::Output {
    let Ok(output) = tokio::time::timeout(HANG, future).await else {
        panic!("{what}: the caller was still waiting after {HANG:?}");
    };
    output
}

/// Wait (bounded) for a class to drain, then assert it did.
///
/// A caller's answer and the work's custody are separate events: a deadline
/// reply can arrive while the worker is still tearing down its owned runtime,
/// and during that window the work is still charged — deliberately, because the
/// capacity is still in use. So "did it drain?" is a question about the end
/// state, not about the instant the caller returned. An unbounded wait would
/// turn a leak into a hung test, hence the deadline.
async fn assert_drains(rt: &TokioRuntime, class: &str, what: &str) {
    let class = TaskClass::new(class.to_string());
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = rt.snapshot();
        let observed = &snapshot.classes[&class];
        if observed.inflight == 0 && observed.queued == 0 {
            assert_eq!(
                snapshot.conservation_violation(),
                None,
                "{what}: drained snapshot must still satisfy conservation"
            );
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{what}: never drained (inflight {}, queued {})",
            observed.inflight,
            observed.queued
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

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
    let err = bounded(
        "deadline_cancels_cooperative_with_deadline: the 40ms run deadline never fired",
        rt.run_io_with(spec, opts, std::future::pending::<Result<(), ()>>()),
    )
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
async fn deadline_on_a_class_that_cannot_enforce_it_is_refused_not_ignored() {
    // Only `CooperativeWithDeadline` can honor `deadline`. A plain `Cooperative`
    // (or `PreSubmitOnly`) class used to *drop* the deadline and run the work to
    // completion, returning `Ok` for a bound it never applied. That is refused
    // at submit now: the caller asked for a bound, and the class cannot give one.
    for policy in [
        CancellationPolicy::Cooperative,
        CancellationPolicy::PreSubmitOnly,
    ] {
        let rt = rt(policy);
        let spec = TaskSpec::io(TaskClass::new("c")).operation("op");
        let opts = SubmitOptions::unbounded().with_deadline(Duration::from_millis(20));
        let ran = Arc::new(AtomicBool::new(false));
        let ran_in_work = Arc::clone(&ran);
        let error = rt
            .run_io_with(spec, opts, async move {
                ran_in_work.store(true, Ordering::SeqCst);
                Ok::<i32, ()>(7)
            })
            .await
            .expect_err("a deadline the class cannot enforce must be refused");
        assert_eq!(
            error,
            RunError::Governor(GovernorError::DeadlineUnsupported {
                class: TaskClass::new("c"),
                policy,
            }),
            "policy {policy:?}"
        );
        assert!(
            !ran.load(Ordering::SeqCst),
            "refused work must not start ({policy:?})"
        );
        assert_eq!(
            rt.snapshot().classes[&TaskClass::new("c")].inflight,
            0,
            "nothing was admitted ({policy:?})"
        );
    }
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
    // The deadline answer is not a claim that the worker is gone: the owned
    // runtime is still being torn down under this lease. What must hold is that
    // the lease is released once — and only once — when that finishes.
    assert_drains(&rt, "c", "requested-stack async deadline").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn absolute_deadline_is_one_budget_across_queue_and_execution_v1() {
    let rt = single_slot_deadline_rt();
    let spec = TaskSpec::io(TaskClass::new("c")).operation("absolute-deadline");
    let occupied = match rt
        .governor()
        .admit(&TaskSpec::io(TaskClass::new("c")).operation("absolute-holder"))
    {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected occupied permit, got {other:?}"),
    };
    let releaser = tokio::spawn({
        let rt = rt.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(rt.governor().release(occupied), ReleaseOutcome::Released);
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
async fn an_absolute_deadline_that_expires_while_queued_is_deadline_exceeded_v1() {
    // The absolute deadline is one budget across the queue wait and the run.
    // When it expires while the request is still queued, the queue wait ends
    // as an acquisition timeout — but the caller asked for a completion bound,
    // not an acquisition bound, and the error must say which budget ran out.
    // The occupied permit is never released: the deadline is the only thing
    // that can end the wait.
    let rt = single_slot_deadline_rt();
    let spec = TaskSpec::io(TaskClass::new("c")).operation("expires-while-queued");
    let occupied = match rt
        .governor()
        .admit(&TaskSpec::io(TaskClass::new("c")).operation("expiry-holder"))
    {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected occupied permit, got {other:?}"),
    };
    let started = Arc::new(AtomicBool::new(false));
    let worker_started = Arc::clone(&started);
    let options = SubmitOptions::unbounded()
        .with_absolute_deadline(std::time::Instant::now() + Duration::from_millis(60));
    let error = bounded(
        "an_absolute_deadline_that_expires_while_queued_is_deadline_exceeded_v1: the queue wait never ended",
        rt.run_io_with(spec, options, async move {
            worker_started.store(true, Ordering::SeqCst);
            Ok::<(), ()>(())
        }),
    )
    .await
    .expect_err("the absolute deadline expires before the slot frees");
    assert!(
        !started.load(Ordering::SeqCst),
        "work that never acquired a permit must not start"
    );
    assert!(
        matches!(error, RunError::Governor(GovernorError::DeadlineExceeded)),
        "an absolute deadline expiring in the queue is DeadlineExceeded, not an acquisition timeout: got {error:?}"
    );
    assert_eq!(rt.governor().release(occupied), ReleaseOutcome::Released);
    assert_drains(&rt, "c", "queued request abandoned on deadline").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_acquire_timeout_under_a_far_absolute_deadline_stays_an_acquisition_timeout_v1() {
    // The converse: the acquisition budget ran out while the completion budget
    // has most of a minute left. The caller's completion bound was not the
    // reason the wait ended, so reporting `DeadlineExceeded` would blame a
    // budget that never expired. The occupied permit is never released.
    let rt = single_slot_deadline_rt();
    let spec = TaskSpec::io(TaskClass::new("c")).operation("acquire-times-out");
    let occupied = match rt
        .governor()
        .admit(&TaskSpec::io(TaskClass::new("c")).operation("timeout-holder"))
    {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected occupied permit, got {other:?}"),
    };
    let options = SubmitOptions::unbounded()
        .with_acquire_timeout(Duration::from_millis(40))
        .with_absolute_deadline(std::time::Instant::now() + Duration::from_secs(60));
    let error = bounded(
        "an_acquire_timeout_under_a_far_absolute_deadline_stays_an_acquisition_timeout_v1: the acquire timeout never fired",
        rt.run_io_with(spec, options, async { Ok::<(), ()>(()) }),
    )
    .await
    .expect_err("the acquire timeout expires before the slot frees");
    assert!(
        matches!(
            error,
            RunError::Governor(GovernorError::Rejected(
                AdmissionVerdict::PermitAcquireTimedOut { .. }
            ))
        ),
        "an acquisition timeout under a live absolute deadline stays PermitAcquireTimedOut: got {error:?}"
    );
    assert_eq!(rt.governor().release(occupied), ReleaseOutcome::Released);
    assert_drains(&rt, "c", "queued request abandoned on acquire timeout").await;
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
    // Same separation as the relative case: the caller has its answer; the
    // worker releases custody when its owned runtime has finished tearing down.
    assert_drains(&rt, "c", "requested-stack absolute deadline").await;
}

#[tokio::test]
async fn elapsed_absolute_deadline_rejects_before_factory_v1() {
    let rt = rt(CancellationPolicy::CooperativeWithDeadline);
    let spec = TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
        .operation("elapsed-absolute-deadline")
        .stack_size_bytes(2 * 1024 * 1024);
    let invoked = Arc::new(AtomicBool::new(false));
    let factory_invoked = Arc::clone(&invoked);
    let options = SubmitOptions::unbounded().with_absolute_deadline(
        std::time::Instant::now()
            .checked_sub(Duration::from_millis(1))
            .expect("the process started more than a millisecond ago"),
    );

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
            if message.contains("cooperative async work")
    ));
    assert!(!invoked.load(Ordering::SeqCst));

    // The same job with a stack request runs on a dedicated thread instead of
    // the pool — still a synchronous job that cannot be stopped, so the
    // absolute deadline is refused the same way. (It used to be accepted:
    // the dedicated-thread dispatch was shared with the *async* requested-stack
    // path, which genuinely can honor one.)
    let invoked = Arc::new(AtomicBool::new(false));
    let job_invoked = Arc::clone(&invoked);
    let error = rt
        .run_blocking_with(
            TaskSpec::blocking(TaskClass::new("c"))
                .operation("stack-absolute-deadline")
                .stack_size_bytes(2 * 1024 * 1024),
            SubmitOptions::unbounded()
                .with_absolute_deadline(std::time::Instant::now() + Duration::from_secs(1)),
            move || {
                job_invoked.store(true, Ordering::SeqCst);
                Ok::<(), ()>(())
            },
        )
        .await
        .expect_err("a blocking job on a dedicated thread cannot promise cancellation either");
    assert!(matches!(
        error,
        RunError::Governor(GovernorError::PolicyViolation(message))
            if message.contains("cooperative async work")
    ));
    assert!(!invoked.load(Ordering::SeqCst));
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
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
            if message.contains("cooperative async work")
    ));
    assert!(!invoked.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requested_stack_absolute_deadline_holds_permit_during_long_poll_v1() {
    let rt = rt(CancellationPolicy::CooperativeWithDeadline);
    let spec = TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
        .operation("requested-stack-long-poll")
        .stack_size_bytes(2 * 1024 * 1024);
    let poll_started = Arc::new(AtomicBool::new(false));
    let worker_poll_started = Arc::clone(&poll_started);
    let options = SubmitOptions::unbounded()
        .with_absolute_deadline(std::time::Instant::now() + Duration::from_millis(30));
    let submission = tokio::spawn({
        let rt = rt.clone();
        async move {
            rt.run_async_with_requested_stack_with(spec, options, move || {
                std::future::poll_fn(move |_| {
                    worker_poll_started.store(true, Ordering::SeqCst);
                    // nosemgrep: taskmesh-test-thread-sleep -- reason: the poll must overrun the absolute deadline by wall-clock time; that overrun is the property under test.
                    std::thread::sleep(Duration::from_millis(120));
                    std::task::Poll::Ready(Ok::<(), ()>(()))
                })
            })
            .await
        }
    });

    bounded(
        "requested_stack_absolute_deadline_holds_permit_during_long_poll_v1: the worker never started polling",
        async {
            while !poll_started.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(
        rt.snapshot().classes[&TaskClass::new("c")].inflight,
        1,
        "a long synchronous poll must retain its permit after the deadline instant"
    );

    let error = submission
        .await
        .expect("submission task joins")
        .expect_err("completion after the absolute deadline must fail closed");
    assert!(matches!(
        error,
        RunError::Governor(GovernorError::DeadlineExceeded)
    ));
    // D10: the deadline answer is sent *before* the owned runtime is torn
    // down, and the lease travels with the teardown. So the caller can hold
    // the error while the worker is still charged for a moment; the contract
    // is that it drains, not that it is already zero.
    assert_drains(&rt, "c", "requested-stack deadline teardown").await;
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

    // Bounded: a host that refuses the stack request (or never charges it)
    // must fail this test by name, not hang the binary — a hang is not an
    // attributable verdict for the mutation harness (D16).
    bounded(
        "requested_stack_async_runtime_honors_cooperative_cancel_v1: the work was never charged to the class",
        async {
            while rt.snapshot().classes[&TaskClass::new("c")].inflight != 1 {
                tokio::task::yield_now().await;
            }
        },
    )
    .await;
    token.cancel();
    let error = worker
        .await
        .expect("caller task must complete")
        .expect_err("requested-stack async cancellation must stop the owned root");

    assert!(matches!(
        error,
        RunError::Governor(GovernorError::Cancelled)
    ));
    // As for the deadline: the terminal reply precedes the owned runtime's
    // teardown (D10), so drain rather than assert an instant zero.
    assert_drains(&rt, "c", "requested-stack cancel teardown").await;
}

// ---- F: run_local honors cooperative cancel (was previously excluded) ------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_local_honors_cooperative_cancel() {
    let rt = rt(CancellationPolicy::Cooperative);
    let token = CancellationToken::new();
    let opts = SubmitOptions::unbounded().with_cancel(token.clone());
    let spec = TaskSpec::local(TaskClass::new("c")).operation("op");

    // run_local's future is !Send, so it stays on this task. A spawned task
    // waits for the local future's first poll before firing cancellation.
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let firer = {
        let token = token.clone();
        tokio::spawn(async move {
            bounded(
                "run_local_honors_cooperative_cancel: local work was never polled",
                started_rx,
            )
            .await
            .expect("local work start sender survives");
            token.cancel();
        })
    };
    let err = rt
        .run_local_with(spec, opts, async {
            let _rc = std::rc::Rc::new(()); // !Send, proves the local path
            started_tx
                .send(())
                .expect("cancellation task observes the local work poll");
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
    accepted: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl CpuExecutor for StallExecutor {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        self.held.lock().unwrap().push(work);
        if let Some(accepted) = self.accepted.lock().unwrap().take() {
            accepted
                .send(())
                .expect("test observes stalled executor custody");
        }
    }

    fn capabilities(&self) -> ExecutorCapabilities {
        ExecutorCapabilities::legacy()
            .nonblocking_submit(true)
            .declared_workers(1)
            .physical_domain(PHYSICAL_CPU)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_cpu_escapes_stalled_executor_via_cancel() {
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let executor = Arc::new(StallExecutor {
        held: Mutex::new(Vec::new()),
        accepted: Mutex::new(Some(accepted_tx)),
    });
    let rt = Builder::new()
        .topology(TopologyConfig::new().cpu_fixed(1))
        .resources(ResourceBudget::new().cpu_units(64).memory_units(64))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(4)
                .cpu_units(1)
                .cancellation_policy(CancellationPolicy::Cooperative),
        )
        .cpu_executor(executor.clone())
        .build()
        .unwrap();

    let token = CancellationToken::new();
    let opts = SubmitOptions::unbounded().with_cancel(token.clone());
    let spec = TaskSpec::cpu(TaskClass::new("c")).operation("op");
    let rt2 = rt.clone();
    let handle =
        tokio::spawn(async move { rt2.run_cpu_with(spec, opts, || Ok::<i32, ()>(1)).await });

    bounded(
        "run_cpu_escapes_stalled_executor_via_cancel: executor never accepted work",
        accepted_rx,
    )
    .await
    .expect("stalled executor acceptance sender survives");
    token.cancel();
    let err = handle
        .await
        .unwrap()
        .expect_err("must escape the stalled executor");
    assert!(matches!(err, RunError::Governor(GovernorError::Cancelled)));
    tokio::task::yield_now().await;
    assert_eq!(
        rt.snapshot().classes[&TaskClass::new("c")].inflight,
        1,
        "cancelling the caller must not make queued CPU capacity appear free"
    );

    executor.held.lock().unwrap().clear();
    tokio::time::timeout(Duration::from_secs(1), async {
        while rt.snapshot().classes[&TaskClass::new("c")].inflight != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("permit must release after the executor drops the queued work");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_deadline_that_does_not_fit_the_clock_is_refused_before_the_work_is_polled() {
    // `with_deadline(Duration::MAX)` reads like "no deadline" but is a bound
    // the host cannot represent as an instant. The honest answers are a typed
    // refusal or a real deadline — never a silent "unbounded", and never a
    // panic from `Instant + Duration`. The refusal must happen before the work
    // is polled, and must leave nothing charged.
    let rt = rt(CancellationPolicy::CooperativeWithDeadline);
    let polled = Arc::new(AtomicBool::new(false));
    let spec = TaskSpec::io(TaskClass::new("c")).operation("op");
    let opts = SubmitOptions::unbounded().with_deadline(Duration::MAX);
    let error = bounded(
        "a_run_deadline_that_does_not_fit_the_clock_is_refused_before_the_work_is_polled: no answer",
        rt.run_io_with(spec, opts, {
            let polled = Arc::clone(&polled);
            async move {
                polled.store(true, Ordering::SeqCst);
                Ok::<(), ()>(())
            }
        }),
    )
    .await
    .expect_err("an unrepresentable deadline is refused, not dropped");
    let RunError::Governor(GovernorError::PolicyViolation(message)) = &error else {
        panic!("expected a typed policy violation, got {error:?}");
    };
    assert!(
        message.contains("exceeds Instant range"),
        "the refusal names the reason: {message}"
    );
    assert!(
        !polled.load(Ordering::SeqCst),
        "a refused deadline must not have started the work"
    );
    assert_drains(&rt, "c", "unrepresentable deadline refusal").await;
}
