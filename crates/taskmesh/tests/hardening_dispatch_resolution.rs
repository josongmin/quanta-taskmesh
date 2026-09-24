//! TM16-023 regression: the pool a submission *reserves* is the pool it
//! *occupies*.
//!
//! The original defect: a stack request changed which executor ran the work — a
//! dedicated OS thread instead of the blocking pool — but the capability gate
//! was still chosen from the declared hint. So `TaskSpec::blocking(c)` plus a
//! stack request took a `blocking` slot and ran on a large-stack worker, and
//! `large_stack_slots = 1` bounded nothing. Two such workers ran concurrently
//! against a cap of one.
//!
//! `BackgroundOnly` reached the same dedicated dispatch branch, so it is covered
//! too: the bypass was a property of the stack request, not of one spelling.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use taskmesh::*;

const STACK: u64 = 2 * 1024 * 1024;

fn runtime() -> TokioRuntime {
    Builder::new()
        // One large-stack slot, and NO blocking slots at all: if a stack request
        // were charged to `blocking`, it would be ungated (`0` means no limit)
        // and the cap below could never bind.
        .topology(
            TopologyConfig::new()
                .large_stack_slots(1)
                .blocking_threads(0),
        )
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(8)
                .max_queue_depth(8)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .expect("runtime builds")
}

async fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// Every blocking-family hint that accepts a stack request must consume the
/// large-stack capability, not the pool its hint happens to name.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stack_request_consumes_the_large_stack_capability_whatever_the_hint_says() {
    for hint in [
        SubstrateHint::BlockingPool,
        SubstrateHint::LargeStackCapability,
        SubstrateHint::BackgroundOnly,
    ] {
        let rt = runtime();
        let running = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(AtomicUsize::new(0));

        let (release, hold) = tokio::sync::oneshot::channel::<()>();
        let rt_first = rt.clone();
        let first_running = Arc::clone(&running);
        let first_started = Arc::clone(&started);
        let first = tokio::spawn(async move {
            rt_first
                .run_blocking(
                    TaskSpec::base(TaskClass::new("c"), hint)
                        .operation("first")
                        .stack_size_bytes(STACK),
                    move || {
                        first_started.fetch_add(1, Ordering::SeqCst);
                        first_running.fetch_add(1, Ordering::SeqCst);
                        // A signal or a dropped sender both release the holder: a test that fails
                        // before signalling never hangs on its own fixture.
                        let _released = hold.blocking_recv();
                        first_running.fetch_sub(1, Ordering::SeqCst);
                        Ok::<i32, ()>(1)
                    },
                )
                .await
        });
        wait_for("the first stack worker to start", || {
            started.load(Ordering::SeqCst) == 1
        })
        .await;

        // The capability charged is `large_stack` — and nothing is charged to
        // the pool the hint named.
        let snapshot = rt.snapshot();
        assert_eq!(
            snapshot.capabilities["large_stack"].in_use, 1,
            "{hint:?}: a stack request must occupy the large-stack pool"
        );
        assert_eq!(snapshot.capabilities["large_stack"].limit, 1);
        assert!(
            !snapshot.capabilities.contains_key("blocking")
                || snapshot.capabilities["blocking"].in_use == 0,
            "{hint:?}: a stack request must not be charged to the blocking pool"
        );

        // A second stack request cannot run: the cap is one.
        let rt_second = rt.clone();
        let second_running = Arc::clone(&running);
        let second_started = Arc::clone(&started);
        let second = tokio::spawn(async move {
            rt_second
                .run_blocking(
                    TaskSpec::base(TaskClass::new("c"), hint)
                        .operation("second")
                        .stack_size_bytes(STACK),
                    move || {
                        second_started.fetch_add(1, Ordering::SeqCst);
                        second_running.fetch_add(1, Ordering::SeqCst);
                        second_running.fetch_sub(1, Ordering::SeqCst);
                        Ok::<i32, ()>(2)
                    },
                )
                .await
        });
        wait_for("the second stack request to queue", || {
            rt.snapshot().classes[&TaskClass::new("c")].queued == 1
        })
        .await;
        assert_eq!(
            started.load(Ordering::SeqCst),
            1,
            "{hint:?}: the second worker must not start while the cap is held"
        );
        assert_eq!(running.load(Ordering::SeqCst), 1, "{hint:?}");

        release.send(()).expect("first worker still waiting");
        assert_eq!(first.await.expect("join").expect("first completes"), 1);
        assert_eq!(second.await.expect("join").expect("second completes"), 2);
        assert_eq!(started.load(Ordering::SeqCst), 2, "{hint:?}: both ran");

        let snapshot = rt.snapshot();
        assert_eq!(snapshot.capabilities["large_stack"].in_use, 0, "{hint:?}");
        assert_eq!(snapshot.conservation_violation(), None, "{hint:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_blocking_submission_without_a_stack_request_uses_the_blocking_pool() {
    // The control for the test above: without a stack request the same hint
    // resolves to the blocking pool, so the reclassification is driven by the
    // request, not by the entry point.
    let rt = Builder::new()
        .topology(
            TopologyConfig::new()
                .blocking_threads(1)
                .large_stack_slots(1),
        )
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(8)
                .max_queue_depth(8)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .expect("runtime builds");

    let (release, hold) = tokio::sync::oneshot::channel::<()>();
    let started = Arc::new(AtomicUsize::new(0));
    let rt_holder = rt.clone();
    let holder_started = Arc::clone(&started);
    let holder = tokio::spawn(async move {
        rt_holder
            .run_blocking(
                TaskSpec::blocking(TaskClass::new("c")).operation("plain"),
                move || {
                    holder_started.fetch_add(1, Ordering::SeqCst);
                    // A signal or a dropped sender both release the holder: a test that fails
                    // before signalling never hangs on its own fixture.
                    let _released = hold.blocking_recv();
                    Ok::<i32, ()>(1)
                },
            )
            .await
    });
    wait_for("the blocking worker to start", || {
        started.load(Ordering::SeqCst) == 1
    })
    .await;

    let snapshot = rt.snapshot();
    assert_eq!(snapshot.capabilities["blocking"].in_use, 1);
    assert_eq!(snapshot.capabilities["large_stack"].in_use, 0);

    release.send(()).expect("holder still waiting");
    assert_eq!(holder.await.expect("join").expect("completes"), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn direct_and_host_multistage_admission_have_distinct_reservation_contracts() {
    let rt = Builder::new()
        .topology(TopologyConfig::new().cpu_fixed(1))
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new().max_inflight(4).cpu_units(1),
        )
        .build()
        .expect("runtime builds");
    let staged = TaskSpec::io(TaskClass::new("c"))
        .operation("fetch")
        .stage(TaskStage::new("rank"), SubstrateHint::SharedCpuExecutor);

    let (release, hold) = tokio::sync::oneshot::channel::<()>();
    let host = rt.clone();
    let host_spec = staged.clone();
    let running = tokio::spawn(async move {
        host.run_io(host_spec, async move {
            let _released = hold.await;
            Ok::<(), ()>(())
        })
        .await
    });
    wait_for("the host IO permit", || {
        rt.snapshot().classes[&TaskClass::new("c")].inflight == 1
    })
    .await;
    assert_eq!(
        rt.snapshot().capabilities["cpu"].in_use,
        0,
        "the host reserves only the capability used by its first dispatch"
    );
    release.send(()).expect("host future still waiting");
    running
        .await
        .expect("host task joins")
        .expect("host IO completes");

    let decision = rt.governor().admit(&staged.operation("direct-fetch"));
    let ext::AdmissionDecision::Admitted { permit_id } = decision else {
        panic!("direct multistage admission must succeed, got {decision:?}");
    };
    assert_eq!(
        rt.snapshot().capabilities["cpu"].in_use,
        1,
        "direct admission reserves every distinct declared stage capability"
    );
    assert_eq!(
        rt.governor().release(permit_id),
        ext::ReleaseOutcome::Released
    );
    assert_eq!(rt.snapshot().capabilities["cpu"].in_use, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_requested_stack_async_path_shares_the_same_capability() {
    // The async large-stack entry point runs on the same dedicated-thread
    // dispatch, so it competes for the same pool. Two authorities over one pool
    // would be the same defect wearing a different hat.
    let rt = runtime();
    let started = Arc::new(AtomicUsize::new(0));
    let (release, hold) = tokio::sync::oneshot::channel::<()>();

    let rt_async = rt.clone();
    let async_started = Arc::clone(&started);
    let async_worker = tokio::spawn(async move {
        rt_async
            .run_async_with_requested_stack(
                TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability)
                    .operation("async-stack")
                    .stack_size_bytes(STACK),
                move || async move {
                    async_started.fetch_add(1, Ordering::SeqCst);
                    // A signal or a dropped sender both release the holder: a test that fails
                    // before signalling never hangs on its own fixture.
                    let _released = hold.await;
                    Ok::<i32, ()>(5)
                },
            )
            .await
    });
    wait_for("the async stack worker to start", || {
        started.load(Ordering::SeqCst) == 1
    })
    .await;
    assert_eq!(rt.snapshot().capabilities["large_stack"].in_use, 1);

    // A blocking stack request now has to wait for the same single slot.
    let rt_blocking = rt.clone();
    let blocking_started = Arc::clone(&started);
    let blocking_worker = tokio::spawn(async move {
        rt_blocking
            .run_blocking(
                TaskSpec::blocking(TaskClass::new("c"))
                    .operation("blocking-stack")
                    .stack_size_bytes(STACK),
                move || {
                    blocking_started.fetch_add(1, Ordering::SeqCst);
                    Ok::<i32, ()>(6)
                },
            )
            .await
    });
    wait_for("the blocking stack request to queue", || {
        rt.snapshot().classes[&TaskClass::new("c")].queued == 1
    })
    .await;
    assert_eq!(started.load(Ordering::SeqCst), 1, "the cap still binds");

    release.send(()).expect("async worker still waiting");
    assert_eq!(async_worker.await.expect("join").expect("async"), 5);
    assert_eq!(blocking_worker.await.expect("join").expect("blocking"), 6);
    assert_eq!(rt.snapshot().conservation_violation(), None);
}

#[tokio::test]
async fn a_hint_the_run_path_does_not_accept_is_rejected_before_any_wait() {
    // Resolution happens before admission, so a mismatch costs nothing.
    let rt = runtime();
    let io_spec = TaskSpec::io(TaskClass::new("c")).operation("wrong-path");
    let error = rt
        .run_blocking(io_spec, || Ok::<i32, ()>(1))
        .await
        .expect_err("an async-io spec is not a blocking submission");
    assert!(matches!(
        error,
        RunError::Governor(GovernorError::Rejected(AdmissionVerdict::SubstrateMismatch))
    ));
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}
