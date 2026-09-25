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

async fn wait_for_class_state(
    rt: &TokioRuntime,
    name: &str,
    expected_inflight: u32,
    expected_queued: u32,
) {
    bounded_storm("class reaches expected state", async {
        loop {
            let snapshot = rt.snapshot();
            let class = &snapshot.classes[&cls(name)];
            if (class.inflight, class.queued) == (expected_inflight, expected_queued) {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
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
/// inflight system-wide regardless of class. Two held jobs fill the budget; a
/// third class must queue and then start when one holder returns its permit.
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
    assert_eq!(rt.config().resources.max_cpu_units, 4);

    let (alpha_started_tx, alpha_started_rx) = tokio::sync::oneshot::channel();
    let (release_alpha_tx, release_alpha_rx) = tokio::sync::oneshot::channel();
    let alpha_rt = rt.clone();
    let alpha = tokio::spawn(async move {
        alpha_rt
            .run_io(
                TaskSpec::io(cls("alpha")).operation("alpha-holder"),
                async {
                    alpha_started_tx.send(()).expect("alpha start is observed");
                    release_alpha_rx.await.expect("alpha release is sent");
                    Ok::<_, ()>("alpha")
                },
            )
            .await
    });

    let (beta_started_tx, beta_started_rx) = tokio::sync::oneshot::channel();
    let (release_beta_tx, release_beta_rx) = tokio::sync::oneshot::channel();
    let beta_rt = rt.clone();
    let beta = tokio::spawn(async move {
        beta_rt
            .run_io(TaskSpec::io(cls("beta")).operation("beta-holder"), async {
                beta_started_tx.send(()).expect("beta start is observed");
                release_beta_rx.await.expect("beta release is sent");
                Ok::<_, ()>("beta")
            })
            .await
    });
    bounded_storm("both global-budget holders start", async {
        alpha_started_rx.await.expect("alpha starts");
        beta_started_rx.await.expect("beta starts");
    })
    .await;

    let (gamma_started_tx, mut gamma_started_rx) = tokio::sync::oneshot::channel();
    let (release_gamma_tx, release_gamma_rx) = tokio::sync::oneshot::channel();
    let gamma_rt = rt.clone();
    let gamma = tokio::spawn(async move {
        gamma_rt
            .run_io(
                TaskSpec::io(cls("gamma")).operation("gamma-waiter"),
                async {
                    gamma_started_tx.send(()).expect("gamma start is observed");
                    release_gamma_rx.await.expect("gamma release is sent");
                    Ok::<_, ()>("gamma")
                },
            )
            .await
    });
    tokio::select! {
        () = wait_for_class_state(&rt, "gamma", 0, 1) => {}
        result = &mut gamma_started_rx => {
            panic!("gamma started before global CPU capacity was released: {result:?}");
        }
    }
    let saturated = rt.snapshot();
    assert_eq!(
        saturated
            .classes
            .values()
            .map(|class| class.cpu_units_held)
            .sum::<u128>(),
        4,
        "the two holders consume the complete global CPU budget"
    );

    release_alpha_tx.send(()).expect("release alpha holder");
    bounded_storm("gamma is promoted across classes", gamma_started_rx)
        .await
        .expect("gamma starts after alpha releases");
    wait_for_class_state(&rt, "gamma", 1, 0).await;
    let promoted = rt.snapshot();
    assert_eq!(
        promoted
            .classes
            .values()
            .map(|class| class.cpu_units_held)
            .sum::<u128>(),
        4,
        "beta and promoted gamma remain within the global CPU budget"
    );

    release_beta_tx.send(()).expect("release beta holder");
    release_gamma_tx.send(()).expect("release gamma holder");
    assert_eq!(alpha.await.expect("alpha task joins"), Ok("alpha"));
    assert_eq!(beta.await.expect("beta task joins"), Ok("beta"));
    assert_eq!(gamma.await.expect("gamma task joins"), Ok("gamma"));

    let names = ["alpha", "beta", "gamma"];
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
        .class_policy(
            cls("sweep-control"),
            ClassPolicy::new()
                .max_inflight(1)
                .cpu_units(1)
                .memory_units(1)
                .memory_release_policy(MemoryReleasePolicy::LeakDetecting),
        )
        .build()
        .unwrap();

    // Keep one leak-detecting lease live and already running for the entire
    // sweep. This makes the reaper exercise its stale-active retention path;
    // without this control every storm class uses OnTaskCompletion and the
    // reaper skips every permit before inspecting its execution phase.
    let control_spec = TaskSpec::io(cls("sweep-control")).operation("sweep-control-holder");
    let ext::AdmissionDecision::Admitted {
        permit_id: control_permit,
    } = rt.governor().admit(&control_spec)
    else {
        panic!("sweep control must admit");
    };
    let ext::AdvanceOutcome::Leased(control_lease) = rt
        .governor()
        .advance_phase(control_permit, ExecutionPhase::Running)
    else {
        panic!("sweep control must become a running lease");
    };
    tokio::time::sleep(Duration::from_millis(2)).await;

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
            let mut max_storm_live_seen = 0u32;
            while !stop.load(Ordering::Relaxed) {
                let report = rt.governor().reap_leaks_with(0);
                assert_eq!(
                    (
                        report.reclaimed_permits,
                        report.suspected_leaks,
                        report.retained_active,
                    ),
                    (0, 1, 1),
                    "the stale running control lease must be observed and retained"
                );
                let snapshot = rt.snapshot();
                assert_eq!(snapshot.conservation_violation(), None);
                let storm_live: u32 = ["io", "heavy", "batch"]
                    .iter()
                    .map(|name| snapshot.classes[&cls(name)].inflight)
                    .sum();
                max_storm_live_seen = max_storm_live_seen.max(storm_live);
                sweeps += 1;
                tokio::task::yield_now().await;
            }
            (sweeps, max_storm_live_seen)
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
    let (sweeps, max_storm_live_seen) =
        bounded_storm("mixed-substrate soak task group", async move {
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
    assert_eq!(
        rt.governor().release_leased(control_lease),
        ext::ReleaseOutcome::Released,
        "the sweeper must leave the live control lease owned by its holder"
    );
    assert!(sweeps > 0, "sweeper must have run concurrently");
    assert!(
        max_storm_live_seen > 0,
        "the sweeper never overlapped the io/heavy/batch storm"
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
    assert_eq!(admitted, 361, "360 jobs plus the sweep control lease");
    assert_eq!(
        terminated, 361,
        "every job and the sweep control lease terminated"
    );

    assert_drained(&rt, &["io", "heavy", "batch", "sweep-control"]);
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

    // Deterministic controls prove both sides of the race before the soak. Two
    // direct reservations fill the class. The first host waiter must queue and
    // later be promoted; the second must time out and abandon its ticket while
    // the reservations remain held.
    let holder_a = match rt
        .governor()
        .admit(&TaskSpec::io(cls("c")).operation("control-holder-a"))
    {
        ext::AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("first control holder must admit, got {other:?}"),
    };
    let holder_b = match rt
        .governor()
        .admit(&TaskSpec::io(cls("c")).operation("control-holder-b"))
    {
        ext::AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("second control holder must admit, got {other:?}"),
    };

    let promoted_rt = rt.clone();
    let promoted = tokio::spawn(async move {
        promoted_rt
            .run_io(
                TaskSpec::io(cls("c")).operation("control-promoted"),
                async { Ok::<_, ()>("promoted") },
            )
            .await
    });
    wait_for_class_state(&rt, "c", 2, 1).await;

    let timeout_rt = rt.clone();
    let timed_out = tokio::spawn(async move {
        timeout_rt
            .run_io_with(
                TaskSpec::io(cls("c")).operation("control-timeout"),
                SubmitOptions::unbounded().with_acquire_timeout(Duration::from_secs(1)),
                async { Ok::<_, ()>("must-not-run") },
            )
            .await
    });
    wait_for_class_state(&rt, "c", 2, 2).await;
    let timeout_error = bounded_storm("control waiter times out", timed_out)
        .await
        .expect("timeout task joins")
        .expect_err("the second queued control must time out");
    assert_eq!(
        timeout_error,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::PermitAcquireTimedOut {
                retry_after_ms: None,
            }
        ))
    );
    wait_for_class_state(&rt, "c", 2, 1).await;
    assert_eq!(
        rt.governor().release(holder_a),
        ext::ReleaseOutcome::Released
    );
    assert_eq!(
        bounded_storm("control waiter is promoted", promoted)
            .await
            .expect("promoted task joins"),
        Ok("promoted")
    );
    assert_eq!(
        rt.governor().release(holder_b),
        ext::ReleaseOutcome::Released
    );
    assert_drained(&rt, &["c"]);

    let mut handles = Vec::new();
    for i in 0..200usize {
        let rt = rt.clone();
        let mut lcg = Lcg(0x1357_9BDF_0000_0000 ^ i as u64);
        // Acquire timeout in a tight band around the tiny service time so the
        // timeout and the promotion frequently race.
        let acquire_ms = 1 + lcg.below(6);
        let work_ms = lcg.below(4);
        handles.push(tokio::spawn(async move {
            let spec = TaskSpec::io(cls("c")).operation(format!("r-{i}"));
            let opts =
                SubmitOptions::unbounded().with_acquire_timeout(Duration::from_millis(acquire_ms));
            let result = rt
                .run_io_with(spec, opts, async move {
                    if work_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(work_ms)).await;
                    }
                    Ok::<_, ()>(i)
                })
                .await;
            (i, result)
        }));
    }
    let (completed, acquire_timeouts) =
        bounded_storm("claim-timeout-abandon race task group", async move {
            let mut completed = 0usize;
            let mut acquire_timeouts = 0usize;
            for handle in handles {
                let (i, result) = handle.await.expect("storm task joins");
                match result {
                    Ok(value) => {
                        assert_eq!(value, i, "the closure's value must survive the race");
                        completed += 1;
                    }
                    Err(RunError::Governor(GovernorError::Rejected(
                        AdmissionVerdict::PermitAcquireTimedOut {
                            retry_after_ms: None,
                        },
                    ))) => acquire_timeouts += 1,
                    Err(other) => {
                        panic!("storm request {i} returned an unexpected error: {other:?}")
                    }
                }
            }
            (completed, acquire_timeouts)
        })
        .await;
    assert_eq!(
        completed + acquire_timeouts,
        200,
        "every storm submission returns an explicitly classified outcome"
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
