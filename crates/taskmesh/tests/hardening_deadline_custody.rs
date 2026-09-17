//! Deadline and custody regressions (H16-012).
//!
//! * **TM16-002** — a `RunFor` budget on the blocking path was accepted and then
//!   ignored: the class advertised `CooperativeWithDeadline`, the option was
//!   taken, and a 60ms job under a 5ms budget returned `Ok`.
//! * **TM16-022** — the relative budget was armed *after* `CpuExecutor::spawn`
//!   returned. An adapter that runs work inline returns after the job is done,
//!   so the timer started when the job had already overrun, and a late result
//!   was reported as success.
//! * **TM16-032** — the acquisition budget covered the asynchronous queue wait
//!   but not the synchronous wait on the admission mutex. An `Admitted` that
//!   arrived after the budget expired started the caller's work anyway.
//! * **TM16-015** — a requested-stack worker sent its result and released its
//!   lease afterwards, so a caller could observe completion while the capacity
//!   was still charged.
//!
//! Throughout: a caller's answer and the work's custody are different events.
//! These tests assert both halves — the answer arrives on time, *and* the
//! capacity is released exactly once, when the work is really finished.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use taskmesh::ext::*;
use taskmesh::*;

const STACK: u64 = 2 * 1024 * 1024;

/// How long a bounded-wait assertion waits before declaring a hang. Far above
/// any budget under test, far below "the test binary never finishes".
const HANG: Duration = Duration::from_secs(5);

/// Releases a held worker when dropped, so a test that *fails* (or panics in
/// a later assertion) still lets the worker finish and the runtime tear down.
/// A hang must surface as an assertion, never as a test binary that never
/// exits — a failure the CI runner cannot attribute to anything.
struct ReleaseOnDrop(Option<std::sync::mpsc::Sender<()>>);

impl ReleaseOnDrop {
    fn release(mut self) {
        if let Some(tx) = self.0.take() {
            tx.send(()).expect("worker still waiting");
        }
    }
}

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            // The worker may already have gone (test passed and released): an
            // undeliverable release is the expected steady state here.
            let _worker_may_be_gone = tx.send(());
        }
    }
}

/// Await `future` for at most [`HANG`]; a longer wait is reported as the named
/// regression, not endured.
async fn bounded<F: std::future::Future>(what: &str, future: F) -> F::Output {
    let Ok(output) = tokio::time::timeout(HANG, future).await else {
        panic!("{what}: the caller was still waiting after {HANG:?}");
    };
    output
}

fn deadline_runtime(slots: usize) -> TokioRuntime {
    Builder::new()
        .topology(
            TopologyConfig::new()
                .blocking_threads(slots)
                .large_stack_slots(slots),
        )
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(8)
                .max_queue_depth(8)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth)
                .cancellation_policy(CancellationPolicy::CooperativeWithDeadline),
        )
        .build()
        .expect("runtime builds")
}

async fn assert_drains(rt: &TokioRuntime, what: &str) {
    let class = TaskClass::new("c");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let snapshot = rt.snapshot();
        let observed = &snapshot.classes[&class];
        if observed.inflight == 0 && observed.queued == 0 {
            assert_eq!(snapshot.conservation_violation(), None, "{what}");
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: never drained (inflight {}, queued {})",
            observed.inflight,
            observed.queued
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

// ---- TM16-002: a blocking run budget is honored for the caller -------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_blocking_run_budget_bounds_the_caller_and_keeps_the_worker_charged() {
    let rt = deadline_runtime(2);
    let finished = Arc::new(AtomicBool::new(false));
    let (release, hold) = std::sync::mpsc::channel::<()>();
    let release = ReleaseOnDrop(Some(release));
    let worker_finished = Arc::clone(&finished);

    let started = Instant::now();
    // Bounded: a regression here (budget ignored on the blocking path) would
    // otherwise wait on the held worker forever.
    let error = bounded(
        "TM16-002 blocking run budget",
        rt.run_blocking_with(
            TaskSpec::blocking(TaskClass::new("c")).operation("long"),
            SubmitOptions::unbounded().with_deadline(Duration::from_millis(20)),
            move || {
                // Runs until explicitly released: a started blocking job cannot
                // be aborted, and this test does not pretend otherwise.
                let _released = hold.recv();
                worker_finished.store(true, Ordering::SeqCst);
                Ok::<i32, ()>(7)
            },
        ),
    )
    .await
    .expect_err("the run budget must bound the caller's wait");
    let waited = started.elapsed();

    assert!(
        matches!(error, RunError::Governor(GovernorError::DeadlineExceeded)),
        "got {error:?}"
    );
    assert!(
        waited < Duration::from_secs(5),
        "the caller waited {waited:?} for a 20ms budget"
    );
    assert!(
        !finished.load(Ordering::SeqCst),
        "the worker is still running: the deadline bounded the wait, not the work"
    );
    // And it is still charged, because it is still using the capacity.
    assert_eq!(
        rt.snapshot().classes[&TaskClass::new("c")].inflight,
        1,
        "a timed-out caller must not free capacity a live worker holds"
    );

    release.release();
    assert_drains(&rt, "blocking run budget").await;
    assert!(finished.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_blocking_job_inside_its_budget_still_succeeds() {
    // Positive control: the budget is enforced, not universally fatal.
    let rt = deadline_runtime(2);
    let out: i32 = rt
        .run_blocking_with(
            TaskSpec::blocking(TaskClass::new("c")).operation("quick"),
            SubmitOptions::unbounded().with_deadline(Duration::from_secs(30)),
            || Ok::<_, ()>(11),
        )
        .await
        .expect("work inside its budget succeeds");
    assert_eq!(out, 11);
    assert_drains(&rt, "blocking control").await;
}

// ---- TM16-022: the budget is anchored to the worker, not to `spawn` ---------

/// A `CpuExecutor` that runs work inline: `spawn` returns only after the job is
/// finished. Legal — the port does not promise otherwise — and it is exactly the
/// shape that made "start the timer after spawn" meaningless.
struct InlineCpuExecutor;

impl CpuExecutor for InlineCpuExecutor {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        work();
    }

    fn capabilities(&self) -> ExecutorCapabilities {
        ExecutorCapabilities::legacy()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_inline_executor_cannot_report_a_late_result_as_success() {
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(8)
                .cpu_units(1)
                .cancellation_policy(CancellationPolicy::CooperativeWithDeadline),
        )
        .cpu_executor(Arc::new(InlineCpuExecutor))
        .build()
        .expect("runtime builds");

    let error = rt
        .run_cpu_with(
            TaskSpec::cpu(TaskClass::new("c")).operation("overrun"),
            SubmitOptions::unbounded().with_deadline(Duration::from_millis(5)),
            || {
                // nosemgrep: taskmesh-test-thread-sleep -- reason: the job must overrun its RunFor budget by wall-clock time on an inline executor; that overrun is the property under test.
                std::thread::sleep(Duration::from_millis(60));
                Ok::<i32, ()>(7)
            },
        )
        .await
        .expect_err("a job that took 60ms under a 5ms budget did not meet it");
    assert!(
        matches!(error, RunError::Governor(GovernorError::DeadlineExceeded)),
        "got {error:?}"
    );
    assert_drains(&rt, "inline executor overrun").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_inline_executor_inside_its_budget_succeeds() {
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(8)
                .cpu_units(1)
                .cancellation_policy(CancellationPolicy::CooperativeWithDeadline),
        )
        .cpu_executor(Arc::new(InlineCpuExecutor))
        .build()
        .expect("runtime builds");

    let out: i32 = rt
        .run_cpu_with(
            TaskSpec::cpu(TaskClass::new("c")).operation("quick"),
            SubmitOptions::unbounded().with_deadline(Duration::from_secs(30)),
            || Ok::<_, ()>(3),
        )
        .await
        .expect("inside budget");
    assert_eq!(out, 3);
    assert_drains(&rt, "inline executor control").await;
}

// ---- TM16-032: an expired acquisition budget starts nothing ----------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_expired_acquisition_budget_does_not_start_work_it_could_admit() {
    // Capacity is free, so admission returns `Admitted` immediately — the case
    // the original deadline check skipped entirely. The budget is already spent
    // by the time that answer comes back, and a caller who asked not to wait
    // past it must not get the side effects of the work.
    let rt = deadline_runtime(4);
    let ran = Arc::new(AtomicBool::new(false));
    let job_ran = Arc::clone(&ran);

    let error = rt
        .run_blocking_with(
            TaskSpec::blocking(TaskClass::new("c")).operation("expired"),
            SubmitOptions::unbounded().with_acquire_timeout(Duration::from_nanos(1)),
            move || {
                job_ran.store(true, Ordering::SeqCst);
                Ok::<i32, ()>(1)
            },
        )
        .await
        .expect_err("an expired acquisition budget must not start work");
    assert!(
        matches!(
            error,
            RunError::Governor(GovernorError::Rejected(
                AdmissionVerdict::PermitAcquireTimedOut { .. }
            ))
        ),
        "got {error:?}"
    );
    assert!(!ran.load(Ordering::SeqCst), "the job must not have started");
    // The permit taken to discover this is handed straight back.
    assert_drains(&rt, "expired acquisition budget").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_zero_acquire_timeout_still_takes_free_capacity() {
    // `ZERO` means *try*, and it kept that meaning: an uncontended acquisition
    // succeeds. Treating it as "already expired" would break every caller using
    // it as a non-blocking submit.
    let rt = deadline_runtime(4);
    let out: i32 = rt
        .run_blocking_with(
            TaskSpec::blocking(TaskClass::new("c")).operation("try"),
            SubmitOptions::unbounded().with_acquire_timeout(Duration::ZERO),
            || Ok::<_, ()>(4),
        )
        .await
        .expect("free capacity satisfies a try-acquire");
    assert_eq!(out, 4);
    assert_drains(&rt, "zero acquire timeout").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_generous_acquisition_budget_admits_normally() {
    let rt = deadline_runtime(4);
    let out: i32 = rt
        .run_blocking_with(
            TaskSpec::blocking(TaskClass::new("c")).operation("normal"),
            SubmitOptions::unbounded().with_acquire_timeout(Duration::from_secs(30)),
            || Ok::<_, ()>(5),
        )
        .await
        .expect("a generous budget admits");
    assert_eq!(out, 5);
    assert_drains(&rt, "generous acquisition budget").await;
}

// ---- TM16-015: a delivered result has already released its capacity --------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_requested_stack_result_is_delivered_after_its_lease_is_released() {
    // Custody travels with the result, so the instant the caller can see the
    // value the capacity is already back. Without the fence the two racing
    // events made an immediate snapshot report phantom inflight work.
    let rt = deadline_runtime(1);
    for round in 0..20 {
        let out: i32 = rt
            .run_blocking(
                TaskSpec::blocking(TaskClass::new("c"))
                    .operation(format!("fenced{round}"))
                    .stack_size_bytes(STACK),
                move || Ok::<_, ()>(round),
            )
            .await
            .expect("stack worker completes");
        assert_eq!(out, round);
        // Immediately — no yield, no sleep, no retry loop.
        let snapshot = rt.snapshot();
        assert_eq!(
            snapshot.classes[&TaskClass::new("c")].inflight,
            0,
            "round {round}: a delivered result must not still be charged"
        );
        assert_eq!(snapshot.capabilities["large_stack"].in_use, 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sequential_try_acquisitions_never_race_the_previous_release() {
    // The same fence from the caller's side: with one slot and a non-waiting
    // acquire, the next submission can only succeed if the previous release has
    // already happened by the time its result was observed.
    let rt = deadline_runtime(1);
    for round in 0..20 {
        let out: i32 = rt
            .run_blocking_with(
                TaskSpec::blocking(TaskClass::new("c"))
                    .operation(format!("seq{round}"))
                    .stack_size_bytes(STACK),
                SubmitOptions::unbounded().with_acquire_timeout(Duration::ZERO),
                move || Ok::<_, ()>(round),
            )
            .await
            .unwrap_or_else(|error| panic!("round {round} must acquire immediately: {error:?}"));
        assert_eq!(out, round);
    }
}

// ---- TM16-024: a terminal answer is not blocked by cleanup -----------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_deadline_answer_is_not_delayed_by_a_blocking_child_teardown() {
    // A requested-stack async root yields to a blocking child on its owned
    // runtime. Dropping that runtime waits for the child — Tokio cannot abort a
    // started blocking task — so delivering the deadline answer *after* teardown
    // made the caller wait for the child. The answer now comes first; the lease
    // stays with the worker until teardown really finishes.
    let rt = deadline_runtime(2);
    let child_running = Arc::new(AtomicUsize::new(0));
    let child_done = Arc::new(AtomicBool::new(false));
    let (release, hold) = std::sync::mpsc::channel::<()>();
    let release = ReleaseOnDrop(Some(release));

    let running = Arc::clone(&child_running);
    let done = Arc::clone(&child_done);
    let started = Instant::now();
    // Bounded: a regression here (answer delivered after teardown) would
    // otherwise wait on the held child forever.
    let error = bounded(
        "TM16-024 deadline answer before teardown",
        rt.run_async_with_requested_stack_with(
            TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
                .operation("root-with-blocking-child")
                .stack_size_bytes(STACK),
            SubmitOptions::unbounded().with_deadline(Duration::from_millis(20)),
            move || async move {
                let _child = tokio::task::spawn_blocking(move || {
                    running.fetch_add(1, Ordering::SeqCst);
                    let _released = hold.recv();
                    done.store(true, Ordering::SeqCst);
                });
                // The root itself stays cooperative; it is the child that pins
                // the runtime.
                std::future::pending::<Result<(), ()>>().await
            },
        ),
    )
    .await
    .expect_err("the root's deadline must fire");
    let answered_in = started.elapsed();

    assert!(
        matches!(error, RunError::Governor(GovernorError::DeadlineExceeded)),
        "got {error:?}"
    );
    assert!(
        answered_in < Duration::from_secs(5),
        "the deadline answer waited {answered_in:?} for the blocking child"
    );
    assert!(
        !child_done.load(Ordering::SeqCst),
        "the child is still running, which is precisely why the lease is still held"
    );
    assert_eq!(
        rt.snapshot().classes[&TaskClass::new("c")].inflight,
        1,
        "cleanup keeps the work charged"
    );

    release.release();
    assert_drains(&rt, "blocking child teardown").await;
    assert!(child_done.load(Ordering::SeqCst));
    assert_eq!(rt.snapshot().capabilities["large_stack"].in_use, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_panic_answer_is_not_delayed_by_a_blocking_child_teardown() {
    // The same custody rule for the *panic* path: a root future that panics
    // after spawning a blocking child must be reported to the caller before
    // its owned runtime is torn down (teardown waits for the child), and the
    // lease stays with the worker until that teardown really finishes.
    let rt = deadline_runtime(2);
    let child_done = Arc::new(AtomicBool::new(false));
    let (release, hold) = std::sync::mpsc::channel::<()>();
    let release = ReleaseOnDrop(Some(release));

    let done = Arc::clone(&child_done);
    let started = Instant::now();
    let error = bounded(
        "panic answer before teardown",
        rt.run_async_with_requested_stack(
            TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
                .operation("root-panics-with-blocking-child")
                .stack_size_bytes(STACK),
            move || async move {
                let _child = tokio::task::spawn_blocking(move || {
                    let _released = hold.recv();
                    done.store(true, Ordering::SeqCst);
                });
                tokio::task::yield_now().await;
                panic!("root future exploded");
                #[allow(unreachable_code, reason = "the panic is the point")]
                Ok::<(), ()>(())
            },
        ),
    )
    .await
    .expect_err("the root's panic must be reported");
    let answered_in = started.elapsed();

    assert_eq!(
        error,
        RunError::Governor(GovernorError::WorkerPanicked {
            context: "requested-stack async worker".into()
        })
    );
    assert!(
        answered_in < HANG,
        "the panic answer waited {answered_in:?} for the blocking child"
    );
    assert!(
        !child_done.load(Ordering::SeqCst),
        "the child is still running, which is why the lease is still held"
    );
    assert_eq!(
        rt.snapshot().classes[&TaskClass::new("c")].inflight,
        1,
        "cleanup keeps the work charged"
    );

    release.release();
    assert_drains(&rt, "blocking child teardown after panic").await;
    assert!(child_done.load(Ordering::SeqCst));
    assert_eq!(rt.snapshot().capabilities["large_stack"].in_use, 0);
}
