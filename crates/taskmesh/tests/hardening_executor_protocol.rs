//! Executor-boundary regressions (H16-011).
//!
//! * **TM16-031** — the OS thread label was built by interpolating the task
//!   class straight into `thread::Builder::name`. C01 now rejects non-canonical
//!   identifiers, while canonical punctuation and maximum-length identifiers
//!   still pass through the same bounded label derivation.
//!
//! The rest of this file pins the ownership protocol around that boundary: a job
//! runs at most once, each failure mode is reported as itself, and a caller who
//! stops waiting does not take the worker's capacity with it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use taskmesh::ext::*;
use taskmesh::*;

const STACK: u64 = 1024 * 1024;

fn runtime_for(class: TaskClass) -> TokioRuntime {
    Builder::new()
        .topology(
            TopologyConfig::new()
                .large_stack_slots(2)
                .blocking_threads(2),
        )
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            class,
            ClassPolicy::new()
                .max_inflight(8)
                .max_queue_depth(8)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .expect("runtime builds")
}

async fn assert_drains(rt: &TokioRuntime, class: &TaskClass, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let snapshot = rt.snapshot();
        let observed = &snapshot.classes[class];
        if observed.inflight == 0 && observed.queued == 0 {
            assert_eq!(snapshot.conservation_violation(), None, "{what}");
            return;
        }
        assert!(Instant::now() < deadline, "{what}: never drained");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// Contract-valid class identifiers that still require OS-label normalization.
fn awkward_classes() -> Vec<(&'static str, TaskClass)> {
    vec![
        ("path-like", TaskClass::new("a/b:c")),
        ("maximum length", TaskClass::new("l".repeat(128))),
    ]
}

fn assert_requested_stack_label(what: &str, name: &str) {
    const PREFIX: &str = "taskmesh.large_stack:";
    assert!(
        name.starts_with(PREFIX),
        "{what}: unexpected worker label {name:?}"
    );
    assert!(
        name.len() <= 48,
        "{what}: worker label is not bounded: {name:?}"
    );
    match what {
        "path-like" => assert_eq!(name, "taskmesh.large_stack:a_b_c"),
        "maximum length" => assert_eq!(name.len(), 48),
        _ => panic!("unknown awkward class case {what}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_awkward_class_never_panics_a_requested_stack_submission() {
    for (what, class) in awkward_classes() {
        let rt = runtime_for(class.clone());
        let ran = Arc::new(AtomicUsize::new(0));
        let job_ran = Arc::clone(&ran);

        let result: Result<(i32, String), RunError<()>> = rt
            .run_blocking(
                TaskSpec::blocking(class.clone())
                    .operation("stack-worker")
                    .stack_size_bytes(STACK),
                move || {
                    job_ran.fetch_add(1, Ordering::SeqCst);
                    Ok((
                        9,
                        std::thread::current()
                            .name()
                            .expect("dedicated worker is named")
                            .to_owned(),
                    ))
                },
            )
            .await;
        let (value, name) = result.expect("accepted class must not break worker construction");
        assert_eq!(value, 9, "{what}");
        assert_requested_stack_label(what, &name);
        assert_eq!(ran.load(Ordering::SeqCst), 1, "{what}: ran exactly once");
        assert_drains(&rt, &class, what).await;

        // The async large-stack entry point builds a label the same way.
        let async_ran = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&async_ran);
        let result: Result<(i32, String), RunError<()>> = rt
            .run_async_with_requested_stack(
                TaskSpec::base(class.clone(), SubstrateHint::LargeStackCapability)
                    .operation("async-stack-worker")
                    .stack_size_bytes(STACK),
                move || async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok((
                        10,
                        std::thread::current()
                            .name()
                            .expect("dedicated worker is named")
                            .to_owned(),
                    ))
                },
            )
            .await;
        let (value, name) = result.expect("async large-stack path must run");
        assert_eq!(value, 10, "{what}");
        assert_requested_stack_label(what, &name);
        assert_eq!(async_ran.load(Ordering::SeqCst), 1, "{what}");
        assert_drains(&rt, &class, what).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn class_identity_is_not_rewritten_to_suit_an_os_label() {
    // The label is derived and lossy; the class itself is untouched. Sanitizing
    // the identity instead would silently merge two distinct classes into one
    // policy bucket.
    let class = TaskClass::new("a/b:c");
    let rt = runtime_for(class.clone());
    let out: i32 = rt
        .run_blocking(
            TaskSpec::blocking(class.clone())
                .operation("x")
                .stack_size_bytes(STACK),
            || Ok::<_, ()>(1),
        )
        .await
        .expect("runs");
    assert_eq!(out, 1);
    let snapshot = rt.snapshot();
    assert!(
        snapshot.classes.contains_key(&class),
        "the class key is preserved verbatim: {:?}",
        snapshot.classes.keys().collect::<Vec<_>>()
    );
    assert_eq!(class.as_str(), "a/b:c");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_stack_preflight_has_zero_governor_and_worker_side_effects() {
    let class = TaskClass::new("c");
    let rt = runtime_for(class.clone());

    // Hold a large-stack slot. An invalid waiter must not join the queue:
    // preflight completes before admission and before the caller's closure can
    // cross a worker boundary.
    let rt_holder = rt.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
    let holder = tokio::spawn(async move {
        rt_holder
            .run_blocking(
                TaskSpec::blocking(TaskClass::new("c"))
                    .operation("holder")
                    .stack_size_bytes(STACK),
                move || {
                    started_tx.send(()).expect("holder start observer is alive");
                    release_rx.recv().expect("holder receives release signal");
                    Ok::<(), ()>(())
                },
            )
            .await
    });
    started_rx.await.expect("holder reached its worker");

    let before = rt.snapshot();
    let ran = Arc::new(AtomicUsize::new(0));
    for (hint, requested, expected) in [
        (
            SubstrateHint::BlockingPool,
            0,
            "requested stack size must be nonzero".to_owned(),
        ),
        (
            SubstrateHint::LargeStackCapability,
            MAX_REQUESTED_STACK_BYTES + 1,
            format!(
                "requested stack size {} bytes exceeds the supported maximum {MAX_REQUESTED_STACK_BYTES}",
                MAX_REQUESTED_STACK_BYTES + 1
            ),
        ),
        (
            SubstrateHint::BackgroundOnly,
            MAX_REQUESTED_STACK_BYTES + 1,
            format!(
                "requested stack size {} bytes exceeds the supported maximum {MAX_REQUESTED_STACK_BYTES}",
                MAX_REQUESTED_STACK_BYTES + 1
            ),
        ),
    ] {
        let job_ran = Arc::clone(&ran);
        let error = rt
            .run_blocking(
                TaskSpec::base(class.clone(), hint)
                    .operation(format!("invalid-{hint:?}-{requested}"))
                    .stack_size_bytes(requested),
                move || {
                    job_ran.fetch_add(1, Ordering::SeqCst);
                    Ok::<(), ()>(())
                },
            )
            .await
            .expect_err("invalid stack must fail in preflight");
        assert_eq!(
            error,
            RunError::Governor(GovernorError::PolicyViolation(expected.into())),
            "{hint:?}/{requested}"
        );
        assert_eq!(rt.snapshot(), before, "{hint:?}/{requested}");
    }

    let async_error = rt
        .run_async_with_requested_stack(
            TaskSpec::base(class.clone(), SubstrateHint::LargeStackCapability)
                .operation("invalid-async")
                .stack_size_bytes(0),
            || async { Ok::<(), ()>(()) },
        )
        .await
        .expect_err("async stack uses the same validator");
    assert_eq!(
        async_error,
        RunError::Governor(GovernorError::PolicyViolation(
            "requested stack size must be nonzero".into()
        ))
    );
    assert_eq!(rt.snapshot(), before, "async invalid request is invisible");
    assert_eq!(ran.load(Ordering::SeqCst), 0, "no invalid job ran");

    release_tx.send(()).expect("holder still owns its worker");
    holder
        .await
        .expect("holder task joins")
        .expect("holder runs");
    assert_drains(&rt, &class, "invalid preflight fixture").await;
}

#[tokio::test]
async fn contract_shape_precedes_dispatch_and_stack_validation() {
    let class = TaskClass::new("c");
    let rt = runtime_for(class.clone());
    let before = rt.snapshot();
    let error = rt
        .run_io(
            TaskSpec::blocking(class)
                // Empty operation/root identity is a C01 shape error. The
                // wrong run path and absurd stack are deliberately present to
                // prove the documented precedence.
                .stack_size_bytes(MAX_REQUESTED_STACK_BYTES + 1),
            async { Ok::<(), ()>(()) },
        )
        .await
        .expect_err("shape validation is first");
    assert_eq!(
        error,
        RunError::Governor(GovernorError::Rejected(AdmissionVerdict::MalformedTask))
    );
    assert_eq!(rt.snapshot(), before);
}

// ---- ownership protocol ----------------------------------------------------

/// Jobs an adapter has taken custody of but not yet run.
type HeldJobs = Arc<std::sync::Mutex<Vec<Box<dyn FnOnce() + Send + 'static>>>>;

/// An adapter that accepts the job and then panics, modelling an executor that
/// fails *after* taking custody.
struct PanicAfterEnqueueExecutor {
    enqueued: HeldJobs,
}

struct DroppingExecutor;

impl CpuExecutor for DroppingExecutor {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        drop(work);
    }

    fn capabilities(&self) -> ExecutorCapabilities {
        ExecutorCapabilities::legacy()
            .nonblocking_submit(true)
            .declared_workers(1)
            .physical_domain(PHYSICAL_CPU)
    }
}

impl CpuExecutor for PanicAfterEnqueueExecutor {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        self.enqueued.lock().expect("lock").push(work);
        panic!("executor failed after taking the job");
    }

    fn capabilities(&self) -> ExecutorCapabilities {
        ExecutorCapabilities::legacy()
            .nonblocking_submit(true)
            .declared_workers(1)
            .physical_domain(PHYSICAL_CPU)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_executor_that_panics_after_accepting_does_not_run_the_job_twice() {
    // The adapter already owns the closure, so re-submitting would be a second
    // execution of the caller's work. The host reports and stops instead.
    let enqueued = Arc::new(std::sync::Mutex::new(Vec::new()));
    let rt = Builder::new()
        .topology(TopologyConfig::new().cpu_fixed(1))
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new().max_inflight(8).cpu_units(1),
        )
        .cpu_executor(Arc::new(PanicAfterEnqueueExecutor {
            enqueued: Arc::clone(&enqueued),
        }))
        .build()
        .expect("runtime builds");

    let ran = Arc::new(AtomicUsize::new(0));
    let job_ran = Arc::clone(&ran);
    let error = rt
        .run_cpu(
            TaskSpec::cpu(TaskClass::new("c")).operation("once"),
            move || {
                job_ran.fetch_add(1, Ordering::SeqCst);
                Ok::<i32, ()>(1)
            },
        )
        .await
        .expect_err("a panicking adapter is a governor-side failure");
    assert!(
        matches!(
            &error,
            RunError::Governor(GovernorError::WorkerPanicked { context })
                if context == "cpu executor spawn"
        ),
        "the adapter's own spawn panicked; got {error:?}"
    );
    assert_eq!(
        ran.load(Ordering::SeqCst),
        0,
        "the job must not have been run by the host as well"
    );

    // Exactly one closure is held by the adapter. Running it now releases the
    // lease it still carries — custody was never duplicated.
    let held = {
        let mut queue = enqueued.lock().expect("lock");
        assert_eq!(queue.len(), 1, "the adapter took the job exactly once");
        queue.pop().expect("one job")
    };
    assert_eq!(
        rt.snapshot().classes[&TaskClass::new("c")].inflight,
        1,
        "the adapter still holds the work, so it is still charged"
    );
    held();
    assert_eq!(ran.load(Ordering::SeqCst), 1, "and it runs exactly once");
    assert_drains(&rt, &TaskClass::new("c"), "panicking adapter").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_executor_that_drops_an_accepted_job_reports_abandonment_and_refunds_custody() {
    let class = TaskClass::new("c");
    let rt = Builder::new()
        .topology(TopologyConfig::new().cpu_fixed(1))
        .resources(ResourceBudget::new().cpu_units(1).memory_units(1))
        .class_policy(
            class.clone(),
            ClassPolicy::new().max_inflight(1).cpu_units(1),
        )
        .cpu_executor(Arc::new(DroppingExecutor))
        .build()
        .expect("runtime builds");
    let ran = Arc::new(AtomicUsize::new(0));
    let ran_in_work = Arc::clone(&ran);

    let error = rt
        .run_cpu(
            TaskSpec::cpu(class.clone()).operation("dropped"),
            move || {
                ran_in_work.fetch_add(1, Ordering::SeqCst);
                Ok::<_, ()>(())
            },
        )
        .await
        .expect_err("dropping accepted custody must be reported");
    assert!(matches!(
        error,
        RunError::Governor(GovernorError::JobAbandoned { .. })
    ));
    assert_eq!(ran.load(Ordering::SeqCst), 0);
    assert_drains(&rt, &class, "dropped accepted job").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_failure_mode_is_reported_as_itself() {
    let class = TaskClass::new("c");
    let rt = runtime_for(class.clone());

    // A task error keeps the caller's error type intact.
    let error = rt
        .run_blocking(
            TaskSpec::blocking(class.clone()).operation("task-error"),
            || Err::<i32, &'static str>("domain failure"),
        )
        .await
        .expect_err("task error");
    assert_eq!(error.task(), Some(&"domain failure"));
    assert!(!error.is_governor(), "a task error is not a governor error");

    // A worker panic is a governor-side failure, not a task error, and does not
    // abort the process. It is reported as a *panic*, naming the worker.
    let error = rt
        .run_blocking(
            TaskSpec::blocking(class.clone()).operation("panic"),
            || -> Result<i32, ()> { panic!("worker exploded") },
        )
        .await
        .expect_err("worker panic");
    assert_eq!(
        error,
        RunError::Governor(GovernorError::WorkerPanicked {
            context: "blocking worker".into()
        })
    );

    // Same again on the dedicated-thread path.
    let error = rt
        .run_blocking(
            TaskSpec::blocking(class.clone())
                .operation("stack-panic")
                .stack_size_bytes(STACK),
            || -> Result<i32, ()> { panic!("stack worker exploded") },
        )
        .await
        .expect_err("stack worker panic");
    assert_eq!(
        error,
        RunError::Governor(GovernorError::WorkerPanicked {
            context: "large-stack worker".into()
        })
    );

    // An absurd stack request is a caller error and is refused by the host
    // *before* `std::thread` sees it — deterministically, on every platform and
    // every build of `std`. (Handing `u64::MAX` to `Builder::stack_size` used
    // to be the test for "the OS refuses": under a debug-built `std`, as with
    // `-Zbuild-std -Zsanitizer=thread`, that arithmetic overflows and panics
    // inside the spawn instead. Environment-dependent behaviour is not a
    // contract.) The job never starts.
    let ran = Arc::new(AtomicUsize::new(0));
    let job_ran = Arc::clone(&ran);
    let error = rt
        .run_blocking(
            TaskSpec::blocking(class.clone())
                .operation("absurd-stack")
                .stack_size_bytes(MAX_REQUESTED_STACK_BYTES + 1),
            move || -> Result<i32, ()> {
                job_ran.fetch_add(1, Ordering::SeqCst);
                Ok(1)
            },
        )
        .await
        .expect_err("an absurd stack request must be refused");
    let oversized = format!(
        "requested stack size {} bytes exceeds the supported maximum {MAX_REQUESTED_STACK_BYTES}",
        MAX_REQUESTED_STACK_BYTES + 1
    );
    assert_eq!(
        error,
        RunError::Governor(GovernorError::PolicyViolation(oversized.clone().into()))
    );
    assert_eq!(ran.load(Ordering::SeqCst), 0, "the job never started");
    // Exactly the maximum is *not* refused by this check (whether the OS can
    // actually provide 16 GiB is the OS's answer — `WorkerUnavailable` — and
    // is not asserted here because it is not deterministic across hosts).
    let boundary = rt
        .run_blocking(
            TaskSpec::blocking(class.clone())
                .operation("max-stack")
                .stack_size_bytes(MAX_REQUESTED_STACK_BYTES),
            || -> Result<i32, ()> { Ok(1) },
        )
        .await;
    match boundary {
        Ok(1) => {}
        Err(RunError::Governor(GovernorError::WorkerUnavailable { context, detail })) => {
            assert_eq!(context, "large-stack worker");
            assert!(!detail.is_empty(), "the OS reason travels with the error");
        }
        other => panic!("the maximum is either honored or refused by the OS, got {other:?}"),
    }
    // The same refusal on the requested-stack async path.
    let error = rt
        .run_async_with_requested_stack(
            TaskSpec::base(class.clone(), SubstrateHint::LargeStackCapability)
                .operation("absurd-stack-async")
                .stack_size_bytes(MAX_REQUESTED_STACK_BYTES + 1),
            || async { Ok::<i32, ()>(1) },
        )
        .await
        .expect_err("an absurd async stack request must be refused");
    assert_eq!(
        error,
        RunError::Governor(GovernorError::PolicyViolation(oversized.into()))
    );

    assert_drains(&rt, &class, "failure modes").await;
}

/// An adapter that takes custody of the job and holds it: the work is
/// `Accepted` but not `Running` until the test says so.
struct HoldingExecutor {
    held: HeldJobs,
}

impl CpuExecutor for HoldingExecutor {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        self.held.lock().expect("lock").push(work);
    }

    fn capabilities(&self) -> ExecutorCapabilities {
        ExecutorCapabilities::legacy()
            .nonblocking_submit(true)
            .declared_workers(1)
            .physical_domain(PHYSICAL_CPU)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_caller_that_leaves_while_the_job_is_accepted_but_not_started_keeps_it_charged() {
    // D01/D10 at the *Accepted* phase: the executor owns the closure but has
    // not run it. The caller drops. The permit must stay — the closure still
    // exists and will run — with the gauges saying `accepted`, not `running`,
    // and the eventual run refunds exactly once.
    let class = TaskClass::new("c");
    let held: HeldJobs = Arc::new(std::sync::Mutex::new(Vec::new()));
    let rt = Builder::new()
        .topology(TopologyConfig::new().cpu_fixed(1))
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            class.clone(),
            ClassPolicy::new().max_inflight(8).cpu_units(1),
        )
        .cpu_executor(Arc::new(HoldingExecutor {
            held: Arc::clone(&held),
        }))
        .build()
        .expect("runtime builds");

    let ran = Arc::new(AtomicUsize::new(0));
    let job_ran = Arc::clone(&ran);
    let mut submission = Box::pin(rt.run_cpu(
        TaskSpec::cpu(class.clone()).operation("held"),
        move || {
            job_ran.fetch_add(1, Ordering::SeqCst);
            Ok::<i32, ()>(1)
        },
    ));
    assert!(
        futures_poll_once(&mut submission).await.is_none(),
        "the job is held, so the caller is pending"
    );
    let snapshot = rt.snapshot();
    let observed = &snapshot.classes[&class];
    assert_eq!(
        (observed.inflight, observed.accepted, observed.running),
        (1, 1, 0),
        "custody is with the executor: accepted, not running"
    );
    assert_eq!(snapshot.capabilities["cpu"].in_use, 1);

    drop(submission);
    let snapshot = rt.snapshot();
    let observed = &snapshot.classes[&class];
    assert_eq!(
        (observed.inflight, observed.accepted),
        (1, 1),
        "the caller left; the accepted job still holds its permit"
    );
    // Nor can anyone holding the governor end it by id: the permit left
    // `DispatchReserved`, so the lease inside the held closure owns it (D14).
    let permit = rt.governor().permit_ledgers()[0].permit_id;
    assert_eq!(
        rt.governor().release(permit),
        ReleaseOutcome::HeldByLease {
            phase: ExecutionPhase::Accepted
        },
        "an accepted job's permit is its lease's, not the governor caller's"
    );
    assert_eq!(
        rt.snapshot(),
        snapshot,
        "the refused release changed nothing"
    );

    // The executor finally runs it: one execution, one refund.
    let job = held.lock().expect("lock").pop().expect("the held job");
    job();
    assert_eq!(ran.load(Ordering::SeqCst), 1);
    assert_drains(&rt, &class, "accepted-then-abandoned").await;
    assert_eq!(rt.snapshot().capabilities["cpu"].in_use, 0);
    let snapshot = rt.snapshot();
    let observed = &snapshot.classes[&class];
    assert_eq!(observed.admitted_total, 1);
    assert_eq!(observed.started_total, 1);
    assert_eq!(observed.terminated_total, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn accepted_unstarted_caller_exit_keeps_follower_queued_until_worker_settles() {
    use tokio_util::sync::CancellationToken;

    for cancel_caller in [true, false] {
        let class = TaskClass::new("c");
        let held: HeldJobs = Arc::new(std::sync::Mutex::new(Vec::new()));
        let rt = Builder::new()
            .topology(TopologyConfig::new().cpu_fixed(1))
            .resources(ResourceBudget::new().cpu_units(1).memory_units(1))
            .class_policy(
                class.clone(),
                ClassPolicy::new()
                    .max_inflight(1)
                    .max_queue_depth(1)
                    .cpu_units(1)
                    .overflow_policy(OverflowPolicy::QueueWithinDepth)
                    .cancellation_policy(CancellationPolicy::Cooperative),
            )
            .cpu_executor(Arc::new(HoldingExecutor {
                held: Arc::clone(&held),
            }))
            .build()
            .expect("one-slot held executor builds");
        let cancel = CancellationToken::new();
        let owner = rt.clone();
        let token = cancel.clone();
        let mut first = tokio::spawn(async move {
            owner
                .run_cpu_with(
                    TaskSpec::cpu(TaskClass::new("c")).operation("accepted-holder"),
                    SubmitOptions::unbounded().with_cancel(token),
                    || Ok::<_, ()>(1),
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while rt.snapshot().classes[&class].accepted != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("first closure becomes accepted without starting");
        assert_eq!(held.lock().expect("lock").len(), 1);

        let follower_rt = rt.clone();
        let follower = tokio::spawn(async move {
            follower_rt
                .run_cpu(
                    TaskSpec::cpu(TaskClass::new("c")).operation("queued-follower"),
                    || Ok::<_, ()>(2),
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while rt.snapshot().classes[&class].queued != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("follower queues behind accepted closure");

        if cancel_caller {
            cancel.cancel();
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(5), &mut first)
                    .await
                    .expect("cancelled caller responds")
                    .expect("caller joins"),
                Err(RunError::Governor(GovernorError::Cancelled))
            ));
        } else {
            first.abort();
            assert!(tokio::time::timeout(Duration::from_secs(5), &mut first)
                .await
                .expect("dropped caller settles")
                .expect_err("dropped caller has no response")
                .is_cancelled());
        }

        let pending = rt.snapshot();
        assert_eq!(
            (
                pending.classes[&class].accepted,
                pending.classes[&class].queued
            ),
            (1, 1)
        );
        assert_eq!(pending.capabilities["cpu"].in_use, 1);
        assert_eq!(held.lock().expect("lock").len(), 1);
        let not_drained = rt
            .drain(Duration::ZERO)
            .await
            .expect_err("accepted closure and pre-drain follower retain custody");
        assert_eq!(
            (
                not_drained.classes[&class].inflight,
                not_drained.classes[&class].queued
            ),
            (1, 1)
        );

        let first_job = held.lock().expect("lock").pop().expect("first closure");
        first_job();
        tokio::time::timeout(Duration::from_secs(5), async {
            while rt.snapshot().classes[&class].accepted != 1
                || rt.snapshot().classes[&class].queued != 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("follower promotes and executor accepts it");
        let second_job = held.lock().expect("lock").pop().expect("second closure");
        second_job();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), follower)
                .await
                .expect("follower responds")
                .expect("follower joins"),
            Ok(2)
        );
        rt.drain(Duration::from_secs(5))
            .await
            .expect("both accepted closures have settled");
        let final_snapshot = rt.snapshot();
        let c = &final_snapshot.classes[&class];
        assert_eq!((c.inflight, c.queued, c.accepted), (0, 0, 0));
        assert_eq!(
            (c.admitted_total, c.started_total, c.terminated_total),
            (2, 2, 2)
        );
        assert_eq!(final_snapshot.capabilities["cpu"].in_use, 0);
        assert_eq!(final_snapshot.conservation_violation(), None);
    }
}

// ---- D05: the adapter's declaration is load-bearing ------------------------

/// An adapter that declares a worker count and runs work on plain threads.
struct DeclaringExecutor {
    workers: u32,
}

impl CpuExecutor for DeclaringExecutor {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        std::thread::spawn(work);
    }

    fn capabilities(&self) -> ExecutorCapabilities {
        ExecutorCapabilities::legacy()
            .nonblocking_submit(true)
            .declared_workers(self.workers)
            .exclusive_pool(true)
            .physical_domain(PHYSICAL_CPU)
    }
}

struct LegacyExecutor;

impl CpuExecutor for LegacyExecutor {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        work();
    }
}

/// An adversarial adapter whose declaration changes after installation.
/// Runtime behavior must use the exact declaration the builder accepted.
struct MutatingDescriptorExecutor {
    capability_reads: Arc<AtomicUsize>,
}

impl CpuExecutor for MutatingDescriptorExecutor {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        std::thread::spawn(work);
    }

    fn capabilities(&self) -> ExecutorCapabilities {
        if self.capability_reads.fetch_add(1, Ordering::SeqCst) == 0 {
            ExecutorCapabilities::legacy()
                .nonblocking_submit(true)
                .declared_workers(1)
                .exclusive_pool(true)
                .physical_domain(PHYSICAL_CPU)
        } else {
            // If the runtime re-queries this port, the old implementation could
            // panic in dispatch planning and expose a descriptor it never built.
            ExecutorCapabilities::legacy()
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_validated_executor_descriptor_is_frozen_for_every_runtime_use() {
    let capability_reads = Arc::new(AtomicUsize::new(0));
    let class = TaskClass::new("c");
    let builder = Builder::new()
        .topology(TopologyConfig::new().cpu_fixed(1).large_stack_slots(1))
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            class.clone(),
            ClassPolicy::new().max_inflight(8).cpu_units(1),
        )
        .cpu_executor(Arc::new(MutatingDescriptorExecutor {
            capability_reads: Arc::clone(&capability_reads),
        }));
    let builder_debug = format!("{builder:?}");
    assert!(builder_debug.contains("cpu_executor_installed: true"));
    assert_eq!(
        capability_reads.load(Ordering::SeqCst),
        0,
        "debug formatting cannot consume an unvalidated adapter declaration"
    );
    let rt = builder
        .build()
        .expect("the first, complete declaration builds");

    let expected = ExecutorCapabilities::legacy()
        .nonblocking_submit(true)
        .declared_workers(1)
        .exclusive_pool(true)
        .physical_domain(PHYSICAL_CPU);
    assert_eq!(capability_reads.load(Ordering::SeqCst), 1);
    assert_eq!(rt.executor_capabilities(), expected);
    let debug = format!("{rt:?}");
    assert!(debug.contains("physical.cpu"), "{debug}");

    let io = rt
        .run_io(
            TaskSpec::io(class.clone()).operation("descriptor-io"),
            async { Ok::<_, ()>(1) },
        )
        .await
        .expect("I/O planning uses the frozen descriptor");
    let blocking = rt
        .run_blocking(
            TaskSpec::blocking(class.clone()).operation("descriptor-blocking"),
            || Ok::<_, ()>(2),
        )
        .await
        .expect("blocking planning uses the frozen descriptor");
    let cpu = rt
        .run_cpu(
            TaskSpec::cpu(class.clone()).operation("descriptor-cpu"),
            || Ok::<_, ()>(3),
        )
        .await
        .expect("CPU planning uses the frozen descriptor");
    let stacked = rt
        .run_async_with_requested_stack(
            TaskSpec::base(class, SubstrateHint::LargeStackCapability)
                .operation("descriptor-stack")
                .stack_size_bytes(STACK),
            || async { Ok::<_, ()>(4) },
        )
        .await
        .expect("requested-stack planning uses the frozen descriptor");

    assert_eq!((io, blocking, cpu, stacked), (1, 2, 3, 4));
    assert_eq!(
        capability_reads.load(Ordering::SeqCst),
        1,
        "build is the only descriptor authority"
    );
}

fn poll_once_without_tokio<F: std::future::Future + ?Sized>(
    future: std::pin::Pin<&mut F>,
) -> std::task::Poll<F::Output> {
    struct NoopWake;
    impl std::task::Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    let waker = std::task::Waker::from(Arc::new(NoopWake));
    let mut context = std::task::Context::from_waker(&waker);
    future.poll(&mut context)
}

#[test]
fn default_tokio_dispatch_without_a_runtime_is_rejected_before_admission() {
    let class = TaskClass::new("c");
    let rt = runtime_for(class.clone());
    let before = rt.snapshot();
    let blocking_ran = Arc::new(AtomicUsize::new(0));
    let ran = Arc::clone(&blocking_ran);
    let mut blocking = Box::pin(rt.run_blocking(
        TaskSpec::blocking(class.clone()).operation("outside-tokio-blocking"),
        move || {
            ran.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ()>(())
        },
    ));
    assert_eq!(
        poll_once_without_tokio(blocking.as_mut()),
        std::task::Poll::Ready(Err(RunError::Governor(GovernorError::WorkerUnavailable {
            context: "blocking worker".into(),
            detail: "an active Tokio runtime context is required before admission".into(),
        })))
    );
    assert_eq!(blocking_ran.load(Ordering::SeqCst), 0);
    assert_eq!(rt.snapshot(), before, "failed preflight cannot admit work");

    let mut local = Box::pin(rt.run_local(
        TaskSpec::local(class.clone()).operation("outside-tokio-local"),
        async { Ok::<_, ()>(()) },
    ));
    assert_eq!(
        poll_once_without_tokio(local.as_mut()),
        std::task::Poll::Ready(Err(RunError::Governor(
            GovernorError::LocalRuntimeUnavailable
        )))
    );
    assert_eq!(rt.snapshot(), before, "failed preflight cannot admit work");

    let cancelled = tokio_util::sync::CancellationToken::new();
    cancelled.cancel();
    let mut pre_cancelled = Box::pin(rt.run_blocking_with(
        TaskSpec::blocking(class.clone()).operation("cancel-before-context"),
        SubmitOptions::unbounded().with_cancel(cancelled),
        || Ok::<_, ()>(()),
    ));
    assert_eq!(
        poll_once_without_tokio(pre_cancelled.as_mut()),
        std::task::Poll::Ready(Err(RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::CancelledBeforeSubmit
        ))))
    );
    assert_eq!(
        rt.snapshot(),
        before,
        "pre-cancel precedence cannot admit work"
    );

    #[cfg(not(feature = "rayon"))]
    {
        let cpu_ran = Arc::new(AtomicUsize::new(0));
        let ran = Arc::clone(&cpu_ran);
        let mut cpu = Box::pin(rt.run_cpu(
            TaskSpec::cpu(class).operation("outside-tokio-cpu"),
            move || {
                ran.fetch_add(1, Ordering::SeqCst);
                Ok::<_, ()>(())
            },
        ));
        assert_eq!(
            poll_once_without_tokio(cpu.as_mut()),
            std::task::Poll::Ready(Err(RunError::Governor(GovernorError::WorkerUnavailable {
                context: "cpu executor".into(),
                detail: "an active Tokio runtime context is required before admission".into(),
            })))
        );
        assert_eq!(cpu_ran.load(Ordering::SeqCst), 0);
        assert_eq!(rt.snapshot(), before, "failed preflight cannot admit work");
    }
}

#[test]
fn explicitly_installed_tokio_cpu_adapter_requires_context_before_admission() {
    let class = TaskClass::new("c");
    let adapter =
        BlockingPoolCpuExecutor::new(std::num::NonZeroU32::new(1).expect("one worker is nonzero"));
    let rt = Builder::new()
        .topology(TopologyConfig::new().shared_blocking_domain(PhysicalDomainMode::Fixed(1)))
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            class.clone(),
            ClassPolicy::new().max_inflight(1).cpu_units(1),
        )
        .cpu_executor(Arc::new(adapter))
        .build()
        .expect("explicit Tokio adapter builds");
    assert!(rt.executor_capabilities().requires_tokio_context);
    let before = rt.snapshot();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    let mut cpu = Box::pin(rt.run_cpu(
        TaskSpec::cpu(class).operation("explicit-tokio"),
        move || {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok::<_, ()>(())
        },
    ));

    assert_eq!(
        poll_once_without_tokio(cpu.as_mut()),
        std::task::Poll::Ready(Err(RunError::Governor(GovernorError::WorkerUnavailable {
            context: "cpu executor".into(),
            detail: "an active Tokio runtime context is required before admission".into(),
        })))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(rt.snapshot(), before, "failed preflight cannot admit work");
}

#[test]
fn tokio_context_is_checked_when_the_dispatch_future_is_polled() {
    let class = TaskClass::new("c");
    let rt = runtime_for(class.clone());
    // Construct both futures outside Tokio. They are valid because the runtime
    // requirement applies to execution, when the future is polled.
    let blocking = rt.run_blocking_with(
        TaskSpec::blocking(class.clone()).operation("poll-inside-blocking"),
        SubmitOptions::unbounded(),
        || Ok::<_, ()>(1),
    );
    #[cfg(not(feature = "rayon"))]
    let cpu = rt.run_cpu_with(
        TaskSpec::cpu(class).operation("poll-inside-cpu"),
        SubmitOptions::unbounded(),
        || Ok::<_, ()>(2),
    );
    let host = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test Tokio runtime");
    host.block_on(async {
        assert_eq!(blocking.await.expect("blocking dispatch"), 1);
        #[cfg(not(feature = "rayon"))]
        assert_eq!(cpu.await.expect("CPU dispatch"), 2);
    });
}

#[test]
fn a_cpu_gate_wider_than_the_executor_declares_is_refused_at_build() {
    // The topology asks for 4 workers; the adapter says it has 2. Building
    // that runtime would admit four concurrent CPU jobs onto two threads and
    // call the queueing "unexplained". The builder refuses, typed.
    let error = Builder::new()
        .topology(TopologyConfig::new().cpu_fixed(4))
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new().max_inflight(8).cpu_units(1),
        )
        .cpu_executor(Arc::new(DeclaringExecutor { workers: 2 }))
        .build();
    let Err(error) = error else {
        panic!("fewer declared workers than the gate must be refused");
    };
    assert_eq!(
        error,
        GovernorError::InvalidTopology(TopologyError::ExecutorWorkerCountMismatch {
            declared: 2,
            resolved: 4,
            domain: PHYSICAL_CPU,
        })
    );

    // The boundary is exact: one fewer than the gate is refused, equal is fine.
    let error = Builder::new()
        .topology(TopologyConfig::new().cpu_fixed(4))
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new().max_inflight(8).cpu_units(1),
        )
        .cpu_executor(Arc::new(DeclaringExecutor { workers: 3 }))
        .build();
    let Err(error) = error else {
        panic!("one worker short of the gate is still fewer");
    };
    assert_eq!(
        error,
        GovernorError::InvalidTopology(TopologyError::ExecutorWorkerCountMismatch {
            declared: 3,
            resolved: 4,
            domain: PHYSICAL_CPU,
        })
    );
    let rt = Builder::new()
        .topology(TopologyConfig::new().cpu_fixed(4))
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new().max_inflight(8).cpu_units(1),
        )
        .cpu_executor(Arc::new(DeclaringExecutor { workers: 4 }))
        .build()
        .expect("exactly the gate is enough");
    assert_eq!(rt.executor_capabilities().declared_workers, Some(4));

    // Declaring more is also a mismatch: inventory must describe the actual
    // finite physical domain, not merely a lower admission bound.
    let error = Builder::new()
        .topology(TopologyConfig::new().cpu_fixed(4))
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new().max_inflight(8).cpu_units(1),
        )
        .cpu_executor(Arc::new(DeclaringExecutor { workers: 16 }))
        .build();
    assert_eq!(
        error.expect_err("wider declaration must be rejected"),
        GovernorError::InvalidTopology(TopologyError::ExecutorWorkerCountMismatch {
            declared: 16,
            resolved: 4,
            domain: PHYSICAL_CPU,
        })
    );

    // A legacy/undeclared adapter is rejected. Unknown is not a physical
    // capacity guarantee and inline submission cannot be admitted as a second
    // unbounded execution path.
    let error = Builder::new()
        .topology(TopologyConfig::new().cpu_fixed(4))
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new().max_inflight(8).cpu_units(1),
        )
        .cpu_executor(Arc::new(LegacyExecutor))
        .build();
    assert!(
        matches!(
            error,
            Err(GovernorError::InvalidTopology(
                TopologyError::ExecutorSubmissionMayBlock
            ))
        ),
        "legacy inline executor must be rejected: {error:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_caller_that_stops_waiting_leaves_custody_with_the_worker() {
    // A dropped caller future is a normal, expected delivery outcome — not a
    // lost message. The worker owns the release in that case, and it happens
    // when the work really finishes.
    let class = TaskClass::new("c");
    let rt = runtime_for(class.clone());
    let (release, hold) = std::sync::mpsc::channel::<()>();
    let finished = Arc::new(AtomicUsize::new(0));
    let worker_finished = Arc::clone(&finished);

    let submission = rt.run_blocking(
        TaskSpec::blocking(class.clone()).operation("abandoned"),
        move || {
            // A signal or a dropped sender both release the holder: a test that fails
            // before signalling never hangs on its own fixture.
            let _released = hold.recv();
            worker_finished.fetch_add(1, Ordering::SeqCst);
            Ok::<i32, ()>(1)
        },
    );
    let mut submission = Box::pin(submission);
    // Poll once so the work is actually submitted, then walk away.
    assert!(
        futures_poll_once(&mut submission).await.is_none(),
        "the held job keeps the caller pending"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while rt.snapshot().classes[&class].inflight == 0 {
        assert!(Instant::now() < deadline, "worker never started");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    drop(submission);

    assert_eq!(
        rt.snapshot().classes[&class].inflight,
        1,
        "dropping the caller does not free capacity the worker still holds"
    );
    release.send(()).expect("worker still waiting");
    assert_drains(&rt, &class, "abandoned caller").await;
    assert_eq!(finished.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_ext_release_cannot_free_a_running_jobs_slot() {
    // H16-003 A7 / D14: `runtime.governor()` is public and permit ids are
    // sequential, so a consumer can name a running job's permit by accident.
    // Naming it is not owning it: the job's runtime lease holds the token the
    // engine minted at dispatch, and a plain `release` of that id is refused
    // without moving a gauge. The job keeps its slot until it finishes, and its
    // own completion releases it — once.
    let class = TaskClass::new("c");
    let rt = runtime_for(class.clone());
    let (finish_job, hold) = std::sync::mpsc::channel::<()>();
    let mut submission = Box::pin(rt.run_blocking(
        TaskSpec::blocking(class.clone()).operation("owned"),
        move || {
            // A signal or a dropped sender both let the job finish, so a test
            // that fails before signalling never hangs on its own fixture.
            let _released = hold.recv();
            Ok::<i32, ()>(7)
        },
    ));
    assert!(
        futures_poll_once(&mut submission).await.is_none(),
        "the held job keeps the caller pending"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while rt.snapshot().classes[&class].running == 0 {
        assert!(Instant::now() < deadline, "the job never started");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let ledgers = rt.governor().permit_ledgers();
    assert_eq!(ledgers.len(), 1, "one job, one permit");
    let permit = ledgers[0].permit_id;

    let before = rt.snapshot();
    assert_eq!(before.classes[&class].inflight, 1);
    assert_eq!(
        rt.governor().release(permit),
        ReleaseOutcome::HeldByLease {
            phase: ExecutionPhase::Running
        },
        "an ext release must not end a permit its runtime lease owns"
    );
    assert_eq!(
        (rt.snapshot(), rt.governor().permit_ledgers()),
        (before, ledgers),
        "the refused release must leave every gauge and ledger as it was"
    );

    finish_job.send(()).expect("the job is still waiting");
    let value = submission.await.expect("the job completes under its lease");
    assert_eq!(value, 7);
    // D10: a caller that sees the value sees the capacity already returned.
    let snapshot = rt.snapshot();
    let observed = &snapshot.classes[&class];
    assert_eq!(
        (observed.inflight, observed.terminated_total),
        (0, 1),
        "the job's own completion must release its slot, exactly once"
    );
    assert_eq!(snapshot.conservation_violation(), None);
    assert_eq!(
        rt.governor().release(permit),
        ReleaseOutcome::UnknownPermit,
        "the lease already ended the permit"
    );
}

/// Poll a future once, discarding the result. Used to start work without
/// awaiting it.
async fn futures_poll_once<F: std::future::Future>(
    future: &mut std::pin::Pin<Box<F>>,
) -> Option<F::Output> {
    std::future::poll_fn(|cx| {
        std::task::Poll::Ready(match future.as_mut().poll(cx) {
            std::task::Poll::Ready(value) => Some(value),
            std::task::Poll::Pending => None,
        })
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_runtimes_sharing_an_executor_each_govern_their_own_submissions() {
    // One real worker and one queue give the shared adapter a measurable
    // aggregate physical bound. Each runtime still owns only its own governor
    // accounting; the adapter supplies the cross-runtime physical serialization.
    struct SerialExecutor {
        queue: std::sync::mpsc::Sender<Box<dyn FnOnce() + Send + 'static>>,
        submitted: tokio::sync::mpsc::UnboundedSender<()>,
    }

    impl SerialExecutor {
        fn new(submitted: tokio::sync::mpsc::UnboundedSender<()>) -> Self {
            let (queue, worker) = std::sync::mpsc::channel::<Box<dyn FnOnce() + Send + 'static>>();
            std::thread::Builder::new()
                .name("taskmesh-test-shared-cpu".into())
                .spawn(move || {
                    while let Ok(work) = worker.recv() {
                        work();
                    }
                })
                .expect("shared executor worker starts");
            Self { queue, submitted }
        }
    }

    impl CpuExecutor for SerialExecutor {
        fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
            self.queue.send(work).expect("shared worker is alive");
            self.submitted.send(()).expect("submission observer alive");
        }

        fn capabilities(&self) -> ExecutorCapabilities {
            ExecutorCapabilities::legacy()
                .nonblocking_submit(true)
                .declared_workers(1)
                .exclusive_pool(false)
                .physical_domain(PHYSICAL_CPU)
        }
    }

    let (submitted_tx, mut submitted_rx) = tokio::sync::mpsc::unbounded_channel();
    let executor = Arc::new(SerialExecutor::new(submitted_tx));
    let build = |executor: Arc<dyn CpuExecutor>| {
        Builder::new()
            .topology(TopologyConfig::new().cpu_fixed(1))
            .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
            .class_policy(
                TaskClass::new("c"),
                ClassPolicy::new().max_inflight(8).cpu_units(1),
            )
            .cpu_executor(executor)
            .build()
            .expect("runtime builds")
    };
    let first = build(executor.clone());
    let second = build(executor.clone());

    assert_eq!(
        first.config().topology.cpu.mode,
        CpuMode::Fixed(1),
        "each runtime resolved the same declared topology"
    );

    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let (first_started_tx, mut first_started_rx) = tokio::sync::mpsc::unbounded_channel();
    let (second_started_tx, mut second_started_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release_first, hold_first) = std::sync::mpsc::channel::<()>();

    let first_run = {
        let runtime = first.clone();
        let active = Arc::clone(&active);
        let peak = Arc::clone(&peak);
        tokio::spawn(async move {
            runtime
                .run_cpu(
                    TaskSpec::cpu(TaskClass::new("c")).operation("first"),
                    move || {
                        let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now, Ordering::SeqCst);
                        first_started_tx.send(()).expect("first observer alive");
                        let _released = hold_first.recv();
                        active.fetch_sub(1, Ordering::SeqCst);
                        Ok::<_, ()>(1)
                    },
                )
                .await
        })
    };
    submitted_rx.recv().await.expect("first job submitted");
    first_started_rx.recv().await.expect("first job started");

    let second_run = {
        let runtime = second.clone();
        let active = Arc::clone(&active);
        let peak = Arc::clone(&peak);
        tokio::spawn(async move {
            runtime
                .run_cpu(
                    TaskSpec::cpu(TaskClass::new("c")).operation("second"),
                    move || {
                        let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now, Ordering::SeqCst);
                        second_started_tx.send(()).expect("second observer alive");
                        active.fetch_sub(1, Ordering::SeqCst);
                        Ok::<_, ()>(2)
                    },
                )
                .await
        })
    };
    submitted_rx.recv().await.expect("second job submitted");

    let first_snapshot = first.snapshot();
    let first_busy = &first_snapshot.classes[&TaskClass::new("c")];
    assert_eq!((first_busy.inflight, first_busy.running), (1, 1));
    let second_snapshot = second.snapshot();
    let second_queued_at_executor = &second_snapshot.classes[&TaskClass::new("c")];
    assert_eq!(
        (
            second_queued_at_executor.inflight,
            second_queued_at_executor.accepted,
            second_queued_at_executor.running,
        ),
        (1, 1, 0),
        "the second runtime accounts custody while the shared executor queues it"
    );
    assert_eq!(
        second_started_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty),
        "one physical worker prevents cross-runtime overlap"
    );
    assert_eq!(peak.load(Ordering::SeqCst), 1);

    release_first.send(()).expect("first worker still waiting");
    second_started_rx
        .recv()
        .await
        .expect("second job starts after the first");
    let a = first_run
        .await
        .expect("first caller task")
        .expect("first job");
    let b = second_run
        .await
        .expect("second caller task")
        .expect("second job");
    assert_eq!((a, b), (1, 2));
    assert_eq!(
        peak.load(Ordering::SeqCst),
        1,
        "aggregate worker bound held"
    );

    // Each runtime reports only the job it admitted and drains independently.
    for runtime in [&first, &second] {
        let snapshot = runtime.snapshot();
        let class = &snapshot.classes[&TaskClass::new("c")];
        assert_eq!((class.admitted_total, class.inflight), (1, 0));
        assert_eq!(snapshot.capabilities["cpu"].limit, 1);
        assert_eq!(snapshot.capabilities["cpu"].in_use, 0);
        assert_eq!(snapshot.conservation_violation(), None);
    }
}
