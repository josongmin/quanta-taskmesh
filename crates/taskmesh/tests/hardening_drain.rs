//! Drain regressions (H16-012-A07, ADR 0003 D17).
//!
//! `TokioRuntime::drain` is the host's only shutdown *contract*: it does not
//! tear anything down (dropping the handle still does that), it stops the
//! runtime taking work and then waits for the work it already holds. The
//! properties asserted here, each against its own mutant in
//! `tools/verification/mutations.json`:
//!
//! * **Refusal before admission.** Once draining, every submission path — the
//!   plain `run_*`, the `_with` variants, the dedicated-thread and
//!   requested-stack paths — is refused with the typed verdict
//!   `Rejected(RuntimeUnavailable)`; the job never runs and the engine's
//!   gauges and totals do not move.
//! * **Graceful.** Work that was queued or in flight when the drain began is
//!   not cancelled; it finishes, its callers get their values, and the drain
//!   returns `Ok` only after every class shows `inflight == 0 && queued == 0`.
//! * **Custody, not replies (D10).** A blocking worker that is still running
//!   keeps the drain waiting; on timeout the drain reports that class with
//!   `inflight == 1` and the work stays charged. The runtime stays draining and
//!   a second drain after the worker ends succeeds.
//! * **Event-driven.** The drain returns promptly when the last custody
//!   returns — a lease released, or a promoted-but-unclaimed ticket abandoned —
//!   not when its timeout elapses; and while it waits it sleeps, so a task
//!   sharing its thread keeps running.
//! * **No admission after `is_draining()` is observable.** Submitters racing
//!   the flag may be admitted before they saw it and are always refused after.
//!
//! Every wait is bounded (`bounded`, 5 s) and every held worker is released on
//! drop, so a regression surfaces as a named assertion, never as a hung test
//! binary.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use taskmesh::*;

const STACK: u64 = 2 * 1024 * 1024;

/// How long a bounded-wait assertion waits before declaring a hang. Far above
/// any budget under test, far below "the test binary never finishes".
const HANG: Duration = Duration::from_secs(5);

/// Releases a held worker when dropped, so a test that *fails* (or panics in
/// a later assertion) still lets the worker finish and the runtime tear down.
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

/// Wait, bounded, until `condition` holds on a fresh snapshot.
async fn until_snapshot(rt: &TokioRuntime, what: &str, condition: impl Fn(&Snapshot) -> bool) {
    let deadline = Instant::now() + HANG;
    while !condition(&rt.snapshot()) {
        assert!(
            Instant::now() < deadline,
            "{what}: never observed within {HANG:?}: {:?}",
            rt.snapshot().classes
        );
        tokio::task::yield_now().await;
    }
}

/// Wait, bounded, until the drain flag is observable from this task.
async fn until_draining(rt: &TokioRuntime, what: &str) {
    let deadline = Instant::now() + HANG;
    while !rt.is_draining() {
        assert!(
            Instant::now() < deadline,
            "{what}: is_draining() never became observable within {HANG:?}"
        );
        tokio::task::yield_now().await;
    }
}

fn class(name: &'static str) -> TaskClass {
    TaskClass::new(name)
}

/// A queueing class `c` with one execution slot plus an idle class `other`, on
/// a topology with room for every substrate the run paths reach.
fn drain_runtime(max_inflight: u32) -> TokioRuntime {
    Builder::new()
        .topology(
            TopologyConfig::new()
                .blocking_threads(4)
                .large_stack_slots(2)
                .local_runtime_slots(2),
        )
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            class("c"),
            ClassPolicy::new()
                .max_inflight(max_inflight)
                .max_queue_depth(8)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .class_policy(
            class("other"),
            ClassPolicy::new().max_inflight(4).cpu_units(1),
        )
        .build()
        .expect("runtime builds")
}

fn assert_idle(snapshot: &Snapshot, what: &str) {
    for (name, observed) in &snapshot.classes {
        assert_eq!(
            (observed.inflight, observed.queued),
            (0, 0),
            "{what}: class {name} must be idle after a drained Ok"
        );
    }
    assert_eq!(snapshot.conservation_violation(), None, "{what}");
}

fn refused<T: std::fmt::Debug>(what: &str, result: Result<T, RunError<()>>) {
    match result {
        Err(RunError::Governor(GovernorError::Rejected(AdmissionVerdict::RuntimeUnavailable))) => {}
        other => {
            panic!("{what} must be refused with RuntimeUnavailable once draining, got {other:?}")
        }
    }
}

// ---- an idle runtime drains at once, and the state is one-way ------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_idle_runtime_drains_at_once_and_stays_draining() {
    let rt = drain_runtime(1);
    assert!(!rt.is_draining(), "a fresh runtime is not draining");

    let report = bounded("idle drain", rt.drain(Duration::from_secs(30)))
        .await
        .expect("nothing is charged, so an idle runtime drains at once");
    assert_eq!(
        report.classes_drained, 2,
        "every class the governor knows is verified idle, busy or not"
    );
    assert!(rt.is_draining(), "drain must leave the runtime draining");
    assert_idle(&rt.snapshot(), "idle drain");
}

// ---- once draining, every submission path is refused before admission ----

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_submission_path_is_refused_before_admission_once_draining() {
    let rt = drain_runtime(4);
    bounded("drain before refusals", rt.drain(Duration::from_secs(30)))
        .await
        .expect("idle runtime drains");
    let before = rt.snapshot();
    let ran = Arc::new(AtomicUsize::new(0));
    let touch = {
        let ran = Arc::clone(&ran);
        move || {
            ran.fetch_add(1, Ordering::SeqCst);
        }
    };

    let io = TaskSpec::io(class("c")).operation("io");
    let blocking = TaskSpec::blocking(class("c")).operation("blocking");
    let cpu = TaskSpec::cpu(class("c")).operation("cpu");
    let local = TaskSpec::local(class("c")).operation("local");
    let stack_blocking = TaskSpec::blocking(class("c"))
        .operation("stack-blocking")
        .stack_size_bytes(STACK);
    let stack_async = TaskSpec::base(class("c"), SubstrateHint::LargeStackCapability)
        .operation("stack-async")
        .stack_size_bytes(STACK);
    let opts = || SubmitOptions::unbounded().with_acquire_timeout(Duration::from_secs(30));

    let t = touch.clone();
    refused(
        "run_io",
        rt.run_io(io.clone(), async move {
            t();
            Ok::<i32, ()>(1)
        })
        .await,
    );
    let t = touch.clone();
    refused(
        "run_io_with",
        rt.run_io_with(io, opts(), async move {
            t();
            Ok::<i32, ()>(1)
        })
        .await,
    );
    let t = touch.clone();
    refused(
        "run_blocking",
        rt.run_blocking(blocking.clone(), move || {
            t();
            Ok::<i32, ()>(1)
        })
        .await,
    );
    let t = touch.clone();
    refused(
        "run_blocking_with",
        rt.run_blocking_with(blocking, opts(), move || {
            t();
            Ok::<i32, ()>(1)
        })
        .await,
    );
    let t = touch.clone();
    refused(
        "run_blocking on a dedicated stack thread",
        rt.run_blocking(stack_blocking, move || {
            t();
            Ok::<i32, ()>(1)
        })
        .await,
    );
    let t = touch.clone();
    refused(
        "run_cpu",
        rt.run_cpu(cpu.clone(), move || {
            t();
            Ok::<i32, ()>(1)
        })
        .await,
    );
    let t = touch.clone();
    refused(
        "run_cpu_with",
        rt.run_cpu_with(cpu, opts(), move || {
            t();
            Ok::<i32, ()>(1)
        })
        .await,
    );
    let t = touch.clone();
    refused(
        "run_local",
        rt.run_local(local.clone(), async move {
            t();
            Ok::<i32, ()>(1)
        })
        .await,
    );
    let t = touch.clone();
    refused(
        "run_local_with",
        rt.run_local_with(local, opts(), async move {
            t();
            Ok::<i32, ()>(1)
        })
        .await,
    );
    let t = touch.clone();
    refused(
        "run_async_with_requested_stack",
        rt.run_async_with_requested_stack(stack_async.clone(), move || async move {
            t();
            Ok::<i32, ()>(1)
        })
        .await,
    );
    let t = touch;
    refused(
        "run_async_with_requested_stack_with",
        rt.run_async_with_requested_stack_with(stack_async, opts(), move || async move {
            t();
            Ok::<i32, ()>(1)
        })
        .await,
    );

    assert_eq!(
        ran.load(Ordering::SeqCst),
        0,
        "a refused submission must never run its job"
    );
    let after = rt.snapshot();
    assert_eq!(
        after.classes, before.classes,
        "a refusal before admission leaves every gauge and total untouched"
    );
    assert_eq!(after.capabilities, before.capabilities);
    assert_eq!(after.conservation_violation(), None);
}

// ---- admitted work is not cancelled; Ok arrives when it is really gone ----

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queued_and_inflight_work_finish_before_drain_reports_ok() {
    // One slot: the first job holds it, the second queues behind it. The drain
    // begins with both outstanding, refuses a third, cancels neither, and
    // returns Ok only once both have released their leases.
    let rt = drain_runtime(1);
    let (release, hold) = std::sync::mpsc::channel::<()>();
    let release = ReleaseOnDrop(Some(release));

    let rt_holder = rt.clone();
    let holder = tokio::spawn(async move {
        rt_holder
            .run_blocking(
                TaskSpec::blocking(class("c")).operation("hold"),
                move || {
                    let _released = hold.recv();
                    Ok::<i32, ()>(1)
                },
            )
            .await
    });
    until_snapshot(&rt, "holder running", |s| {
        s.classes[&class("c")].inflight == 1
    })
    .await;

    let rt_queued = rt.clone();
    let queued = tokio::spawn(async move {
        rt_queued
            .run_blocking(TaskSpec::blocking(class("c")).operation("queued"), || {
                Ok::<i32, ()>(2)
            })
            .await
    });
    until_snapshot(&rt, "second job queued", |s| {
        s.classes[&class("c")].queued == 1
    })
    .await;

    let rt_drain = rt.clone();
    let drain = tokio::spawn(async move { rt_drain.drain(Duration::from_secs(30)).await });
    until_draining(&rt, "drain in progress").await;

    // Graceful: the drain refuses new work but has cancelled nothing.
    refused(
        "a submission during the drain",
        rt.run_blocking(TaskSpec::blocking(class("c")).operation("late"), || {
            Ok::<i32, ()>(3)
        })
        .await,
    );
    let during = rt.snapshot();
    assert_eq!(
        (
            during.classes[&class("c")].inflight,
            during.classes[&class("c")].queued
        ),
        (1, 1),
        "drain must not cancel the held worker or the queued request"
    );
    assert!(!drain.is_finished(), "drain must wait for outstanding work");

    release.release();
    let report = bounded("drain after release", drain)
        .await
        .expect("drain task joins")
        .expect("both jobs finish, so the drain succeeds");
    assert_eq!(
        report.classes_drained, 2,
        "both classes are verified idle in the report"
    );
    assert_eq!(
        bounded("held job", holder).await.expect("join"),
        Ok(1),
        "the held job was not cancelled by the drain"
    );
    assert_eq!(
        bounded("queued job", queued).await.expect("join"),
        Ok(2),
        "the queued job was promoted and run despite the drain"
    );
    let after = rt.snapshot();
    assert_idle(&after, "after release");
    assert_eq!(
        after.classes[&class("c")].admitted_total,
        2,
        "exactly the two pre-drain submissions were ever admitted"
    );
}

// ---- a live worker keeps the drain waiting; timeout is typed and sticky ---

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_held_blocking_worker_makes_drain_report_not_drained_and_stay_charged() {
    let rt = drain_runtime(2);
    let (release, hold) = std::sync::mpsc::channel::<()>();
    let release = ReleaseOnDrop(Some(release));
    let finished = Arc::new(AtomicBool::new(false));
    let worker_finished = Arc::clone(&finished);

    let rt_holder = rt.clone();
    let holder = tokio::spawn(async move {
        rt_holder
            .run_blocking(
                TaskSpec::blocking(class("c")).operation("hold"),
                move || {
                    // Runs until explicitly released: a started blocking job cannot
                    // be aborted, and the drain does not pretend otherwise.
                    let _released = hold.recv();
                    worker_finished.store(true, Ordering::SeqCst);
                    Ok::<i32, ()>(7)
                },
            )
            .await
    });
    until_snapshot(&rt, "holder running", |s| {
        s.classes[&class("c")].inflight == 1
    })
    .await;

    let timeout = Duration::from_millis(50);
    let not_drained = bounded("timed-out drain", rt.drain(timeout))
        .await
        .expect_err("a held worker must make drain report NotDrained");
    assert!(
        not_drained.elapsed >= timeout,
        "the drain gave up after {:?}, before its {timeout:?} budget",
        not_drained.elapsed
    );
    let expected: BTreeMap<TaskClass, Outstanding> = BTreeMap::from([(
        class("c"),
        Outstanding {
            inflight: 1,
            queued: 0,
        },
    )]);
    assert_eq!(
        not_drained.classes, expected,
        "NotDrained must carry the held worker under its own class"
    );
    assert!(
        !finished.load(Ordering::SeqCst),
        "the worker is still running: the timeout bounded the wait, not the work"
    );
    assert_eq!(
        rt.snapshot().classes[&class("c")].inflight,
        1,
        "a timed-out drain must not free capacity a live worker holds (D10)"
    );
    assert!(
        rt.is_draining(),
        "a timed-out drain must leave the runtime draining"
    );
    refused(
        "a submission after a timed-out drain",
        rt.run_blocking(TaskSpec::blocking(class("c")).operation("late"), || {
            Ok::<i32, ()>(3)
        })
        .await,
    );
    // A zero timeout takes one look and answers with the same counts.
    let one_look = bounded("zero-timeout drain", rt.drain(Duration::ZERO))
        .await
        .expect_err("the worker is still held, so one look reports NotDrained");
    assert_eq!(
        one_look.classes, expected,
        "one look sees the same held worker"
    );

    // The worker ends; a second drain continues the same wait and succeeds.
    release.release();
    assert_eq!(
        bounded("held job", holder).await.expect("join"),
        Ok(7),
        "the held job completes normally after the drain gave up on it"
    );
    let report = bounded("second drain", rt.drain(Duration::from_secs(30)))
        .await
        .expect("with the worker gone, the second drain succeeds");
    assert_eq!(report.classes_drained, 2);
    assert!(
        rt.is_draining(),
        "a second drain must not resume the runtime: draining is one-way"
    );
    assert!(finished.load(Ordering::SeqCst));
    assert_idle(&rt.snapshot(), "after the second drain");
}

// ---- the wait yields: a drain that is waiting costs its thread nothing ----

#[tokio::test(flavor = "current_thread")]
async fn a_waiting_drain_yields_its_thread_instead_of_spinning() {
    // Event-driven means the drain *sleeps* between custody returns. On a
    // current-thread runtime a drain that re-polled instead of sleeping (a
    // deadline future that resolves at once, a loop that never parks) would
    // hold the only thread for its whole timeout: a ticker task sharing that
    // thread would not run once. The result alone cannot tell the two apart —
    // both return NotDrained after the timeout — so the ticker is the witness.
    let rt = drain_runtime(2);
    let (release, hold) = std::sync::mpsc::channel::<()>();
    let release = ReleaseOnDrop(Some(release));
    let rt_holder = rt.clone();
    let holder = tokio::spawn(async move {
        rt_holder
            .run_blocking(
                TaskSpec::blocking(class("c")).operation("hold"),
                move || {
                    let _released = hold.recv();
                    Ok::<i32, ()>(1)
                },
            )
            .await
    });
    until_snapshot(&rt, "holder running", |s| {
        s.classes[&class("c")].inflight == 1
    })
    .await;

    let ticks = Arc::new(AtomicUsize::new(0));
    let ticker = {
        let ticks = Arc::clone(&ticks);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(5)).await;
                ticks.fetch_add(1, Ordering::SeqCst);
            }
        })
    };
    let before = ticks.load(Ordering::SeqCst);
    let not_drained = bounded(
        "drain with a held worker",
        rt.drain(Duration::from_millis(200)),
    )
    .await
    .expect_err("the held worker keeps the drain from completing");
    let during = ticks.load(Ordering::SeqCst) - before;
    ticker.abort();
    assert!(
        not_drained.elapsed >= Duration::from_millis(200),
        "the drain waited its whole budget"
    );
    // ~40 ticks fit in 200 ms; 3 is a floor a loaded machine still clears and a
    // spinning drain (0) cannot.
    assert!(
        during >= 3,
        "a waiting drain must yield its thread: a task sharing it ticked only {during} times in {:?}",
        not_drained.elapsed
    );

    release.release();
    assert_eq!(bounded("holder", holder).await.expect("join"), Ok(1));
    bounded("second drain", rt.drain(Duration::from_secs(30)))
        .await
        .expect("with the worker gone the drain completes");
}

// ---- a promoted-but-unclaimed ticket abandoned mid-drain wakes it ---------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abandoning_an_unclaimed_promotion_is_a_custody_return_the_drain_hears() {
    // The waiter is a future this test drives by hand, so after the holder
    // releases, its ticket is promoted (the engine charges the permit) but
    // never claimed. Dropping the future is then the *last* custody return —
    // the `TicketGuard` abandon — and the drain must complete on that event,
    // not on its timeout.
    let rt = drain_runtime(1);
    let (release, hold) = std::sync::mpsc::channel::<()>();
    let release = ReleaseOnDrop(Some(release));

    let rt_holder = rt.clone();
    let holder = tokio::spawn(async move {
        rt_holder
            .run_blocking(
                TaskSpec::blocking(class("c")).operation("hold"),
                move || {
                    let _released = hold.recv();
                    Ok::<i32, ()>(1)
                },
            )
            .await
    });
    until_snapshot(&rt, "holder running", |s| {
        s.classes[&class("c")].inflight == 1
    })
    .await;

    let mut waiter = Box::pin(rt.run_blocking(
        TaskSpec::blocking(class("c")).operation("unclaimed"),
        || Ok::<i32, ()>(2),
    ));
    // Poll it into the queue, then stop polling it.
    let parked = tokio::time::timeout(Duration::from_millis(20), waiter.as_mut()).await;
    assert!(
        parked.is_err(),
        "the waiter must queue behind the held slot, got {parked:?}"
    );
    until_snapshot(&rt, "waiter queued", |s| s.classes[&class("c")].queued == 1).await;

    let rt_drain = rt.clone();
    let drain = tokio::spawn(async move { rt_drain.drain(Duration::from_secs(30)).await });
    until_draining(&rt, "drain in progress").await;

    // The holder finishes: the engine promotes the waiter's ticket in the same
    // transition, so the slot is charged to a permit nobody has claimed.
    release.release();
    assert_eq!(bounded("holder", holder).await.expect("join"), Ok(1));
    until_snapshot(&rt, "waiter promoted but unclaimed", |s| {
        let c = &s.classes[&class("c")];
        c.inflight == 1 && c.queued == 0
    })
    .await;
    assert!(
        !drain.is_finished(),
        "an unclaimed promotion is still charged, so the drain must keep waiting"
    );

    // Abandon it: the guard returns the promoted permit, and that return is
    // what the drain has been waiting to hear.
    drop(waiter);
    let report = bounded("drain after the unclaimed promotion was abandoned", drain)
        .await
        .expect("drain task joins")
        .expect("with the abandoned permit returned, nothing is charged");
    assert_eq!(report.classes_drained, 2);
    let after = rt.snapshot();
    assert_idle(&after, "after the abandon");
    assert_eq!(
        after.classes[&class("c")].admitted_total,
        2,
        "the holder and the promoted (then abandoned) permit were the only admissions"
    );
    assert_eq!(
        after.classes[&class("c")].started_total,
        1,
        "the abandoned permit never started: only the holder ran"
    );
}

// ---- the report attributes outstanding work to the class that holds it ---

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn not_drained_lists_each_class_with_its_own_inflight_and_queued_counts() {
    // One blocking slot shared by two queueing classes: `a` holds it (inflight
    // 1), `b` waits for it (queued 1, inflight 0). Only a per-class report
    // that counts *both* gauges, under the right class, describes this.
    let rt = Builder::new()
        .topology(TopologyConfig::new().blocking_threads(1))
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            class("a"),
            ClassPolicy::new()
                .max_inflight(4)
                .max_queue_depth(8)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .class_policy(
            class("b"),
            ClassPolicy::new()
                .max_inflight(4)
                .max_queue_depth(8)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .expect("runtime builds");
    let (release, hold) = std::sync::mpsc::channel::<()>();
    let release = ReleaseOnDrop(Some(release));

    let rt_holder = rt.clone();
    let holder = tokio::spawn(async move {
        rt_holder
            .run_blocking(
                TaskSpec::blocking(class("a")).operation("hold"),
                move || {
                    let _released = hold.recv();
                    Ok::<i32, ()>(1)
                },
            )
            .await
    });
    until_snapshot(&rt, "a holds the slot", |s| {
        s.classes[&class("a")].inflight == 1
    })
    .await;
    let rt_waiter = tokio::spawn({
        let rt = rt.clone();
        async move {
            rt.run_blocking(TaskSpec::blocking(class("b")).operation("wait"), || {
                Ok::<i32, ()>(2)
            })
            .await
        }
    });
    until_snapshot(&rt, "b queued on the pool", |s| {
        s.classes[&class("b")].queued == 1
    })
    .await;

    let not_drained = bounded("timed-out drain", rt.drain(Duration::from_millis(50)))
        .await
        .expect_err("a held slot and a queued request must make drain report NotDrained");
    let classes = &not_drained.classes;
    let a_left = classes.get(&class("a")).unwrap_or_else(|| {
        panic!("class a must be listed: its worker is still held (got {classes:?})")
    });
    assert_eq!(
        a_left.inflight, 1,
        "class a must report its held worker as inflight"
    );
    assert_eq!(a_left.queued, 0, "class a has nothing queued");
    let b_left = classes.get(&class("b")).unwrap_or_else(|| {
        panic!("class b must be listed: its request is still queued (got {classes:?})")
    });
    assert_eq!(
        b_left.queued, 1,
        "class b must report its queued request as queued"
    );
    assert_eq!(b_left.inflight, 0, "class b holds nothing yet");
    assert_eq!(
        classes.len(),
        2,
        "only classes with outstanding work are listed"
    );
    assert!(
        not_drained.to_string().contains("a inflight=1 queued=0"),
        "Display carries the per-class counts: {not_drained}"
    );

    release.release();
    assert_eq!(bounded("holder", holder).await.expect("join"), Ok(1));
    assert_eq!(bounded("waiter", rt_waiter).await.expect("join"), Ok(2));
    bounded("second drain", rt.drain(Duration::from_secs(30)))
        .await
        .expect("both classes idle: the second drain succeeds");
    assert_idle(&rt.snapshot(), "after the second drain");
}

// ---- submitters racing the flag: admitted before, never after -------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_submission_is_admitted_after_is_draining_became_observable() {
    // Each submitter reads `is_draining()` immediately before submitting and
    // records the pair. A submission that began after the flag was observable
    // must be refused; one that began before may go either way. The barrier
    // makes every submitter live before the drain starts, and each submitter
    // leaves only after it has been refused while draining — no sleeps, no
    // guessed windows.
    const SUBMITTERS: usize = 4;
    let rt = drain_runtime(8);
    let barrier = Arc::new(tokio::sync::Barrier::new(SUBMITTERS + 1));
    let admitted = Arc::new(AtomicUsize::new(0));

    let mut submitters = Vec::with_capacity(SUBMITTERS);
    for id in 0..SUBMITTERS {
        let rt = rt.clone();
        let barrier = Arc::clone(&barrier);
        let admitted = Arc::clone(&admitted);
        submitters.push(tokio::spawn(async move {
            // Positive control: before the drain, this submitter is admitted.
            let warm = rt
                .run_blocking(
                    TaskSpec::blocking(class("c")).operation(format!("warm{id}")),
                    || Ok::<i32, ()>(1),
                )
                .await;
            assert_eq!(warm, Ok(1), "submitter {id}: admitted before the drain");
            admitted.fetch_add(1, Ordering::SeqCst);
            bounded("submitter at the start barrier", barrier.wait()).await;
            let mut rounds = 0usize;
            loop {
                rounds += 1;
                let observed_draining = rt.is_draining();
                let result = rt
                    .run_blocking(
                        TaskSpec::blocking(class("c")).operation(format!("race{id}-{rounds}")),
                        || Ok::<i32, ()>(1),
                    )
                    .await;
                match result {
                    Ok(1) => {
                        admitted.fetch_add(1, Ordering::SeqCst);
                        assert!(
                            !observed_draining,
                            "submitter {id} round {rounds}: a submission that began after \
                             is_draining() was observable must not be admitted"
                        );
                    }
                    Err(RunError::Governor(GovernorError::Rejected(
                        AdmissionVerdict::RuntimeUnavailable,
                    ))) => {
                        if observed_draining {
                            return rounds;
                        }
                    }
                    other => panic!("submitter {id} round {rounds}: unexpected {other:?}"),
                }
                tokio::task::yield_now().await;
            }
        }));
    }

    // Bounded: a submitter that fails its positive control panics before the
    // barrier, and an unbounded wait here would turn that named failure into a
    // hung test binary.
    bounded("every submitter reaches the start barrier", barrier.wait()).await;
    let report = bounded(
        "drain under racing submitters",
        rt.drain(Duration::from_secs(30)),
    )
    .await
    .expect("every admitted job is trivial, so the drain completes");
    assert_eq!(report.classes_drained, 2);
    for (id, submitter) in submitters.into_iter().enumerate() {
        let rounds = bounded("submitter exits after a refusal", submitter)
            .await
            .expect("submitter task joins");
        assert!(rounds >= 1, "submitter {id} made at least one racing round");
    }

    // The engine agrees with the callers: nothing was admitted that did not
    // come back Ok, and nothing is outstanding.
    let after = rt.snapshot();
    assert_idle(&after, "after the race");
    assert_eq!(
        after.classes[&class("c")].admitted_total,
        u128::try_from(admitted.load(Ordering::SeqCst)).expect("fits"),
        "every admitted permit belongs to a submission that returned Ok"
    );
}
