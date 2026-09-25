//! Audit P1#5 + P1#2: substrate classification is authoritative (each run path
//! requires a matching hint) and topology slot counts are real capability-pool
//! capacity limits, not inert metadata.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use taskmesh::*;

fn rt_unlimited() -> TokioRuntime {
    Builder::new()
        .resources(ResourceBudget::new().cpu_units(64).memory_units(64))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(8)
                .max_queue_depth(64)
                .cpu_units(1),
        )
        .build()
        .unwrap()
}

// ---- P1#5: hint ↔ run-path enforcement ------------------------------------

#[tokio::test]
async fn run_blocking_rejects_nonblocking_hints() {
    let rt = rt_unlimited();
    for (hint, spec) in [
        ("io", TaskSpec::io(TaskClass::new("c"))),
        ("cpu", TaskSpec::cpu(TaskClass::new("c"))),
    ] {
        let ran = Arc::new(AtomicBool::new(false));
        let ran_in_work = Arc::clone(&ran);
        let err = rt
            .run_blocking(spec.operation(hint), move || {
                ran_in_work.store(true, Ordering::SeqCst);
                Ok::<i32, ()>(1)
            })
            .await
            .expect_err("nonblocking hint on blocking path must reject");
        assert!(
            matches!(
                err,
                RunError::Governor(GovernorError::Rejected(AdmissionVerdict::SubstrateMismatch))
            ),
            "{hint} spec on blocking path must reject"
        );
        assert!(!ran.load(Ordering::SeqCst));
        assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
    }
}

#[tokio::test]
async fn run_io_rejects_blocking_hinted_spec() {
    let rt = rt_unlimited();
    let blk = TaskSpec::blocking(TaskClass::new("c")).operation("x");
    let ran = Arc::new(AtomicBool::new(false));
    let ran_in_work = Arc::clone(&ran);
    let err = rt
        .run_io(blk, async move {
            ran_in_work.store(true, Ordering::SeqCst);
            Ok::<i32, ()>(1)
        })
        .await
        .expect_err("blocking spec on io path must reject");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(AdmissionVerdict::SubstrateMismatch))
    ));
    assert!(!ran.load(Ordering::SeqCst));
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}

#[tokio::test]
async fn run_cpu_rejects_io_hinted_spec() {
    let rt = rt_unlimited();
    let io_spec = TaskSpec::io(TaskClass::new("c")).operation("x");
    let ran = Arc::new(AtomicBool::new(false));
    let ran_in_work = Arc::clone(&ran);
    let err = rt
        .run_cpu(io_spec, move || {
            ran_in_work.store(true, Ordering::SeqCst);
            Ok::<i32, ()>(1)
        })
        .await
        .expect_err("io spec on cpu path must reject");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(AdmissionVerdict::SubstrateMismatch))
    ));
    assert!(!ran.load(Ordering::SeqCst));
    assert_eq!(rt.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}

#[tokio::test]
async fn run_blocking_accepts_large_stack_hint() {
    let rt = rt_unlimited();
    let big =
        TaskSpec::base(TaskClass::new("c"), SubstrateHint::LargeStackCapability).operation("x");
    let out: i32 = rt.run_blocking(big, || Ok::<_, ()>(7)).await.unwrap();
    assert_eq!(out, 7);
}

// ---- P1#2: topology slot counts are real capacity pools -------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blocking_threads_caps_substrate_concurrency() {
    // One blocking slot; class inflight is generous, so the SUBSTRATE pool is the
    // binding constraint (not the governor permit).
    let rt = Builder::new()
        .topology(TopologyConfig::new().blocking_threads(1))
        .resources(ResourceBudget::new().cpu_units(64).memory_units(64))
        .class_policy(
            TaskClass::new("c"),
            ClassPolicy::new()
                .max_inflight(8)
                .max_queue_depth(64)
                .cpu_units(1),
        )
        .build()
        .unwrap();

    // Holder occupies the single blocking slot until released.
    let (release, hold) = tokio::sync::oneshot::channel::<()>();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let rt_h = rt.clone();
    let holder = tokio::spawn(async move {
        rt_h.run_blocking(
            TaskSpec::blocking(TaskClass::new("c")).operation("hold"),
            move || {
                started_tx.send(()).expect("test observes holder start");
                // A signal or a dropped sender both release the holder: a test that fails
                // before signalling never hangs on its own fixture.
                let _released = hold.blocking_recv();
                Ok::<_, ()>(())
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(1), started_rx)
        .await
        .expect("holder starts within the test bound")
        .expect("holder start sender survives");

    // The second blocking submission cannot have a worker. The class here does
    // not queue (`OverflowPolicy::Reject`), and capability occupancy is decided
    // by the same admission transition as class capacity — so it is shed now,
    // with the cause named, instead of parking in an unbounded holding area the
    // class's own overflow policy never governed.
    let opts = SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(60));
    let err = rt
        .run_blocking_with(
            TaskSpec::blocking(TaskClass::new("c")).operation("blocked"),
            opts,
            || Ok::<i32, ()>(2),
        )
        .await
        .expect_err("blocking pool is full -> non-queueing class sheds");
    assert!(
        matches!(
            err,
            RunError::Governor(GovernorError::Rejected(
                AdmissionVerdict::SubstrateSaturated { .. }
            ))
        ),
        "expected a capability-pool shed, got {err:?}"
    );
    // The rejection is visible in the same snapshot the class is governed by:
    // the shed request is not hiding in a queue the class did not opt into.
    let observed = rt.snapshot();
    assert_eq!(observed.classes[&TaskClass::new("c")].queued, 0);
    assert_eq!(observed.capabilities["blocking"].in_use, 1);
    assert_eq!(observed.capabilities["blocking"].limit, 1);

    release.send(()).unwrap();
    holder.await.unwrap().unwrap();

    // After the holder frees the slot, a fresh blocking submission succeeds.
    let out: i32 = rt
        .run_blocking(
            TaskSpec::blocking(TaskClass::new("c")).operation("after"),
            || Ok::<_, ()>(9),
        )
        .await
        .unwrap();
    assert_eq!(out, 9);
}
