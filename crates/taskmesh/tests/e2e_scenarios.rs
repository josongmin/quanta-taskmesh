//! Hardcore operational e2e scenarios — "does the whole thing actually behave
//! under pressure?" These go beyond the per-feature proof gate (`e2e_proof.rs`):
//! concurrency storms, cancellation chaos, backpressure forward-progress,
//! bounded fail-closed overload, a multi-stage composite pipeline, and a CPU soak
//! through the Rayon executor.
//!
//! Robustness rules followed here: no assertions on cross-thread ordering (only
//! on completion, conservation, drain-to-zero, and forward progress), and every
//! randomized scenario is driven by a fixed-seed LCG so failures reproduce.

use std::sync::atomic::{AtomicUsize, Ordering};
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

/// Deterministic PRNG so "random" scenarios reproduce exactly on failure.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
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

// ───────────────────────── 1. concurrency storm ─────────────────────────

/// A production-like 3-class mix hammered by 240 concurrent submissions across
/// real worker threads. Every request must complete (forward progress through
/// queue→promote) and all accounting must drain to zero.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn high_concurrency_multiclass_drains_clean() {
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            cls("retrieval"),
            ClassPolicy::new()
                .max_inflight(4)
                .max_queue_depth(512)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .class_policy(
            cls("rank"),
            ClassPolicy::new()
                .max_inflight(2)
                .max_queue_depth(512)
                .cpu_units(2)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .class_policy(
            cls("index"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(512)
                .cpu_units(1)
                .best_effort(true)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .unwrap();

    let names = ["retrieval", "rank", "index"];
    let mut handles = Vec::new();
    for i in 0..240usize {
        let rt = rt.clone();
        let name = names[i % 3];
        handles.push(tokio::spawn(async move {
            let spec = TaskSpec::io(cls(name)).operation(format!("{name}-{i}"));
            rt.run_io(spec, async move { Ok::<_, ()>(i) }).await
        }));
    }

    let ok = bounded_storm("high-concurrency multiclass storm", async move {
        let mut ok = 0usize;
        for handle in handles {
            if handle.await.unwrap().is_ok() {
                ok += 1;
            }
        }
        ok
    })
    .await;
    assert_eq!(ok, 240, "every queued submission must eventually complete");
    assert_drained(&rt, &names);
}

/// A single-slot class flooded by 128 concurrent submissions: serialization is
/// total, yet every one must drain through the queue with no deadlock or leak.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_slot_storm_serializes_without_deadlock() {
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            cls("solo"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(1024)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .unwrap();

    let done = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for i in 0..128usize {
        let rt = rt.clone();
        let done = done.clone();
        handles.push(tokio::spawn(async move {
            let spec = TaskSpec::blocking(cls("solo")).operation(format!("s-{i}"));
            let r: Result<usize, RunError<()>> = rt.run_blocking(spec, move || Ok(i)).await;
            if r.is_ok() {
                done.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }
    bounded_storm("single-slot serialization storm", async move {
        for handle in handles {
            handle.await.unwrap();
        }
    })
    .await;
    assert_eq!(done.load(Ordering::SeqCst), 128);
    assert_drained(&rt, &["solo"]);
}

// ───────────────────────── 2. cancellation chaos ────────────────────────

/// Spawn a churn of submissions, cancel a deterministic ~half of them mid-flight
/// via outer timeouts, and prove the governor never leaks: after everything
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
        // Deterministic per-task work/deadline so the run reproduces.
        let mut lcg = Lcg(0xDEAD_BEEF_0000_0000 ^ i as u64);
        let work_ms = lcg.below(20);
        let deadline_ms = 1 + lcg.below(15);
        handles.push(tokio::spawn(async move {
            let spec = TaskSpec::io(cls("c")).operation(format!("c-{i}"));
            let fut = rt.run_io(spec, async move {
                tokio::time::sleep(Duration::from_millis(work_ms)).await;
                Ok::<_, ()>(i)
            });
            match tokio::time::timeout(Duration::from_millis(deadline_ms), fut).await {
                Ok(Ok(_)) => completed.fetch_add(1, Ordering::SeqCst),
                _ => cancelled.fetch_add(1, Ordering::SeqCst),
            };
        }));
    }
    bounded_storm("cancellation chaos task group", async move {
        for handle in handles {
            handle.await.unwrap();
        }
    })
    .await;

    // Both paths must have been exercised (the scenario is meaningful).
    assert!(completed.load(Ordering::SeqCst) > 0, "some must complete");
    assert!(
        cancelled.load(Ordering::SeqCst) > 0,
        "some must be cancelled"
    );

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

// ─────────────────── 3. all substrates concurrently ─────────────────────

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
    let (a, b, c, d) = bounded_storm("all-substrate join group", async {
        tokio::join!(io, blk, cpu, local)
    })
    .await;
    assert_eq!(a.unwrap(), "io");
    assert_eq!(b.unwrap(), "blk");
    assert_eq!(c.unwrap(), "cpu");
    assert_eq!(d.unwrap(), "local");
    assert_drained(&rt, &["svc"]);
}

// ─────────────────── 4. backpressure forward progress ───────────────────

/// A client that retries on a bounded acquire timeout must eventually make
/// progress once a long-held slot frees — the backpressure contract is usable.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn backpressure_retry_makes_forward_progress() {
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(64).memory_units(64))
        .class_policy(
            cls("c"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(0) // not queueable -> immediate CpuSaturated
                .cpu_units(1),
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

    // The first rejection proves saturation; only then release the holder and
    // permit the client to retry. This has no scheduler-delay assumption.
    let (saturated_tx, saturated_rx) = tokio::sync::oneshot::channel::<()>();
    let (retry_tx, retry_rx) = tokio::sync::oneshot::channel::<()>();
    let rt_c = rt.clone();
    let client = tokio::spawn(async move {
        let mut saturated_tx = Some(saturated_tx);
        let mut retry_rx = Some(retry_rx);
        let mut attempt = 0u32;
        loop {
            let spec = TaskSpec::blocking(cls("c")).operation(format!("try-{attempt}"));
            match rt_c.run_blocking(spec, move || Ok::<_, ()>(attempt)).await {
                Ok(_) => return attempt,
                Err(RunError::Governor(GovernorError::Rejected(
                    AdmissionVerdict::CpuSaturated { .. },
                ))) => {
                    if let Some(saturated_tx) = saturated_tx.take() {
                        saturated_tx
                            .send(())
                            .expect("test observes the first saturation");
                        retry_rx
                            .take()
                            .expect("retry gate exists once")
                            .await
                            .expect("test permits retry after releasing holder");
                    } else {
                        tokio::task::yield_now().await;
                    }
                }
                other => panic!("unexpected: {other:?}"),
            }
            attempt = attempt
                .checked_add(1)
                .expect("attempt counter cannot overflow");
        }
    });

    bounded(
        "backpressure_retry_makes_forward_progress: client was never saturated",
        saturated_rx,
    )
    .await
    .expect("saturation signal sender survives");
    release.send(()).unwrap();
    retry_tx
        .send(())
        .expect("client waits for retry permission");

    let (holder_result, client_result) = bounded_storm(
        "backpressure_retry_makes_forward_progress: released task group did not finish",
        async { tokio::join!(holder, client) },
    )
    .await;
    holder_result.unwrap().unwrap();
    let attempts = client_result.expect("client task must not panic");
    assert!(attempts > 0, "client should have hit saturation first");
    assert_drained(&rt, &["c"]);
}

// ───────────── 5. bounded, fail-closed overload under concurrency ─────────

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

// ───────────── 6. multi-stage composite pipeline ─────────────────────────

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
    assert!(Governor::validate_reduce(&bad).is_err());
    let good = TaskSpec::cpu(cls("worker")).reduce_stage(
        TaskStage::new("merge"),
        SubstrateHint::SharedCpuExecutor,
        DeterministicReducePolicy::keyed("doc_id"),
    );
    assert!(Governor::validate_reduce(&good).is_ok());

    // Releasing all children clears the root attribution.
    for p in permits {
        assert_eq!(g.release(p), ReleaseOutcome::Released);
    }
    assert!(g.root_attribution("root-1").is_none());
}

// ───────────── 7. memory reconcile lifecycle ─────────────────────────────

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

// ───────────── 8. CPU soak through the Rayon executor ────────────────────

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
