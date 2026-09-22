//! Maximum-difficulty operational chaos e2e. These go past `e2e_scenarios.rs`:
//! cross-class contention on a *shared global budget* (not just per-class caps),
//! a mixed-substrate soak with a concurrent leak-sweeper hammering the same
//! state, and an adversarial claim/timeout/abandon race storm that pounds the
//! exact promotion window the `TicketGuard` protects.
//!
//! Invariants asserted are scheduling-independent: forward progress (no
//! deadlock), fail-closed bounds, and drain-to-exactly-zero. No cross-thread
//! ordering is asserted; randomized timing is LCG-seeded for reproducibility.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use taskmesh::*;

const STORM_TIMEOUT: Duration = Duration::from_secs(30);

async fn bounded_storm<F: std::future::Future>(what: &str, future: F) -> F::Output {
    let Ok(output) = tokio::time::timeout(STORM_TIMEOUT, future).await else {
        panic!("{what}: timed out after {STORM_TIMEOUT:?}");
    };
    output
}

fn cls(name: &str) -> TaskClass {
    TaskClass::new(name.to_string())
}

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
    bounded_storm("global-budget contention task group", async move {
        for handle in handles {
            handle.await.unwrap();
        }
    })
    .await;
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
    //
    // Overlap is *observed*, not assumed: the sweeper records the largest
    // `inflight` it saw during a sweep, and the soak requires that number to be
    // positive. A sweeper that only ever ran against an empty ledger proved
    // nothing about concurrent reaping.
    let stop = Arc::new(AtomicBool::new(false));
    let sweeper = {
        let rt = rt.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            let mut sweeps = 0u64;
            let mut max_live_seen = 0u32;
            while !stop.load(Ordering::Relaxed) {
                let report = rt.governor().reap_leaks();
                assert_eq!(
                    report.reclaimed_permits, 0,
                    "nothing in this soak is stale under the default window"
                );
                let snapshot = rt.snapshot();
                assert_eq!(snapshot.conservation_violation(), None);
                let live: u32 = snapshot.classes.values().map(|c| c.inflight).sum();
                max_live_seen = max_live_seen.max(live);
                sweeps += 1;
                tokio::task::yield_now().await;
            }
            (sweeps, max_live_seen)
        })
    };

    // Every submission's verdict is recorded and classified. A `JoinHandle`
    // that resolves is not a job that ran: a soak in which all 360 submissions
    // were rejected before execution used to pass the assertions below.
    #[derive(Default, Debug)]
    struct SubstrateTally {
        attempted: usize,
        completed: usize,
        started: Arc<AtomicUsize>,
    }
    let tallies: Arc<std::sync::Mutex<BTreeMap<&'static str, SubstrateTally>>> =
        Arc::new(std::sync::Mutex::new(BTreeMap::new()));
    let started_io = Arc::new(AtomicUsize::new(0));
    let started_cpu = Arc::new(AtomicUsize::new(0));
    let started_blocking = Arc::new(AtomicUsize::new(0));
    for (name, started) in [
        ("io", &started_io),
        ("heavy", &started_cpu),
        ("batch", &started_blocking),
    ] {
        tallies.lock().expect("tally lock").insert(
            name,
            SubstrateTally {
                started: Arc::clone(started),
                ..SubstrateTally::default()
            },
        );
    }

    let names = ["io", "heavy", "batch"];
    let mut handles = Vec::new();
    for i in 0..360usize {
        let rt = rt.clone();
        let mut lcg = Lcg(0xA5A5_0000_0000_0000 ^ i as u64);
        let started_io = Arc::clone(&started_io);
        let started_cpu = Arc::clone(&started_cpu);
        let started_blocking = Arc::clone(&started_blocking);
        handles.push(tokio::spawn(async move {
            let name = names[(lcg.below(3)) as usize];
            let work = lcg.below(8);
            // Vary substrate by class for realism. Each closure marks that it
            // actually began executing, so "completed" below is backed by a
            // side effect and not just by an `Ok` value.
            let outcome: Result<usize, RunError<()>> = match name {
                "io" => {
                    let spec = TaskSpec::io(cls(name)).operation(format!("{name}-{i}"));
                    rt.run_io(spec, async move {
                        started_io.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(work)).await;
                        Ok::<_, ()>(i)
                    })
                    .await
                }
                "heavy" => {
                    let spec = TaskSpec::cpu(cls(name)).operation(format!("{name}-{i}"));
                    rt.run_cpu(spec, move || {
                        started_cpu.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, ()>(i)
                    })
                    .await
                }
                _ => {
                    let spec = TaskSpec::blocking(cls(name)).operation(format!("{name}-{i}"));
                    rt.run_blocking(spec, move || {
                        started_blocking.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, ()>(i)
                    })
                    .await
                }
            };
            (name, i, outcome)
        }));
    }
    let joined_tallies = Arc::clone(&tallies);
    let joined_stop = Arc::clone(&stop);
    let (sweeps, max_live_seen) = bounded_storm("mixed-substrate soak task group", async move {
        for handle in handles {
            let (name, i, outcome) = handle.await.unwrap();
            let mut tallies = joined_tallies.lock().expect("tally lock");
            let tally = tallies.get_mut(name).expect("known substrate");
            tally.attempted += 1;
            match outcome {
                Ok(value) => {
                    assert_eq!(value, i, "{name}-{i}: the closure's own value comes back");
                    tally.completed += 1;
                }
                // This soak is sized so nothing is shed: every class queues within
                // a depth larger than the whole storm. Any other verdict is a
                // regression, and it is named, not discarded.
                Err(error) => panic!("{name}-{i} did not complete: {error:?}"),
            }
        }
        joined_stop.store(true, Ordering::Relaxed);
        sweeper.await.unwrap()
    })
    .await;
    assert!(sweeps > 0, "sweeper must have run concurrently");
    assert!(
        max_live_seen > 0,
        "the sweeper never observed live work: no concurrent overlap was exercised"
    );

    // Conservation per substrate: attempted == completed == started.
    let tallies = tallies.lock().expect("tally lock");
    let mut total_attempted = 0;
    for (name, tally) in tallies.iter() {
        let started = tally.started.load(Ordering::SeqCst);
        assert!(
            tally.attempted > 0,
            "{name}: the storm must exercise every substrate"
        );
        assert_eq!(
            tally.completed, tally.attempted,
            "{name}: every attempted submission completed"
        );
        assert_eq!(
            started, tally.completed,
            "{name}: every completed submission actually executed its closure"
        );
        total_attempted += tally.attempted;
    }
    assert_eq!(total_attempted, 360, "every submission was accounted for");

    // And the cumulative counters agree with the tallies.
    let snapshot = rt.snapshot();
    let admitted: u128 = snapshot.classes.values().map(|c| c.admitted_total).sum();
    let terminated: u128 = snapshot.classes.values().map(|c| c.terminated_total).sum();
    assert_eq!(admitted, 360, "admitted_total across classes");
    assert_eq!(terminated, 360, "terminated_total across classes");

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
                !matches!(r, Err(RunError::Task(()))),
                "task error must not appear in the race"
            );
            returned.fetch_add(1, Ordering::SeqCst);
        }));
    }
    bounded_storm("claim-timeout-abandon race task group", async move {
        for handle in handles {
            handle.await.unwrap();
        }
    })
    .await;
    assert_eq!(
        returned.load(Ordering::SeqCst),
        200,
        "every submission returns (no hang)"
    );

    // Every submission future has returned; a permit must be released before
    // that completion becomes observable.
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
