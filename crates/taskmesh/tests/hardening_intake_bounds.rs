//! TM16-001 regression: there is no unbounded holding area in front of
//! admission.
//!
//! The original shape: the host took the physical capability slot *first*, on a
//! plain semaphore, and only then asked the governor for semantic permission.
//! Three consequences followed, and all three are asserted against here.
//!
//! 1. The semaphore had no depth limit, so `max_queue_depth` and
//!    `OverflowPolicy::Reject` governed a queue requests reached *second*.
//!    Thirty-two submissions to a `Reject` class parked instead of rejecting,
//!    and the class snapshot reported `queued = 0` while they waited.
//! 2. A request could hold a worker slot while waiting for class capacity,
//!    idling a worker another class was runnable on.
//! 3. The semaphore is FIFO, so arrival order arbitrated before class fairness
//!    ever saw the request.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use taskmesh::ext::*;
use taskmesh::*;

fn reject_runtime() -> TokioRuntime {
    Builder::new()
        .topology(TopologyConfig::new().blocking_threads(1))
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(0)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::Reject),
        )
        .build()
        .expect("runtime builds")
}

fn blocking(op: &str) -> TaskSpec {
    TaskSpec::blocking(TaskClass::new("c")).operation(op.to_string())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reject_class_rejects_instead_of_parking_behind_a_busy_worker() {
    let rt = reject_runtime();
    let (release, hold) = tokio::sync::oneshot::channel::<()>();
    let rt_holder = rt.clone();
    let holder = tokio::spawn(async move {
        rt_holder
            .run_blocking(blocking("hold"), move || {
                // A signal or a dropped sender both release the holder: a test that fails
                // before signalling never hangs on its own fixture.
                let _released = hold.blocking_recv();
                Ok::<_, ()>(())
            })
            .await
    });

    // Wait until the holder is actually executing, rather than sleeping a
    // guessed interval and hoping.
    let deadline = Instant::now() + Duration::from_secs(5);
    while rt.snapshot().classes[&TaskClass::new("c")].inflight == 0 {
        assert!(Instant::now() < deadline, "holder never started");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    let started = Instant::now();
    let mut rejected = 0usize;
    for i in 0..32 {
        let result: Result<i32, RunError<()>> = rt
            .run_blocking_with(
                blocking(&format!("burst{i}")),
                // A generous budget is the control: if anything waits, this test
                // notices by taking a long time, not by timing out.
                SubmitOptions::unbounded().with_acquire_timeout(Duration::from_secs(30)),
                || Ok(1),
            )
            .await;
        // The class quota (`max_inflight = 1`) is the first limit the intake
        // transition meets, so the verdict names it; nothing in front of the
        // decision waited on the (equally full) blocking pool.
        match result {
            Err(RunError::Governor(GovernorError::Rejected(AdmissionVerdict::CpuSaturated {
                ..
            }))) => rejected += 1,
            other => {
                panic!("submission {i} must be shed with the class-quota verdict, got {other:?}")
            }
        }
    }
    assert_eq!(rejected, 32);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "a Reject class waited for capacity: {:?}",
        started.elapsed()
    );

    // Nothing accumulated anywhere the class could not see.
    let snapshot = rt.snapshot();
    assert_eq!(snapshot.classes[&TaskClass::new("c")].queued, 0);
    assert_eq!(snapshot.classes[&TaskClass::new("c")].inflight, 1);
    assert_eq!(snapshot.capabilities["blocking"].in_use, 1);
    assert_eq!(snapshot.conservation_violation(), None);

    release.send(()).expect("holder still waiting");
    holder.await.expect("join").expect("holder completes");
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_and_disabled_classes_reject_without_waiting_for_a_worker() {
    // A terminal rejection must not be delayed behind a saturated pool: the
    // answer does not depend on capacity at all.
    let rt = reject_runtime();
    let (release, hold) = tokio::sync::oneshot::channel::<()>();
    let rt_holder = rt.clone();
    let holder = tokio::spawn(async move {
        rt_holder
            .run_blocking(blocking("hold"), move || {
                // A signal or a dropped sender both release the holder: a test that fails
                // before signalling never hangs on its own fixture.
                let _released = hold.blocking_recv();
                Ok::<_, ()>(())
            })
            .await
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while rt.snapshot().classes[&TaskClass::new("c")].inflight == 0 {
        assert!(Instant::now() < deadline, "holder never started");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    let started = Instant::now();
    let unknown = TaskSpec::blocking(TaskClass::new("ghost")).operation("x");
    let result: Result<i32, RunError<()>> = rt
        .run_blocking_with(
            unknown,
            SubmitOptions::unbounded().with_acquire_timeout(Duration::from_secs(30)),
            || Ok(1),
        )
        .await;
    assert!(
        matches!(
            result,
            Err(RunError::Governor(GovernorError::Rejected(
                AdmissionVerdict::UnknownClass { .. }
            )))
        ),
        "got {result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "an unknown class waited for a worker"
    );

    release.send(()).expect("holder still waiting");
    holder.await.expect("join").expect("holder completes");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_class_blocked_on_its_own_quota_does_not_idle_a_shared_worker() {
    // `busy` is at its class cap; `spare` is not. They share one blocking
    // worker. A `busy` submission must not occupy that worker while it waits for
    // its own class capacity — if it does, `spare` cannot run even though a
    // worker is free by every accounting that matters.
    let rt = Builder::new()
        .topology(TopologyConfig::new().blocking_threads(1))
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            TaskClass::new("busy"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(8)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .class_policy(
            TaskClass::new("spare"),
            ClassPolicy::new()
                .max_inflight(4)
                .max_queue_depth(8)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .expect("runtime builds");

    // `busy` takes its only class slot *without* a worker, by going through the
    // governor directly: the worker stays free for `spare`.
    let busy_spec = TaskSpec::io(TaskClass::new("busy")).operation("quota-holder");
    let quota_holder = match rt.governor().admit(&busy_spec) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    };

    // A second `busy` submission is class-blocked; it queues rather than taking
    // the worker.
    let rt_blocked = rt.clone();
    let blocked = tokio::spawn(async move {
        rt_blocked
            .run_blocking_with(
                TaskSpec::blocking(TaskClass::new("busy")).operation("class-blocked"),
                SubmitOptions::unbounded(),
                || Ok::<i32, ()>(1),
            )
            .await
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    while rt.snapshot().classes[&TaskClass::new("busy")].queued == 0 {
        assert!(
            Instant::now() < deadline,
            "the blocked request never queued"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(
        rt.snapshot().capabilities["blocking"].in_use,
        0,
        "a request waiting for class capacity must not hold a worker"
    );

    // `spare` therefore runs immediately.
    let out: i32 = tokio::time::timeout(
        Duration::from_secs(5),
        rt.run_blocking(
            TaskSpec::blocking(TaskClass::new("spare")).operation("runnable"),
            || Ok::<_, ()>(7),
        ),
    )
    .await
    .expect("a runnable class must not be blocked by an idle-held worker")
    .expect("spare completes");
    assert_eq!(out, 7);

    assert_eq!(
        rt.governor().release(quota_holder),
        ReleaseOutcome::Released
    );
    assert_eq!(
        blocked.await.expect("join").expect("blocked completes"),
        1,
        "the queued request runs once its class frees up"
    );
    let snapshot = rt.snapshot();
    assert_eq!(snapshot.classes[&TaskClass::new("busy")].inflight, 0);
    assert_eq!(snapshot.conservation_violation(), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn class_fairness_is_not_pre_empted_by_arrival_at_a_worker_gate() {
    // A best-effort class must never run ahead of a queued primary one. When a
    // FIFO worker gate sat in front of admission, whichever request reached the
    // gate first won, and the scheduler never got to express the tier rule.
    let rt = Builder::new()
        .topology(TopologyConfig::new().blocking_threads(1))
        .resources(ResourceBudget::new().cpu_units(1).memory_units(1000))
        .class_policy(
            TaskClass::new("primary"),
            ClassPolicy::new()
                .max_inflight(4)
                .max_queue_depth(16)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .class_policy(
            TaskClass::new("scavenger"),
            ClassPolicy::new()
                .max_inflight(4)
                .max_queue_depth(16)
                .cpu_units(1)
                .best_effort(true)
                .fairness(FairnessPolicy::BestEffortScavenger)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .expect("runtime builds");

    let order = Arc::new(AtomicUsize::new(0));
    let scavenger_position = Arc::new(AtomicUsize::new(usize::MAX));
    let primary_position = Arc::new(AtomicUsize::new(usize::MAX));

    // Occupy the single cpu unit so both submissions must queue.
    let holder = match rt
        .governor()
        .admit(&TaskSpec::io(TaskClass::new("primary")).operation("holder"))
    {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    };

    // The best-effort request arrives FIRST — the arrival order that used to win.
    let rt_scav = rt.clone();
    let scav_order = Arc::clone(&order);
    let scav_slot = Arc::clone(&scavenger_position);
    let scavenger = tokio::spawn(async move {
        rt_scav
            .run_blocking(
                TaskSpec::blocking(TaskClass::new("scavenger")).operation("late-but-first"),
                move || {
                    scav_slot.store(scav_order.fetch_add(1, Ordering::SeqCst), Ordering::SeqCst);
                    Ok::<i32, ()>(1)
                },
            )
            .await
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while rt.snapshot().classes[&TaskClass::new("scavenger")].queued == 0 {
        assert!(Instant::now() < deadline, "scavenger never queued");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    let rt_prim = rt.clone();
    let prim_order = Arc::clone(&order);
    let prim_slot = Arc::clone(&primary_position);
    let primary = tokio::spawn(async move {
        rt_prim
            .run_blocking(
                TaskSpec::blocking(TaskClass::new("primary")).operation("second-but-first"),
                move || {
                    prim_slot.store(prim_order.fetch_add(1, Ordering::SeqCst), Ordering::SeqCst);
                    Ok::<i32, ()>(2)
                },
            )
            .await
    });
    while rt.snapshot().classes[&TaskClass::new("primary")].queued == 0 {
        assert!(Instant::now() < deadline, "primary never queued");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    assert_eq!(rt.governor().release(holder), ReleaseOutcome::Released);
    assert_eq!(scavenger.await.expect("join").expect("scavenger runs"), 1);
    assert_eq!(primary.await.expect("join").expect("primary runs"), 2);

    assert!(
        primary_position.load(Ordering::SeqCst) < scavenger_position.load(Ordering::SeqCst),
        "the primary class must dispatch before the best-effort one regardless of arrival order"
    );
}
