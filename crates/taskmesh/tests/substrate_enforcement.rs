//! Audit P1#5 + P1#2: substrate classification is authoritative (each run path
//! requires a matching hint) and topology slot counts are real capability-pool
//! capacity limits, not inert metadata.

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
async fn run_blocking_rejects_io_hinted_spec() {
    let rt = rt_unlimited();
    let io_spec = TaskSpec::io(TaskClass::new("c")).operation("x");
    let err = rt
        .run_blocking(io_spec, || Ok::<i32, ()>(1))
        .await
        .expect_err("io spec on blocking path must reject");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(AdmissionVerdict::SubstrateMismatch))
    ));
}

#[tokio::test]
async fn run_io_rejects_blocking_hinted_spec() {
    let rt = rt_unlimited();
    let blk = TaskSpec::blocking(TaskClass::new("c")).operation("x");
    let err = rt
        .run_io(blk, async { Ok::<i32, ()>(1) })
        .await
        .expect_err("blocking spec on io path must reject");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(AdmissionVerdict::SubstrateMismatch))
    ));
}

#[tokio::test]
async fn run_cpu_rejects_io_hinted_spec() {
    let rt = rt_unlimited();
    let io_spec = TaskSpec::io(TaskClass::new("c")).operation("x");
    let err = rt
        .run_cpu(io_spec, || Ok::<i32, ()>(1))
        .await
        .expect_err("io spec on cpu path must reject");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(AdmissionVerdict::SubstrateMismatch))
    ));
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
    let rt_h = rt.clone();
    let holder = tokio::spawn(async move {
        rt_h.run_blocking(
            TaskSpec::blocking(TaskClass::new("c")).operation("hold"),
            move || {
                let _ = hold.blocking_recv();
                Ok::<_, ()>(())
            },
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Second blocking submission must fail to acquire the substrate slot in time,
    // even though the governor would admit it (cap 8).
    let opts = SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(60));
    let err = rt
        .run_blocking_with(
            TaskSpec::blocking(TaskClass::new("c")).operation("blocked"),
            opts,
            || Ok::<i32, ()>(2),
        )
        .await
        .expect_err("blocking pool is full -> bounded acquire times out");
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::SubstratePoolTimedOut { .. }
        ))
    ));

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

#[tokio::test]
async fn zero_slots_means_unlimited() {
    // Default topology (all slot counts 0) imposes no substrate gate: blocking
    // work runs without a configured pool.
    let rt = rt_unlimited(); // default topology
    let out: i32 = rt
        .run_blocking(
            TaskSpec::blocking(TaskClass::new("c")).operation("x"),
            || Ok::<_, ()>(5),
        )
        .await
        .unwrap();
    assert_eq!(out, 5);
}
