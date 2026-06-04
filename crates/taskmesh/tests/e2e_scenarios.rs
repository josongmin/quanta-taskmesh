//! Hardcore operational e2e scenarios — "does the whole thing actually behave
//! under pressure?" These go beyond the per-feature proof gate (`e2e_proof.rs`):
//! concurrency storms, cancellation chaos, an accounting-conservation invariant,
//! backpressure forward-progress, bounded fail-closed overload, a multi-stage
//! composite pipeline, and a CPU soak through the Rayon executor.
//!
//! Robustness rules followed here: no assertions on cross-thread ordering (only
//! on completion, conservation, drain-to-zero, and forward progress), and every
//! randomized scenario is driven by a fixed-seed LCG so failures reproduce.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use taskmesh::ext::*;
use taskmesh::*;

fn cls(name: &str) -> TaskClass {
    TaskClass::new(name.to_string())
}

/// Deterministic PRNG so "random" scenarios reproduce exactly on failure.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
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

    let mut ok = 0usize;
    for h in handles {
        if h.await.unwrap().is_ok() {
            ok += 1;
        }
    }
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
    for h in handles {
        h.await.unwrap();
    }
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
    for h in handles {
        h.await.unwrap();
    }

    // Both paths must have been exercised (the scenario is meaningful).
    assert!(completed.load(Ordering::SeqCst) > 0, "some must complete");
    assert!(
        cancelled.load(Ordering::SeqCst) > 0,
        "some must be cancelled"
    );

    // Let any in-flight drops settle, then prove zero leak.
    tokio::time::sleep(Duration::from_millis(50)).await;
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
    let (a, b, c, d) = tokio::join!(io, blk, cpu, local);
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

    // Holder occupies the only slot for ~120ms, then releases.
    let (release, hold) = tokio::sync::oneshot::channel::<()>();
    let rt_h = rt.clone();
    let holder = tokio::spawn(async move {
        rt_h.run_blocking(TaskSpec::blocking(cls("c")).operation("hold"), move || {
            let _ = hold.blocking_recv();
            Ok::<_, ()>(())
        })
        .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Client retries on saturation; must succeed within a bounded number of tries.
    let rt_c = rt.clone();
    let client = tokio::spawn(async move {
        for attempt in 0..1000u32 {
            let spec = TaskSpec::blocking(cls("c")).operation(format!("try-{attempt}"));
            match rt_c.run_blocking(spec, move || Ok::<_, ()>(attempt)).await {
                Ok(_) => return attempt,
                Err(RunError::Governor(GovernorError::Rejected(
                    AdmissionVerdict::CpuSaturated { .. },
                ))) => {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                other => panic!("unexpected: {other:?}"),
            }
        }
        panic!("client never made progress");
    });

    // Release the holder so the client can get in.
    tokio::time::sleep(Duration::from_millis(80)).await;
    release.send(()).unwrap();

    holder.await.unwrap().unwrap();
    let attempts = client.await.unwrap();
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
                Err(RunError::Task(_)) => panic!("task error must not appear under overload"),
            };
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    let total = admitted.load(Ordering::SeqCst) + shed.load(Ordering::SeqCst);
    assert_eq!(total, 64, "every request must return a decision (no hang)");
    assert!(shed.load(Ordering::SeqCst) > 0, "overload must shed load");
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_drained(&rt, &["c"]);
}

// ───────────── 6. accounting conservation invariant (randomized) ─────────

/// The load-bearing invariant: after an arbitrary admit/release sequence, the
/// snapshot's per-class accounting must exactly equal the live permit set. Driven
/// by a fixed-seed LCG; checked after every single operation.
#[test]
fn accounting_conservation_under_randomized_ops() {
    let rt = Builder::new()
        .resources(
            ResourceBudget::new()
                .cpu_units(1_000_000)
                .memory_units(1_000_000),
        )
        .class_policy(cls("c"), ClassPolicy::new().cpu_units(2).memory_units(3))
        .build()
        .unwrap();
    let g = rt.governor();
    let c = cls("c");

    let mut held: Vec<u64> = Vec::new(); // permit ids
    let mut lcg = Lcg(0x0BAD_F00D_1234_5678);

    for step in 0..4000u64 {
        let do_admit = held.is_empty() || lcg.below(100) < 60;
        if do_admit {
            let op = format!("op-{step}");
            match g.admit(
                &TaskSpec::blocking(c.clone()).operation(op.clone()),
                RequestKey::new(op),
            ) {
                AdmissionDecision::Admitted { permit_id } => held.push(permit_id),
                other => panic!("unbounded admit must succeed at step {step}: {other:?}"),
            }
        } else {
            let idx = (lcg.next() as usize) % held.len();
            g.release(held.swap_remove(idx));
        }

        let snap = rt.snapshot();
        let cs = &snap.classes[&c];
        assert_eq!(cs.inflight as usize, held.len(), "inflight at step {step}");
        assert_eq!(
            cs.cpu_units_held,
            held.len() as u32 * 2,
            "cpu at step {step}"
        );
        assert_eq!(
            cs.memory_units_held,
            held.len() as u32 * 3,
            "mem at step {step}"
        );
    }

    for p in held {
        g.release(p);
    }
    assert_drained(&rt, &["c"]);
}

// ───────────── 7. multi-stage composite pipeline ─────────────────────────

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

    // Fan three children across distinct substrate stages under one root.
    let mut permits = Vec::new();
    for (hint, _label) in [
        (SubstrateHint::AsyncIo, "io"),
        (SubstrateHint::BlockingPool, "blocking"),
        (SubstrateHint::SharedCpuExecutor, "cpu"),
    ] {
        let child =
            TaskSpec::base(cls("worker"), hint).child_of("root-1", TaskStage::new("fanout"));
        match g.admit(&child, RequestKey::new("root-1")) {
            AdmissionDecision::Admitted { permit_id } => permits.push(permit_id),
            other => panic!("child must admit: {other:?}"),
        }
    }
    let root = g.root_attribution("root-1").expect("root tracked");
    assert_eq!(root.child_inflight, 3);
    assert_eq!(root.cpu_units, 6); // 3 children * 2 cpu units
    assert_eq!(root.active_stages, 3);

    // A fourth child re-entering an already-active stage is a recursive loop.
    let dup = TaskSpec::io(cls("worker")).child_of("root-1", TaskStage::new("fanout"));
    assert!(matches!(
        g.admit(&dup, RequestKey::new("root-1")),
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
        g.release(p);
    }
    assert!(g.root_attribution("root-1").is_none());
}

// ───────────── 8. memory reconcile lifecycle ─────────────────────────────

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
                .memory_permit_mode(MemoryPermitMode::Hybrid),
        )
        .build()
        .unwrap();
    let g = rt.governor();
    let h = cls("h");
    let mem = |rt: &TokioRuntime| rt.snapshot().classes[&cls("h")].memory_units_held;

    let p = match g.admit(
        &TaskSpec::blocking(h.clone()).operation("op"),
        RequestKey::new("op"),
    ) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("{o:?}"),
    };
    assert_eq!(mem(&rt), 5, "initial estimate reserved");

    assert!(g.reconcile_memory(p, 80)); // 80/10 = 8 -> max(5,8)=8
    assert_eq!(mem(&rt), 8);

    assert!(g.reconcile_memory(p, 20)); // 20/10 = 2 -> max(5,2)=5
    assert_eq!(mem(&rt), 5, "hybrid never drops below estimate");

    let freed = g.release_stage_memory(p, 2);
    assert_eq!(freed, 2);
    assert_eq!(mem(&rt), 3);
    assert_eq!(
        rt.snapshot().classes[&h].inflight,
        1,
        "stage release keeps the permit alive"
    );

    g.release(p);
    assert_drained(&rt, &["h"]);
}

// ───────────── 9. CPU soak through the Rayon executor ────────────────────

/// 96 concurrent CPU jobs through the shared Rayon pool, each computing a real
/// reduction; results must be correct and accounting must drain.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rayon_cpu_soak_results_correct_and_drains() {
    let topology = TopologyConfig::new().cpu_fixed(4);
    let rayon = Arc::new(taskmesh_rayon::RayonCpuExecutor::from_topology(&topology));
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

    for (i, h) in handles.into_iter().enumerate() {
        let expected: u64 = (0..1000u64).map(|x| x ^ i as u64).sum();
        assert_eq!(h.await.unwrap().unwrap(), expected, "cpu job {i} result");
    }
    assert_drained(&rt, &["cpu"]);
}

// ───────────── 10. sequential soak (slow-leak detector) ──────────────────

/// 3000 sequential submissions. Catches slow leaks / monotonic drift: the final
/// state must be perfectly clean.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sequential_soak_stays_clean() {
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(64).memory_units(64))
        .class_policy(
            cls("c"),
            ClassPolicy::new()
                .max_inflight(4)
                .max_queue_depth(64)
                .cpu_units(1),
        )
        .build()
        .unwrap();

    for i in 0..3000u64 {
        let spec = TaskSpec::io(cls("c")).operation(format!("soak-{i}"));
        let out: u64 = rt
            .run_io(spec, async move { Ok::<_, ()>(i) })
            .await
            .unwrap();
        assert_eq!(out, i);
    }
    assert_drained(&rt, &["c"]);
}
