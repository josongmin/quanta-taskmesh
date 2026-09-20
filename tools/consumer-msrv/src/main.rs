//! Exercises the public surface a downstream consumer relies on, so that
//! "compiles on the declared MSRV" is a statement about the API people use and
//! not about an empty crate.
//!
//! Since 0.2.0 this is also the **migration fixture**: every API shape that
//! `CHANGELOG.md` tells a 0.1.0 consumer to migrate to is used here the way the
//! changelog shows it, on the declared MSRV, with the default feature set and
//! again with `rayon`. `tools/consumer-msrv/check.py` *runs* this binary, so a
//! wrong assertion is a non-zero exit with a message — never a green check.
//!
//! Each section is labelled with the changelog entry it proves.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use taskmesh::ext::{
    AdmissionDecision, AdvanceOutcome, AdvanceRefusal, ClaimOutcome, CpuExecutor,
    ExecutorCapabilities, Governor, ManualClock, PolicySet, ReconcileOutcome, ReleaseOutcome,
    StageReleaseOutcome, BUILTIN_SUBSTRATES,
};
use taskmesh::{
    AdmissionVerdict, Builder, CancellationPolicy, ClassPolicy, ExecutionPhase, GovernorError,
    HeldCapacity, MemoryReleasePolicy, MemoryUnitScale, OverflowPolicy, ResourceBudget, RunError,
    Runtime, SubmitOptions, TaskClass, TaskScope, TaskSpec, TaskStage, TokioRuntime,
    TopologyConfig, TopologyError, PHYSICAL_CPU, PHYSICAL_SHARED_BLOCKING,
};

/// The wire schema a 0.2.0 consumer must expect. The literal is the consumer's
/// own knowledge (what its deserializer was written against); the facade's
/// `SNAPSHOT_SCHEMA_VERSION` must agree with it, and that agreement is checked
/// below — through `taskmesh`, the only crate a consumer depends on.
const EXPECTED_SNAPSHOT_SCHEMA: u32 = 2;

/// Fail loudly: a wrong assertion is a message on stderr and a non-zero exit.
macro_rules! check {
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            eprintln!("taskmesh-consumer-msrv FAIL: {}", format_args!($($arg)+));
            std::process::exit(1);
        }
    };
}

fn retrieval() -> TaskClass {
    TaskClass::new("retrieval")
}

fn build() -> TokioRuntime {
    Builder::new()
        .topology(
            TopologyConfig::new()
                .blocking_threads(2)
                .large_stack_slots(1),
        )
        .resources(ResourceBudget::new().cpu_units(8).memory_units(8))
        .class_policy(
            retrieval(),
            ClassPolicy::new()
                .max_inflight(4)
                .max_queue_depth(8)
                .cpu_units(1)
                .memory_units(1)
                .memory_release_policy(MemoryReleasePolicy::OnStageBoundary)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )
        .build()
        .expect("consumer configuration builds")
}

// ---- CHANGELOG: `GovernorError` new variants + `non_exhaustive` ------------

/// A consumer's error classifier. Every 0.2.0 variant is named so the arms are
/// proven to exist, and the `_` arm keeps this compiling when a later minor
/// release adds another one. Note what is *not* here any more: a worker
/// failure is never `PolicyViolation` in 0.2.0.
fn classify(error: &GovernorError) -> &'static str {
    match error {
        GovernorError::Rejected(AdmissionVerdict::SubstrateSaturated { .. }) => {
            "substrate-saturated"
        }
        GovernorError::Rejected(AdmissionVerdict::SubstratePoolTimedOut { .. }) => {
            "substrate-pool-timed-out"
        }
        GovernorError::Rejected(_) => "rejected",
        GovernorError::DeadlineUnsupported { .. } => "deadline-unsupported",
        GovernorError::LeaseReclaimed { .. } => "lease-reclaimed",
        GovernorError::WorkerUnavailable { .. } => "worker-unavailable",
        GovernorError::WorkerPanicked { .. } => "worker-panicked",
        GovernorError::JobAbandoned { .. } => "job-abandoned",
        GovernorError::InvalidTopology(_) => "invalid-topology",
        GovernorError::TicketClaimTerminated { .. } => "ticket-terminated",
        GovernorError::InvalidTicketClaim { .. } => "ticket-invalid",
        GovernorError::PolicyViolation(_) => "policy-violation",
        GovernorError::Cancelled => "cancelled",
        GovernorError::DeadlineExceeded => "deadline-exceeded",
        GovernorError::LocalRuntimeUnavailable => "local-runtime-unavailable",
        _ => "other",
    }
}

// ---- CHANGELOG: custom `CpuExecutor` declares `ExecutorCapabilities` -------

/// A consumer-written adapter that says how many workers it has.
struct DeclaringExecutor {
    workers: u32,
}

impl CpuExecutor for DeclaringExecutor {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        std::thread::spawn(work);
    }

    fn capabilities(&self) -> ExecutorCapabilities {
        ExecutorCapabilities::legacy()
            .nonblocking_submit(true)
            .declared_workers(self.workers)
            .exclusive_pool(true)
            .physical_domain(PHYSICAL_CPU)
    }
}

/// A 0.1.0-era adapter that never heard of `capabilities()`. It still compiles,
/// but H03 rejects installation because synchronous/unknown submission cannot
/// be placed in a finite physical domain.
struct LegacyExecutor;

impl CpuExecutor for LegacyExecutor {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        std::thread::spawn(work);
    }
}

/// An adapter that accepts the job and drops it. In 0.1.0 this surfaced as
/// `PolicyViolation("cpu worker dropped result")`; in 0.2.0 it is `JobAbandoned`.
struct DroppingExecutor;

impl CpuExecutor for DroppingExecutor {
    fn spawn(&self, work: Box<dyn FnOnce() + Send + 'static>) {
        drop(work);
    }

    fn capabilities(&self) -> ExecutorCapabilities {
        ExecutorCapabilities::legacy()
            .nonblocking_submit(true)
            .declared_workers(4)
            .physical_domain(PHYSICAL_CPU)
    }
}

fn cpu_runtime(executor: Arc<dyn CpuExecutor>) -> Result<TokioRuntime, GovernorError> {
    Builder::new()
        .topology(TopologyConfig::new().cpu_fixed(4))
        .resources(ResourceBudget::new().cpu_units(64).memory_units(64))
        .class_policy(retrieval(), ClassPolicy::new().max_inflight(8).cpu_units(1))
        .cpu_executor(executor)
        .build()
}

#[tokio::main]
async fn main() {
    everyday_ports().await;
    snapshot_schema_2().await;
    deadline_unsupported().await;
    worker_failures_are_typed().await;
    executor_capabilities_are_load_bearing().await;
    topology_is_validated();
    governor_outcomes();
    memory_outcomes();
    drain_is_the_shutdown_contract().await;
    declared_nested_wait_is_refused();
    println!("taskmesh-consumer-msrv ok");
}

/// The everyday driving port, unchanged in shape since 0.1.0.
async fn everyday_ports() {
    let runtime = build();

    let out: i32 = runtime
        .run_blocking(TaskSpec::blocking(retrieval()).operation("op"), || {
            Ok::<_, ()>(7)
        })
        .await
        .expect("blocking work runs");
    check!(out == 7, "run_blocking returned {out}");

    let out: i32 = runtime
        .run_cpu_with(
            TaskSpec::cpu(retrieval()).operation("cpu"),
            SubmitOptions::unbounded(),
            || Ok::<_, ()>(8),
        )
        .await
        .expect("cpu work runs");
    check!(out == 8, "run_cpu_with returned {out}");

    let out: i32 = runtime
        .run_io(TaskSpec::io(retrieval()).operation("io"), async {
            Ok::<_, ()>(9)
        })
        .await
        .expect("io work runs");
    check!(out == 9, "run_io returned {out}");

    // The typed error split survives; an unknown class is still a verdict.
    let rejected = runtime
        .run_io(
            TaskSpec::io(TaskClass::new("ghost")).operation("x"),
            async { Ok::<i32, ()>(0) },
        )
        .await
        .expect_err("unknown class rejects");
    check!(
        matches!(rejected, RunError::Governor(GovernorError::Rejected(_))),
        "unknown class should be a governor rejection, got {rejected:?}"
    );
    if let RunError::Governor(error) = &rejected {
        check!(classify(error) == "rejected", "classify({error:?})");
    }
}

/// CHANGELOG: `Snapshot` wire schema 2 — `schema_version`, `u128` held units,
/// phase gauges, cumulative totals, `capabilities`, and the conservation check.
async fn snapshot_schema_2() {
    let runtime = build();
    for i in 0..3 {
        let out: i32 = runtime
            .run_blocking(TaskSpec::blocking(retrieval()).operation("op"), move || {
                Ok::<_, ()>(i)
            })
            .await
            .expect("blocking work runs");
        check!(out == i, "run {i} returned {out}");
    }

    let snapshot = runtime.snapshot();
    check!(
        snapshot.schema_version == EXPECTED_SNAPSHOT_SCHEMA,
        "snapshot schema_version {} != {EXPECTED_SNAPSHOT_SCHEMA}",
        snapshot.schema_version
    );
    check!(
        taskmesh::SNAPSHOT_SCHEMA_VERSION == EXPECTED_SNAPSHOT_SCHEMA,
        "the facade exports SNAPSHOT_SCHEMA_VERSION = {} but the consumer expects {EXPECTED_SNAPSHOT_SCHEMA}",
        taskmesh::SNAPSHOT_SCHEMA_VERSION
    );
    check!(
        snapshot.conservation_violation().is_none(),
        "snapshot conservation: {:?}",
        snapshot.conservation_violation()
    );

    let observed = &snapshot.classes[&retrieval()];
    // Exact-width aggregates (`u128`), not `u32`.
    check!(
        observed.cpu_units_held == 0u128,
        "cpu_units_held {}",
        observed.cpu_units_held
    );
    check!(
        observed.memory_units_held == 0u128,
        "memory_units_held {}",
        observed.memory_units_held
    );
    // Phase gauges partition `inflight`.
    let phases = observed.dispatch_reserved
        + observed.accepted
        + observed.running
        + observed.cleanup_pending;
    check!(
        observed.inflight == 0 && phases == 0,
        "inflight {} / phase sum {phases}",
        observed.inflight
    );
    // Cumulative totals: three admitted, three started, three terminated.
    check!(
        observed.admitted_total == 3u128,
        "admitted_total {}",
        observed.admitted_total
    );
    check!(
        observed.started_total == 3u128,
        "started_total {}",
        observed.started_total
    );
    check!(
        observed.terminated_total == 3u128,
        "terminated_total {}",
        observed.terminated_total
    );
    check!(
        observed.admitted_total == u128::from(observed.inflight) + observed.terminated_total,
        "admitted_total identity"
    );

    // Capability-pool occupancy, keyed by pool name, `(in_use, limit)`.
    let blocking = snapshot
        .capabilities
        .get("blocking")
        .copied()
        .expect("blocking pool is reported");
    check!(
        blocking.in_use == 0 && blocking.limit == 2,
        "blocking pool usage {blocking:?}"
    );
    let cpu = snapshot
        .capabilities
        .get("cpu")
        .copied()
        .expect("cpu pool is reported");
    check!(
        cpu.limit >= 1,
        "cpu pool is always gated to the resolved worker count: {cpu:?}"
    );
    let large_stack = snapshot
        .capabilities
        .get("large_stack")
        .copied()
        .expect("large_stack pool is reported");
    check!(
        large_stack.limit == 1,
        "large_stack limit {}",
        large_stack.limit
    );
}

/// CHANGELOG: `SubmitOptions::deadline` on a class that cannot enforce one is
/// refused with `DeadlineUnsupported` (0.1.0 silently dropped the deadline and
/// ran the work to completion).
async fn deadline_unsupported() {
    let runtime = build();
    // `retrieval` keeps the default `CancellationPolicy::PreSubmitOnly`.
    let invoked = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&invoked);
    let error = runtime
        .run_io_with(
            TaskSpec::io(retrieval()).operation("bounded"),
            SubmitOptions::unbounded().with_deadline(Duration::from_secs(1)),
            async move {
                seen.store(true, Ordering::SeqCst);
                Ok::<i32, ()>(1)
            },
        )
        .await
        .expect_err("a deadline the class cannot enforce is refused");
    match &error {
        RunError::Governor(GovernorError::DeadlineUnsupported { class, policy }) => {
            check!(*class == retrieval(), "DeadlineUnsupported.class = {class}");
            check!(
                *policy == CancellationPolicy::PreSubmitOnly,
                "DeadlineUnsupported.policy = {policy:?}"
            );
        }
        other => check!(false, "expected DeadlineUnsupported, got {other:?}"),
    }
    check!(
        !invoked.load(Ordering::SeqCst),
        "the work must not run when its deadline was refused"
    );
    // Refused before admission: nothing was admitted, nothing is held.
    let observed = &runtime.snapshot().classes[&retrieval()];
    check!(
        observed.admitted_total == 0u128,
        "refusal must not admit: {observed:?}"
    );

    // A class that *can* enforce a deadline still gets one.
    let runtime = Builder::new()
        .topology(TopologyConfig::new().blocking_threads(1))
        .resources(ResourceBudget::new().cpu_units(8).memory_units(8))
        .class_policy(
            TaskClass::new("bounded"),
            ClassPolicy::new()
                .max_inflight(1)
                .cancellation_policy(CancellationPolicy::CooperativeWithDeadline),
        )
        .build()
        .expect("deadline-capable class builds");
    let error = runtime
        .run_io_with(
            TaskSpec::io(TaskClass::new("bounded")).operation("slow"),
            SubmitOptions::unbounded().with_deadline(Duration::from_millis(20)),
            async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok::<i32, ()>(1)
            },
        )
        .await
        .expect_err("the run deadline fires");
    check!(
        matches!(error, RunError::Governor(GovernorError::DeadlineExceeded)),
        "expected DeadlineExceeded, got {error:?}"
    );
}

/// CHANGELOG: worker failures are `WorkerPanicked` / `JobAbandoned` (and
/// `WorkerUnavailable`), not `PolicyViolation(String)`.
async fn worker_failures_are_typed() {
    // Keep the panic below out of the check's output; it is the point of the test.
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let runtime = cpu_runtime(Arc::new(DeclaringExecutor { workers: 4 })).expect("builds");
    let error = runtime
        .run_cpu_with(
            TaskSpec::cpu(retrieval()).operation("boom"),
            SubmitOptions::unbounded(),
            || -> Result<i32, ()> { panic!("consumer job panics") },
        )
        .await
        .expect_err("a panicking job is reported");
    std::panic::set_hook(previous_hook);
    match &error {
        RunError::Governor(inner @ GovernorError::WorkerPanicked { context }) => {
            check!(!context.is_empty(), "WorkerPanicked carries its context");
            check!(classify(inner) == "worker-panicked", "classify({inner:?})");
        }
        other => check!(false, "expected WorkerPanicked, got {other:?}"),
    }
    // Custody returned with the failure: nothing is left charged.
    let observed = &runtime.snapshot().classes[&retrieval()];
    check!(
        observed.inflight == 0,
        "panicked job left inflight {}",
        observed.inflight
    );

    let runtime = cpu_runtime(Arc::new(DroppingExecutor)).expect("builds");
    let error = runtime
        .run_cpu_with(
            TaskSpec::cpu(retrieval()).operation("dropped"),
            SubmitOptions::unbounded(),
            || Ok::<i32, ()>(1),
        )
        .await
        .expect_err("a dropped job is reported");
    match &error {
        RunError::Governor(inner @ GovernorError::JobAbandoned { .. }) => {
            check!(classify(inner) == "job-abandoned", "classify({inner:?})");
        }
        other => check!(false, "expected JobAbandoned, got {other:?}"),
    }
    let observed = &runtime.snapshot().classes[&retrieval()];
    check!(
        observed.inflight == 0,
        "abandoned job left inflight {}",
        observed.inflight
    );
}

/// CHANGELOG: installed executors declare nonblocking submission, an exact
/// finite worker count, and a registered physical domain.
async fn executor_capabilities_are_load_bearing() {
    let error = cpu_runtime(Arc::new(DeclaringExecutor { workers: 2 }))
        .err()
        .expect("fewer declared workers than the cpu gate is refused");
    check!(
        error
            == GovernorError::InvalidTopology(TopologyError::ExecutorWorkerCountMismatch {
                declared: 2,
                resolved: 4,
                domain: PHYSICAL_CPU,
            }),
        "expected exact worker/domain mismatch, got {error:?}"
    );
    check!(
        classify(&error) == "invalid-topology",
        "classify({error:?})"
    );

    let runtime = cpu_runtime(Arc::new(DeclaringExecutor { workers: 4 })).expect("equal is enough");
    let declared = runtime.executor_capabilities();
    check!(
        declared.declared_workers == Some(4),
        "declared_workers {declared:?}"
    );
    check!(
        declared.nonblocking_submit,
        "nonblocking_submit {declared:?}"
    );
    check!(declared.exclusive_pool, "exclusive_pool {declared:?}");
    check!(
        declared.physical_domain == Some(PHYSICAL_CPU),
        "physical domain {declared:?}"
    );
    let cpu = runtime.snapshot().capabilities["cpu"];
    check!(
        cpu.limit == 4,
        "cpu gate follows the resolved topology: {cpu:?}"
    );
    let out: i32 = runtime
        .run_cpu_with(
            TaskSpec::cpu(retrieval()).operation("cpu"),
            SubmitOptions::unbounded(),
            || Ok::<_, ()>(42),
        )
        .await
        .expect("cpu work runs on the consumer's executor");
    check!(out == 42, "custom executor returned {out}");

    // A 0.1.0-era adapter compiles unchanged but is rejected at installation.
    let error = cpu_runtime(Arc::new(LegacyExecutor))
        .err()
        .expect("legacy inline/unknown submission is rejected");
    check!(
        error == GovernorError::InvalidTopology(TopologyError::ExecutorSubmissionMayBlock),
        "legacy adapter rejection: {error:?}"
    );

    // Both defaults declare finite physical ownership. Rayon owns the CPU pool;
    // the Tokio fallback shares the aggregate blocking domain.
    let default_runtime = build();
    let declared = default_runtime.executor_capabilities();
    #[cfg(feature = "rayon")]
    check!(
        declared.declared_workers.is_some()
            && declared.nonblocking_submit
            && declared.physical_domain == Some(PHYSICAL_CPU),
        "rayon default executor declares its pool: {declared:?}"
    );
    #[cfg(not(feature = "rayon"))]
    check!(
        declared.declared_workers.is_some()
            && declared.nonblocking_submit
            && !declared.exclusive_pool
            && declared.physical_domain == Some(PHYSICAL_SHARED_BLOCKING),
        "blocking-pool default executor declares its shared finite domain: {declared:?}"
    );
}

/// CHANGELOG: `TopologyConfig::validate` / `TopologyError` — an impossible
/// topology is a typed `Err` from `validate()` and from `Builder::build()`, not
/// a panic.
fn topology_is_validated() {
    let inverted = TopologyConfig::new().min_workers(8).max_workers(2);
    match inverted.validate() {
        Err(TopologyError::InvertedCpuWorkerBounds {
            min_workers,
            max_workers,
        }) => check!(
            min_workers == 8 && max_workers == 2,
            "InvertedCpuWorkerBounds carries the window: {min_workers}/{max_workers}"
        ),
        other => check!(false, "expected InvertedCpuWorkerBounds, got {other:?}"),
    }
    let error = Builder::new()
        .topology(inverted)
        .resources(ResourceBudget::new().cpu_units(8).memory_units(8))
        .class_policy(retrieval(), ClassPolicy::new().max_inflight(1))
        .build()
        .err()
        .expect("an inverted window is refused by build()");
    check!(
        matches!(
            error,
            GovernorError::InvalidTopology(TopologyError::InvertedCpuWorkerBounds { .. })
        ),
        "build() reports the topology error: {error:?}"
    );
    check!(
        matches!(
            TopologyConfig::new().cpu_fixed(0).validate(),
            Err(TopologyError::ZeroFixedCpuWorkers)
        ),
        "Fixed(0) is refused"
    );
    check!(
        TopologyConfig::new().cpu_fixed(2).validate().is_ok(),
        "a valid topology validates"
    );
}

/// CHANGELOG: direct `Governor` embedding through `ext` — `claim` returns
/// `ClaimOutcome`, `release` returns `ReleaseOutcome` (`#[must_use]`),
/// `advance_phase` returns `AdvanceOutcome` and a dispatched permit is released
/// only by its `LeaseToken`, `PolicySet::default()` seeds the built-in inventory
/// and its registry is reached through `substrates()`.
fn governor_outcomes() {
    let policy = PolicySet::default();
    // The registry is an accessor now (0.1.0: a public field), and a defaulted
    // set is not empty: every built-in substrate is present.
    for name in BUILTIN_SUBSTRATES {
        check!(
            policy.substrates().contains_key(*name),
            "PolicySet::default() lacks built-in substrate {name}"
        );
    }
    let governor = Governor::new(policy, Arc::new(ManualClock::new(0)))
        .expect("default policy set is constructible");
    check!(
        matches!(
            governor.admit(&TaskSpec::io(TaskClass::new("unknown")).operation("x")),
            AdmissionDecision::Rejected(_)
        ),
        "unknown class rejects at the engine boundary"
    );
    check!(
        governor.claim(u64::MAX) == ClaimOutcome::Invalid,
        "claim of a never-issued ticket"
    );
    // `ReleaseOutcome` is `#[must_use]`: a consumer inspects it.
    check!(
        governor.release(u64::MAX) == ReleaseOutcome::UnknownPermit,
        "release of a never-granted permit"
    );
    check!(
        governor.advance_phase(u64::MAX, ExecutionPhase::Running)
            == AdvanceOutcome::Refused(AdvanceRefusal::UnknownPermit),
        "phase of no permit is refused as unknown"
    );
    check!(
        governor.reap_leaks().retained_active == 0,
        "leak sweep reports retained_active"
    );

    // A real queue → promote → claim → release cycle.
    let mut classes = std::collections::BTreeMap::new();
    classes.insert(
        retrieval(),
        ClassPolicy::new()
            .max_inflight(1)
            .max_queue_depth(4)
            .cpu_units(1)
            .memory_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    );
    let policy = PolicySet::new(ResourceBudget::new().cpu_units(8).memory_units(8), classes);
    let governor =
        Governor::new(policy, Arc::new(ManualClock::new(0))).expect("queueing policy builds");

    let first = match governor.admit(&TaskSpec::io(retrieval()).operation("first")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => {
            check!(false, "first admit should be admitted, got {other:?}");
            unreachable!()
        }
    };
    let ticket = match governor.admit(&TaskSpec::io(retrieval()).operation("second")) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => {
            check!(
                false,
                "second admit should queue behind max_inflight 1, got {other:?}"
            );
            unreachable!()
        }
    };
    // Match every `ClaimOutcome` arm the way a waiter loop would.
    let describe = |outcome: ClaimOutcome| match outcome {
        ClaimOutcome::Ready(permit) => format!("ready:{permit}"),
        ClaimOutcome::Pending => "pending".to_string(),
        ClaimOutcome::Terminal(reason) => format!("terminal:{reason:?}"),
        ClaimOutcome::Invalid => "invalid".to_string(),
    };
    check!(
        describe(governor.claim(ticket)) == "pending",
        "queued ticket is Pending before promotion"
    );
    check!(
        governor.release(first) == ReleaseOutcome::Released,
        "first release"
    );
    let second = match governor.claim(ticket) {
        ClaimOutcome::Ready(permit) => permit,
        other => {
            check!(
                false,
                "promoted ticket should be Ready, got {}",
                describe(other)
            );
            unreachable!()
        }
    };
    // The first advance out of DispatchReserved hands back the permit's one
    // lease; from here only the token ends the permit.
    let token = match governor.advance_phase(second, ExecutionPhase::Running) {
        AdvanceOutcome::Leased(token) => token,
        other => {
            check!(false, "the first advance leases the permit, got {other:?}");
            unreachable!()
        }
    };
    check!(token.permit_id() == second, "the lease names its permit");
    check!(
        governor.phase(second) == Some(ExecutionPhase::Running),
        "phase reads back Running"
    );
    check!(
        governor.advance_phase(second, ExecutionPhase::Accepted)
            == AdvanceOutcome::Refused(AdvanceRefusal::NotLater {
                current: ExecutionPhase::Running
            }),
        "phases are monotonic: a backwards declaration is refused"
    );
    let observed = governor.snapshot();
    let class = &observed.classes[&retrieval()];
    check!(
        class.running == 1 && class.inflight == 1,
        "running gauge follows the phase: {class:?}"
    );
    check!(
        class.admitted_total == 2u128,
        "admitted_total {}",
        class.admitted_total
    );
    check!(
        observed.conservation_violation().is_none(),
        "engine snapshot conserves"
    );
    // Naming a dispatched permit is not owning it.
    check!(
        governor.release(second)
            == ReleaseOutcome::HeldByLease {
                phase: ExecutionPhase::Running
            },
        "a plain release of a leased permit is refused"
    );
    check!(
        governor.snapshot().classes[&retrieval()].inflight == 1,
        "the refused release changed nothing"
    );
    check!(
        governor.release_leased(token) == ReleaseOutcome::Released,
        "the lease releases its permit"
    );
    check!(
        governor.release(second) == ReleaseOutcome::UnknownPermit,
        "double release is reported, not ignored"
    );
    check!(
        governor.claim(ticket) == ClaimOutcome::Invalid,
        "a claimed ticket is Invalid afterwards"
    );
}

/// CHANGELOG: `TokioRuntime::drain` (D17) — admission closes in the engine, the
/// wait is on custody, a timeout is `NotDrained` per class, and the state is
/// one-way.
async fn drain_is_the_shutdown_contract() {
    let runtime = build();
    check!(!runtime.is_draining(), "a fresh runtime is not draining");
    let (release, hold) = std::sync::mpsc::channel::<()>();
    let holder = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .run_blocking(
                    TaskSpec::blocking(retrieval()).operation("held"),
                    move || {
                        let _released = hold.recv();
                        Ok::<i32, ()>(1)
                    },
                )
                .await
        })
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while runtime.snapshot().classes[&retrieval()].inflight == 0 {
        check!(
            std::time::Instant::now() < deadline,
            "the held job never started"
        );
        tokio::task::yield_now().await;
    }
    let not_drained = match runtime.drain(Duration::from_millis(20)).await {
        Err(not_drained) => not_drained,
        Ok(report) => {
            check!(
                false,
                "a held worker must keep the drain waiting: {report:?}"
            );
            unreachable!()
        }
    };
    check!(
        not_drained
            .classes
            .get(&retrieval())
            .map(|left| left.inflight)
            == Some(1),
        "NotDrained names the class holding the worker: {not_drained}"
    );
    check!(runtime.is_draining(), "a timed-out drain stays draining");
    check!(
        matches!(
            runtime
                .run_io(TaskSpec::io(retrieval()).operation("late"), async {
                    Ok::<i32, ()>(2)
                })
                .await,
            Err(RunError::Governor(GovernorError::Rejected(
                AdmissionVerdict::RuntimeUnavailable
            )))
        ),
        "a submission while draining is refused before admission"
    );
    release.send(()).expect("the held job is waiting");
    check!(
        matches!(holder.await, Ok(Ok(1))),
        "the held job was not cancelled by the drain"
    );
    let report = match runtime.drain(Duration::from_secs(5)).await {
        Ok(report) => report,
        Err(not_drained) => {
            check!(
                false,
                "with the worker gone the drain completes: {not_drained}"
            );
            unreachable!()
        }
    };
    check!(report.classes_drained >= 1, "the report counts classes");
    check!(
        runtime.governor().admission_closed(),
        "the engine's own flag is what refuses"
    );
}

/// CHANGELOG: `TaskSpec::awaited_child_of` / `NestedWaitCycle` (D12) and the
/// `TaskScope::Child { parent_stage, .. }` pattern.
fn declared_nested_wait_is_refused() {
    let mut classes = std::collections::BTreeMap::new();
    classes.insert(
        retrieval(),
        ClassPolicy::new()
            .max_inflight(1)
            .max_queue_depth(4)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    );
    let policy = PolicySet::new(ResourceBudget::new().cpu_units(8).memory_units(8), classes);
    let governor =
        Governor::new(policy, Arc::new(ManualClock::new(0))).expect("queueing policy builds");
    let parent = match governor.admit(&TaskSpec::io(retrieval()).operation("parent")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => {
            check!(false, "the parent takes the slot, got {other:?}");
            unreachable!()
        }
    };
    let child = TaskSpec::io(retrieval())
        .awaited_child_of("parent", "parent", TaskStage::new("child"))
        .operation("declared-child");
    check!(
        matches!(
            &child.scope,
            TaskScope::Child {
                parent_awaits: true,
                ..
            }
        ),
        "awaited_child_of declares the wait"
    );
    match governor.admit(&child) {
        AdmissionDecision::Rejected(AdmissionVerdict::NestedWaitCycle { held_by_root }) => check!(
            held_by_root == HeldCapacity::ClassInflight { class: retrieval() },
            "the verdict names the capacity: {held_by_root}"
        ),
        other => check!(false, "a declared cycle is refused, got {other:?}"),
    }
    let lineage_only = TaskSpec::io(retrieval())
        .child_of("parent", "parent", TaskStage::new("child"))
        .operation("lineage-child");
    check!(
        matches!(
            governor.admit(&lineage_only),
            AdmissionDecision::Queued { .. }
        ),
        "an undeclared wait is never inferred: the child queues"
    );
    check!(
        governor.release(parent) == ReleaseOutcome::Released,
        "parent release"
    );
}

/// CHANGELOG: `release_stage_memory` returns `StageReleaseOutcome` (policy is
/// enforced), `reconcile_memory_at` returns `ReconcileOutcome` (epoch-ordered),
/// `MemoryUnitScale::units_for` is checked.
fn memory_outcomes() {
    let mut classes = std::collections::BTreeMap::new();
    classes.insert(
        TaskClass::new("staged"),
        ClassPolicy::new()
            .max_inflight(2)
            .memory_units(4)
            .memory_release_policy(MemoryReleasePolicy::OnStageBoundary),
    );
    classes.insert(
        TaskClass::new("whole"),
        ClassPolicy::new()
            .max_inflight(2)
            .memory_units(4)
            .memory_release_policy(MemoryReleasePolicy::OnTaskCompletion),
    );
    let policy = PolicySet::new(ResourceBudget::new().cpu_units(8).memory_units(64), classes);
    let governor = Governor::new(policy, Arc::new(ManualClock::new(0))).expect("builds");

    let staged = match governor.admit(&TaskSpec::io(TaskClass::new("staged")).operation("s")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => {
            check!(false, "staged admit, got {other:?}");
            unreachable!()
        }
    };
    let whole = match governor.admit(&TaskSpec::io(TaskClass::new("whole")).operation("w")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => {
            check!(false, "whole admit, got {other:?}");
            unreachable!()
        }
    };

    // 0.1.0 returned `u32` ("units freed") and `0` for a refusal; 0.2.0 names it.
    let outcome = governor.release_stage_memory(staged, 1);
    check!(
        outcome == StageReleaseOutcome::Released { freed_units: 1 },
        "stage release on OnStageBoundary: {outcome:?}"
    );
    check!(
        outcome.freed_units() == 1 && outcome.is_released(),
        "outcome accessors"
    );
    let outcome = governor.release_stage_memory(whole, 1);
    check!(
        outcome
            == StageReleaseOutcome::PolicyForbids {
                policy: MemoryReleasePolicy::OnTaskCompletion
            },
        "stage release on OnTaskCompletion is refused: {outcome:?}"
    );
    check!(
        outcome.freed_units() == 0 && !outcome.is_released(),
        "refusal frees nothing"
    );
    check!(
        governor.release_stage_memory(u64::MAX, 1) == StageReleaseOutcome::UnknownPermit,
        "stage release of no permit"
    );
    // Explicit sequence numbers reject replayed stage events.
    check!(
        governor.release_stage_memory_seq(staged, 1, 5)
            == StageReleaseOutcome::Released { freed_units: 1 },
        "seq 5 applies"
    );
    check!(
        matches!(
            governor.release_stage_memory_seq(staged, 1, 5),
            StageReleaseOutcome::StaleSequence {
                current_sequence: 5
            }
        ),
        "seq 5 replayed is stale"
    );

    // Reconcile with an explicit epoch. (`Estimated` mode charges the remaining
    // reservation: 4 reserved - 2 released = 2.)
    match governor.reconcile_memory_at(staged, 0, 1) {
        ReconcileOutcome::Applied { held_units } => {
            check!(
                held_units == 2,
                "held_units after two stage releases: {held_units}"
            );
        }
        ReconcileOutcome::UnknownPermit => check!(false, "permit is live"),
        ReconcileOutcome::StaleEpoch { current_epoch } => {
            check!(false, "epoch 1 is fresh, current {current_epoch}");
        }
        ReconcileOutcome::ConversionFailed(error) => check!(false, "conversion: {error}"),
        _ => check!(false, "unexpected reconcile outcome"),
    }
    check!(
        matches!(
            governor.reconcile_memory_at(staged, 0, 1),
            ReconcileOutcome::StaleEpoch { current_epoch: 1 }
        ),
        "epoch 1 replayed is stale"
    );
    check!(
        governor.reconcile_memory_at(u64::MAX, 0, 1) == ReconcileOutcome::UnknownPermit,
        "reconcile of no permit"
    );
    // The `bool` form is still there for callers that do not order reporters.
    check!(
        governor.reconcile_memory(staged, 0).is_applied(),
        "bool reconcile applies"
    );

    check!(
        governor.release(staged) == ReleaseOutcome::Released,
        "release staged"
    );
    check!(
        governor.release(whole) == ReleaseOutcome::Released,
        "release whole"
    );

    // `units_for` is checked on both edges instead of saturating.
    let scale = MemoryUnitScale {
        bytes_per_unit: 1024,
    };
    check!(scale.units_for(2048) == Ok(2), "2048 bytes at 1024/unit");
    check!(scale.units_for(2049) == Ok(3), "rounds up");
    // The error type is reachable through the facade, and each edge names its
    // own cause — a consumer can tell "your scale is 0" from "your reading is
    // too large" without parsing a string.
    check!(
        MemoryUnitScale { bytes_per_unit: 0 }.units_for(1)
            == Err(taskmesh::ResourceConversionError::UnscaledMemoryUnits),
        "an unscaled conversion is UnscaledMemoryUnits, not 0"
    );
    check!(
        matches!(
            MemoryUnitScale { bytes_per_unit: 1 }.units_for(u64::MAX),
            Err(taskmesh::ResourceConversionError::MemoryUnitsOverflow {
                bytes: u64::MAX,
                ..
            })
        ),
        "a reading past the u32 unit domain is MemoryUnitsOverflow, not u32::MAX"
    );
}
