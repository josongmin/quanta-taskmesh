//! Maximum-difficulty operational chaos e2e. These go past `e2e_scenarios.rs`:
//! cross-class contention on a *shared global budget* (not just per-class caps),
//! a mixed-substrate soak with a concurrent leak-sweeper hammering the same
//! state, and an adversarial claim/timeout/abandon race storm that pounds the
//! exact promotion window the `TicketGuard` protects.
//!
//! Invariants asserted are scheduling-independent: forward progress (no
//! deadlock), fail-closed bounds, and drain-to-exactly-zero. No cross-thread
//! ordering is asserted; randomized timing is LCG-seeded for reproducibility.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use taskmesh::*;

fn cls(name: &str) -> TaskClass {
    TaskClass::new(name.to_string())
}

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

// ───────── 1. cross-class contention on a shared GLOBAL budget ─────────────

/// The bottleneck is the *global* cpu budget, not any per-class cap: three
/// classes (each cost 2) share a budget of 4, so at most two permits are
/// inflight system-wide regardless of class. 300 concurrent submissions must all
/// make forward progress through global-budget-driven cross-class promotion and
/// drain to zero — a deadlock or lost wakeup would hang or strand here.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn global_budget_cross_class_contention_makes_progress() {
    let policy = || {
        ClassPolicy::new()
            .max_inflight(1000) // per-class cap is NOT the bottleneck
            .max_queue_depth(4096)
            .cpu_units(2)
            .overflow_policy(OverflowPolicy::QueueWithinDepth)
    };
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(4).memory_units(1_000_000)) // 4/2 = 2 inflight max
        .class_policy(cls("alpha"), policy())
        .class_policy(cls("beta"), policy())
        .class_policy(cls("gamma"), policy())
        .build()
        .unwrap();

    let names = ["alpha", "beta", "gamma"];
    let done = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for i in 0..300usize {
        let rt = rt.clone();
        let done = done.clone();
        let name = names[i % 3];
        handles.push(tokio::spawn(async move {
            let spec = TaskSpec::io(cls(name)).operation(format!("{name}-{i}"));
            if rt.run_io(spec, async move { Ok::<_, ()>(i) }).await.is_ok() {
                done.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    assert_eq!(
        done.load(Ordering::SeqCst),
        300,
        "all must progress under global contention"
    );
    assert_drained(&rt, &names);
}

// ───────── 2. mixed-substrate soak with a concurrent leak-sweeper ──────────

/// A churn of io/blocking/cpu submissions across heterogeneous classes (one
/// memory-overcommit-queue class, one tiny-cap, one best-effort), with a
/// background task hammering `reap_leaks` + `snapshot` on the *same* governor
/// state throughout. The sweep must never corrupt live accounting (guarded by
/// the engine's debug `assert_consistent`) and everything must drain.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mixed_substrate_soak_with_concurrent_sweeper() {
    let rt = Builder::new()
        .resources(
            ResourceBudget::new()
                .cpu_units(64)
                .memory_units(256)
                .memory_unit_scale(64),
        )
        .class_policy(
            cls("io"),
            ClassPolicy::new()
                .max_inflight(8)
                .max_queue_depth(256)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .class_policy(
            cls("heavy"),
            ClassPolicy::new()
                .max_inflight(2)
                .max_queue_depth(256)
                .cpu_units(2)
                .memory_units(4)
                .memory_overcommit_policy(MemoryOvercommitPolicy::Queue)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .class_policy(
            cls("batch"),
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(256)
                .cpu_units(1)
                .best_effort(true)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .unwrap();

    // Background sweeper: read + reap concurrently with the storm. With the
    // default staleness window it reclaims nothing live, but it must not corrupt.
    let stop = Arc::new(AtomicBool::new(false));
    let sweeper = {
        let rt = rt.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            let mut sweeps = 0u64;
            while !stop.load(Ordering::Relaxed) {
                let _ = rt.governor().reap_leaks();
                let _ = rt.snapshot();
                sweeps += 1;
                tokio::task::yield_now().await;
            }
            sweeps
        })
    };

    let names = ["io", "heavy", "batch"];
    let mut handles = Vec::new();
    for i in 0..360usize {
        let rt = rt.clone();
        let mut lcg = Lcg(0xA5A5_0000_0000_0000 ^ i as u64);
        handles.push(tokio::spawn(async move {
            let name = names[(lcg.below(3)) as usize];
            let work = lcg.below(8);
            // Vary substrate by class for realism.
            match name {
                "io" => {
                    let spec = TaskSpec::io(cls(name)).operation(format!("{name}-{i}"));
                    let _ = rt
                        .run_io(spec, async move {
                            tokio::time::sleep(Duration::from_millis(work)).await;
                            Ok::<_, ()>(i)
                        })
                        .await;
                }
                "heavy" => {
                    let spec = TaskSpec::cpu(cls(name)).operation(format!("{name}-{i}"));
                    let _ = rt.run_cpu(spec, move || Ok::<_, ()>(i)).await;
                }
                _ => {
                    let spec = TaskSpec::blocking(cls(name)).operation(format!("{name}-{i}"));
                    let _ = rt.run_blocking(spec, move || Ok::<_, ()>(i)).await;
                }
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    let sweeps = sweeper.await.unwrap();
    assert!(sweeps > 0, "sweeper must have run concurrently");

    assert_drained(&rt, &names);
}

// ───────── 3. adversarial claim / timeout / abandon race storm ─────────────

/// Pound the exact promotion window: a single-slot class with many concurrent
/// submissions whose acquire timeouts are tuned to fire *around* when promotion
/// happens — maximally stressing the claim-vs-timeout-vs-abandon race the
/// `TicketGuard` guards. Every submission must return a decision (no hang), no
/// task error may appear, and the system must drain to zero (no stranded
/// promoted-but-unclaimed permit).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn claim_timeout_abandon_race_storm() {
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(1000).memory_units(1000))
        .class_policy(
            cls("c"),
            ClassPolicy::new()
                .max_inflight(2)
                .max_queue_depth(512)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .unwrap();

    let returned = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for i in 0..200usize {
        let rt = rt.clone();
        let returned = returned.clone();
        let mut lcg = Lcg(0x1357_9BDF_0000_0000 ^ i as u64);
        // Acquire timeout in a tight band around the tiny service time so the
        // timeout and the promotion frequently race.
        let acquire_ms = 1 + lcg.below(6);
        let work_ms = lcg.below(4);
        handles.push(tokio::spawn(async move {
            let spec = TaskSpec::io(cls("c")).operation(format!("r-{i}"));
            let opts =
                SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(acquire_ms));
            let r = rt
                .run_io_with(spec, opts, async move {
                    if work_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(work_ms)).await;
                    }
                    Ok::<_, ()>(i)
                })
                .await;
            // Must be a decision, never a task error.
            assert!(
                !matches!(r, Err(RunError::Task(_))),
                "task error must not appear in the race"
            );
            returned.fetch_add(1, Ordering::SeqCst);
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    assert_eq!(
        returned.load(Ordering::SeqCst),
        200,
        "every submission returns (no hang)"
    );

    // Give any timeout-driven abandons their final promote/release chain a beat,
    // then prove nothing stranded.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_drained(&rt, &["c"]);

    // Still usable.
    let out: i32 = rt
        .run_io(TaskSpec::io(cls("c")).operation("final"), async {
            Ok::<_, ()>(1)
        })
        .await
        .unwrap();
    assert_eq!(out, 1);
}
