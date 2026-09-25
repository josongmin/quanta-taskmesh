//! Hardcore operational e2e scenarios — "does the whole thing actually behave
//! under pressure?" These go beyond the per-feature proof gate (`e2e_proof.rs`):
//! cancellation chaos, backpressure forward-progress, bounded fail-closed
//! overload, a multi-stage composite pipeline, and a CPU soak through the Rayon
//! executor. Cross-class contention and mixed-substrate soak live in
//! `e2e_chaos.rs`.
//!
//! Robustness rules followed here: no assertions on cross-thread ordering (only
//! on completion, conservation, drain-to-zero, and forward progress), and
//! timing-sensitive scenarios use explicit completion and cancellation cohorts.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use taskmesh::ext::*;
use taskmesh::*;

const HANG: Duration = Duration::from_secs(5);
const STORM_TIMEOUT: Duration = Duration::from_secs(30);

async fn bounded<F: std::future::Future>(what: &str, future: F) -> F::Output {
    let Ok(output) = tokio::time::timeout(HANG, future).await else {
        panic!("{what}: timed out after {HANG:?}");
    };
    output
}

async fn bounded_storm<F: std::future::Future>(what: &str, future: F) -> F::Output {
    let Ok(output) = tokio::time::timeout(STORM_TIMEOUT, future).await else {
        panic!("{what}: timed out after {STORM_TIMEOUT:?}");
    };
    output
}

fn cls(name: &str) -> TaskClass {
    TaskClass::new(name.to_string())
}

fn assert_drained(rt: &TokioRuntime, classes: &[&str]) {
    let snap = rt.snapshot();
    for name in classes {
        let c = &snap.classes[&cls(name)];
        assert_eq!(
            (c.inflight, c.queued, c.cpu_units_held, c.memory_units_held),
            (0, 0, 0, 0),
            "class {name} did not drain clean: {c:?}"
        );
    }
}

// ───────────────────────── 1. cancellation chaos ────────────────────────

/// Spawn a churn of submissions, cancel exactly half of them mid-flight via
/// outer timeouts, and prove the governor never leaks: after everything
/// settles, all accounting is zero and the runtime still serves fresh work.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancellation_chaos_never_leaks() {
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            cls("c"),
            ClassPolicy::new()
                .max_inflight(4)
                .max_queue_depth(512)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .unwrap();

    let completed = Arc::new(AtomicUsize::new(0));
    let cancelled = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    for i in 0..160usize {
        let rt = rt.clone();
        let completed = completed.clone();
        let cancelled = cancelled.clone();
        // Keep both outcomes meaningful under host contention. Completion is
        // not decided by wall-clock speed; only the cancellation cohort uses
        // a timeout, against work that cannot finish inside that budget.
        let should_cancel = i % 2 != 0;
        handles.push(tokio::spawn(async move {
            let spec = TaskSpec::io(cls("c")).operation(format!("c-{i}"));
            let fut = rt.run_io(spec, async move {
                if should_cancel {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Ok::<_, ()>(i)
            });
            if should_cancel {
                match tokio::time::timeout(Duration::from_millis(1), fut).await {
                    Err(_) => cancelled.fetch_add(1, Ordering::SeqCst),
                    Ok(Ok(value)) => panic!("cancellation cohort completed as {value}"),
                    Ok(Err(error)) => panic!("unexpected runtime failure: {error:?}"),
                };
            } else {
                match fut.await {
                    Ok(_) => completed.fetch_add(1, Ordering::SeqCst),
                    Err(error) => panic!("unexpected runtime failure: {error:?}"),
                };
            }
        }));
    }
    bounded_storm("cancellation chaos task group", async move {
        for handle in handles {
            handle.await.unwrap();
        }
    })
    .await;

    assert_eq!(completed.load(Ordering::SeqCst), 80);
    assert_eq!(cancelled.load(Ordering::SeqCst), 80);

    // Each submission future has returned; its permit release is part of that
    // completion contract. A delay here would hide a late-release regression.
    assert_drained(&rt, &["c"]);

    // Runtime is still fully usable after the chaos.
    let out: i32 = rt
        .run_io(TaskSpec::io(cls("c")).operation("after"), async {
            Ok::<_, ()>(7)
        })
        .await
        .unwrap();
    assert_eq!(out, 7);
}

// ─────────────────── 2. all substrates concurrently ─────────────────────

/// Drive all four substrates at once on one runtime; each lands on its path and
/// returns its own value, with the governor/task error split intact.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_substrates_concurrently() {
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(64).memory_units(64))
        .class_policy(
            cls("svc"),
            ClassPolicy::new()
                .max_inflight(8)
                .max_queue_depth(64)
                .cpu_units(1),
        )
        .build()
        .unwrap();

    let io = rt.run_io(TaskSpec::io(cls("svc")).operation("io"), async {
        Ok::<_, ()>("io")
    });
    let blk = rt.run_blocking(TaskSpec::blocking(cls("svc")).operation("blk"), || {
        Ok::<_, ()>("blk")
    });
    let cpu = rt.run_cpu(TaskSpec::cpu(cls("svc")).operation("cpu"), || {
        Ok::<_, ()>("cpu")
    });
    let local = rt.run_local(TaskSpec::local(cls("svc")).operation("local"), async {
        // !Send work proves the local path.
        let rc = std::rc::Rc::new("local");
        Ok::<_, ()>(*rc)
    });

    // `join!` polls the non-Send local future on this task (no spawn needed).
    // Boxed: the joined run futures exceed the large-future stack budget.
    let (a, b, c, d) = bounded_storm(
        "all-substrate join group",
        Box::pin(async { tokio::join!(io, blk, cpu, local) }),
    )
    .await;
    assert_eq!(a.unwrap(), "io");
    assert_eq!(b.unwrap(), "blk");
    assert_eq!(c.unwrap(), "cpu");
    assert_eq!(d.unwrap(), "local");
    assert_drained(&rt, &["svc"]);
}

// ─────────────────── 3. backpressure forward progress ───────────────────

/// A rejected submission must make progress on its first retry after the
/// holder has returned its lease.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn backpressure_retry_makes_forward_progress() {
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(64).memory_units(64))
        .class_policy(
            cls("c"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(0)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::Reject),
        )
        .build()
        .unwrap();

    // Holder occupies the only slot until the test releases it.
    let (release, hold) = tokio::sync::oneshot::channel::<()>();
    let (holder_started_tx, holder_started_rx) = tokio::sync::oneshot::channel::<()>();
    let rt_h = rt.clone();
    let holder = tokio::spawn(async move {
        rt_h.run_blocking(TaskSpec::blocking(cls("c")).operation("hold"), move || {
            holder_started_tx
                .send(())
                .expect("test observes holder admission");
            // A signal or a dropped sender both release the holder: a test that fails
            // before signalling never hangs on its own fixture.
            let _released = hold.blocking_recv();
            Ok::<_, ()>(())
        })
        .await
    });
    bounded(
        "backpressure_retry_makes_forward_progress: holder was never admitted",
        holder_started_rx,
    )
    .await
    .expect("holder start sender survives");

    let rejected_work_ran = Arc::new(AtomicBool::new(false));
    let rejected_work_ran_in_closure = Arc::clone(&rejected_work_ran);
    let first = bounded(
        "backpressure_retry_makes_forward_progress: first submission did not reject",
        rt.run_blocking(TaskSpec::blocking(cls("c")).operation("first"), move || {
            rejected_work_ran_in_closure.store(true, Ordering::SeqCst);
            Ok::<_, ()>(1)
        }),
    )
    .await;
    assert!(matches!(
        first,
        Err(RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::CpuSaturated { .. }
        )))
    ));
    assert!(!rejected_work_ran.load(Ordering::SeqCst));

    release.send(()).expect("holder waits for test release");
    bounded(
        "backpressure_retry_makes_forward_progress: holder did not finish after release",
        holder,
    )
    .await
    .unwrap()
    .unwrap();
    let class = &rt.snapshot().classes[&cls("c")];
    assert_eq!((class.inflight, class.queued), (0, 0));

    let retry = bounded(
        "backpressure_retry_makes_forward_progress: retry did not finish after holder completion",
        rt.run_blocking(TaskSpec::blocking(cls("c")).operation("retry"), || {
            Ok::<_, ()>(42)
        }),
    )
    .await
    .expect("first retry after holder completion must succeed");
    assert_eq!(retry, 42);
    assert_drained(&rt, &["c"]);
}

// ───────────── 4. bounded, fail-closed overload under concurrency ─────────

/// 64 concurrent submissions onto a 1-slot / depth-4 class with a short acquire
/// timeout. Every outcome is success or a *governor-side* rejection/timeout —
/// never a task error, never a hang — and the queue bound is respected (no leak).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn overload_is_bounded_and_fail_closed() {
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            cls("c"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(4)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .unwrap();

    let admitted = Arc::new(AtomicUsize::new(0));
    let shed = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for i in 0..64usize {
        let rt = rt.clone();
        let admitted = admitted.clone();
        let shed = shed.clone();
        handles.push(tokio::spawn(async move {
            let spec = TaskSpec::io(cls("c")).operation(format!("o-{i}"));
            let opts = SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(40));
            let r = rt
                .run_io_with(spec, opts, async move {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    Ok::<_, ()>(i)
                })
                .await;
            match r {
                Ok(_) => admitted.fetch_add(1, Ordering::SeqCst),
                Err(RunError::Governor(_)) => shed.fetch_add(1, Ordering::SeqCst),
                Err(RunError::Task(())) => panic!("task error must not appear under overload"),
            };
        }));
    }
    bounded_storm("bounded overload task group", async move {
        for handle in handles {
            handle.await.unwrap();
        }
    })
    .await;

    let total = admitted.load(Ordering::SeqCst) + shed.load(Ordering::SeqCst);
    assert_eq!(total, 64, "every request must return a decision (no hang)");
    assert!(shed.load(Ordering::SeqCst) > 0, "overload must shed load");
    assert_drained(&rt, &["c"]);
}

// ───────────── 5. multi-stage composite pipeline ─────────────────────────

/// A realistic composite: a root fans into children across distinct stages;
/// attribution rolls up, the recursion guard rejects a same-stage re-entry, and
/// reduce validation gates a fan-out spec missing its policy.
#[test]
fn composite_pipeline_attribution_recursion_and_reduce() {
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            cls("worker"),
            ClassPolicy::new()
                .max_inflight(64)
                .cpu_units(2)
                .memory_units(1),
        )
        .build()
        .unwrap();
    let g = rt.governor();

    // Fan three children across distinct declared lineage stages under one root.
    let mut permits = Vec::new();
    for (hint, label) in [
        (SubstrateHint::AsyncIo, "io"),
        (SubstrateHint::BlockingPool, "blocking"),
        (SubstrateHint::SharedCpuExecutor, "cpu"),
    ] {
        let child = TaskSpec::base(cls("worker"), hint)
            .child_of("root-1", "root-1", TaskStage::new(label.to_string()))
            .operation(format!("child-{label}"));
        match g.admit(&child) {
            AdmissionDecision::Admitted { permit_id } => permits.push(permit_id),
            other => panic!("child must admit: {other:?}"),
        }
    }
    let root = g.root_attribution("root-1").expect("root tracked");
    assert_eq!(root.child_inflight, 3);
    assert_eq!(root.cpu_units, 6); // 3 children * 2 cpu units
    assert_eq!(root.active_stages, 3);

    // A fourth child re-entering an already-active stage ("io") is a recursive loop.
    let dup = TaskSpec::io(cls("worker"))
        .child_of("root-1", "root-1", TaskStage::new("io"))
        .operation("duplicate-io-child");
    assert!(matches!(
        g.admit(&dup),
        AdmissionDecision::Rejected(AdmissionVerdict::RecursiveAdmission)
    ));

    // Reduce enforcement: fan-out without a policy is unshippable.
    let bad = TaskSpec::cpu(cls("worker"))
        .fan_out_stage(TaskStage::new("merge"), SubstrateHint::SharedCpuExecutor);
    assert_eq!(
        Governor::validate_reduce(&bad),
        Err(GovernorError::PolicyViolation(
            "parallel stage merge requires a deterministic reduce policy".into()
        ))
    );
    let good = TaskSpec::cpu(cls("worker")).reduce_stage(
        TaskStage::new("merge"),
        SubstrateHint::SharedCpuExecutor,
        DeterministicReducePolicy::keyed("doc_id"),
    );
    assert_eq!(Governor::validate_reduce(&good), Ok(()));

    // Releasing all children clears the root attribution.
    for p in permits {
        assert_eq!(g.release(p), ReleaseOutcome::Released);
    }
    assert!(g.root_attribution("root-1").is_none());
}

// ───────────── 6. memory reconcile lifecycle ─────────────────────────────

/// Hybrid memory across its full lifecycle: reserve estimate, reconcile up to
/// measured, reconcile back down (held never drops below the estimate), stage
/// boundary partial release, then full release — accounting consistent at each.
#[test]
fn memory_reconcile_lifecycle_is_consistent() {
    let rt = Builder::new()
        .resources(
            ResourceBudget::new()
                .cpu_units(1000)
                .memory_units(10_000)
                .memory_unit_scale(10),
        )
        .class_policy(
            cls("h"),
            ClassPolicy::new()
                .max_inflight(8)
                .memory_units(5)
                .memory_permit_mode(MemoryPermitMode::Hybrid)
                .memory_release_policy(MemoryReleasePolicy::OnStageBoundary),
        )
        .build()
        .unwrap();
    let g = rt.governor();
    let h = cls("h");
    let mem = |rt: &TokioRuntime| rt.snapshot().classes[&cls("h")].memory_units_held;

    let p = match g.admit(&TaskSpec::blocking(h.clone()).operation("op")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("{o:?}"),
    };
    assert_eq!(mem(&rt), 5, "initial estimate reserved");

    assert!(g.reconcile_memory(p, 80).is_applied()); // 80/10 = 8 -> max(5,8)=8
    assert_eq!(mem(&rt), 8);

    assert!(g.reconcile_memory(p, 20).is_applied()); // 20/10 = 2 -> max(5,2)=5
    assert_eq!(mem(&rt), 5, "hybrid never drops below estimate");

    let freed = g.release_stage_memory(p, 2);
    assert_eq!(freed, StageReleaseOutcome::Released { freed_units: 2 });
    assert_eq!(mem(&rt), 3);
    assert_eq!(
        rt.snapshot().classes[&h].inflight,
        1,
        "stage release keeps the permit alive"
    );

    assert_eq!(g.release(p), ReleaseOutcome::Released);
    assert_drained(&rt, &["h"]);
}

// ───────────── 7. CPU soak through the Rayon executor ────────────────────

/// 96 concurrent CPU jobs through the shared Rayon pool, each computing a real
/// reduction; results must be correct and accounting must drain.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rayon_cpu_soak_results_correct_and_drains() {
    let topology = TopologyConfig::new().cpu_fixed(4);
    let rayon = Arc::new(
        taskmesh_rayon::RayonCpuExecutor::try_from_topology(&topology).expect("rayon builds"),
    );
    let rt = Builder::new()
        .topology(topology)
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            cls("cpu"),
            ClassPolicy::new()
                .max_inflight(4)
                .max_queue_depth(256)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .cpu_executor(rayon)
        .build()
        .unwrap();

    let mut handles = Vec::new();
    for i in 0..96u64 {
        let rt = rt.clone();
        handles.push(tokio::spawn(async move {
            let spec = TaskSpec::cpu(cls("cpu")).operation(format!("sum-{i}"));
            rt.run_cpu(spec, move || {
                // A genuine CPU reduction, deterministic per i.
                let s: u64 = (0..1000).map(|x| x ^ i).sum();
                Ok::<_, ()>(s)
            })
            .await
        }));
    }

    bounded_storm("Rayon CPU task group", async move {
        for (i, handle) in handles.into_iter().enumerate() {
            let expected: u64 = (0..1000u64).map(|x| x ^ i as u64).sum();
            assert_eq!(
                handle.await.unwrap().unwrap(),
                expected,
                "cpu job {i} result"
            );
        }
    })
    .await;
    assert_drained(&rt, &["cpu"]);
}
