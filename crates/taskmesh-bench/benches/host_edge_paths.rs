//! Host-side complex-path coverage not exercised by the engine-only hot-path
//! benches: async/local deadline fail-closed semantics, per-substrate
//! capability-pool contention beyond the default blocking pool, and control-plane
//! overhead on the non-blocking host entry points.

use std::convert::Infallible;
use std::sync::mpsc;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};
use taskmesh::{
    AdmissionVerdict, Builder, CancellationPolicy, ClassPolicy, GovernorError, ResourceBudget,
    RunError, Runtime, SubmitOptions, SubstrateHint, TaskClass, TaskSpec, TokioRuntime,
    TopologyConfig,
};

fn build_runtime(
    topology: TopologyConfig,
    cancellation_policy: CancellationPolicy,
) -> TokioRuntime {
    Builder::new()
        .topology(topology)
        .resources(
            ResourceBudget::new()
                .cpu_units(1_000_000)
                .memory_units(1_000_000),
        )
        .class_policy(
            TaskClass::new("retrieval"),
            ClassPolicy::new()
                .max_inflight(1_000_000)
                .max_queue_depth(64)
                .cpu_units(1)
                .memory_units(1)
                .cancellation_policy(cancellation_policy),
        )
        .build()
        .expect("runtime must build")
}

fn assert_deadline(err: RunError<Infallible>) {
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::DeadlineExceeded)
    ));
}

fn assert_substrate_timeout(err: RunError<Infallible>) {
    assert!(matches!(
        err,
        RunError::Governor(GovernorError::Rejected(
            AdmissionVerdict::SubstratePoolTimedOut { .. }
        ))
    ));
}

fn host_edge_paths(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("tokio runtime");

    let default_runtime = build_runtime(
        TopologyConfig::new().cpu_fixed(4),
        CancellationPolicy::PreSubmitOnly,
    );
    let deadline_runtime = build_runtime(
        TopologyConfig::new().local_runtime_slots(4),
        CancellationPolicy::CooperativeWithDeadline,
    );

    let mut noop_group = c.benchmark_group("host_noop_paths");

    noop_group.bench_function("run_io_noop", |b| {
        b.to_async(&rt).iter(|| {
            let runtime = default_runtime.clone();
            async move {
                runtime
                    .run_io(
                        TaskSpec::io(TaskClass::new("retrieval")).operation("io-noop"),
                        async { Ok::<(), Infallible>(()) },
                    )
                    .await
                    .expect("io noop");
            }
        });
    });

    noop_group.bench_function("run_cpu_noop", |b| {
        b.to_async(&rt).iter(|| {
            let runtime = default_runtime.clone();
            async move {
                runtime
                    .run_cpu(
                        TaskSpec::cpu(TaskClass::new("retrieval")).operation("cpu-noop"),
                        || Ok::<(), Infallible>(()),
                    )
                    .await
                    .expect("cpu noop");
            }
        });
    });

    noop_group.bench_function("run_local_noop", |b| {
        b.to_async(&rt).iter(|| {
            let runtime = default_runtime.clone();
            async move {
                runtime
                    .run_local(
                        TaskSpec::local(TaskClass::new("retrieval")).operation("local-noop"),
                        async { Ok::<(), Infallible>(()) },
                    )
                    .await
                    .expect("local noop");
            }
        });
    });

    noop_group.finish();

    let mut deadline_group = c.benchmark_group("host_deadline_failclosed");

    deadline_group.bench_function("run_io_deadline_zero_budget", |b| {
        b.to_async(&rt).iter(|| {
            let runtime = deadline_runtime.clone();
            async move {
                let err = runtime
                    .run_io_with(
                        TaskSpec::io(TaskClass::new("retrieval")).operation("io-deadline"),
                        SubmitOptions::unbounded().with_deadline(Duration::ZERO),
                        std::future::pending::<Result<(), Infallible>>(),
                    )
                    .await
                    .expect_err("deadline must fire");
                assert_deadline(err);
            }
        });
    });

    deadline_group.bench_function("run_local_deadline_zero_budget", |b| {
        b.to_async(&rt).iter(|| {
            let runtime = deadline_runtime.clone();
            async move {
                let err = runtime
                    .run_local_with(
                        TaskSpec::local(TaskClass::new("retrieval")).operation("local-deadline"),
                        SubmitOptions::unbounded().with_deadline(Duration::ZERO),
                        std::future::pending::<Result<(), Infallible>>(),
                    )
                    .await
                    .expect_err("deadline must fire");
                assert_deadline(err);
            }
        });
    });

    deadline_group.finish();

    let large_stack_runtime = build_runtime(
        TopologyConfig::new().large_stack_slots(1),
        CancellationPolicy::PreSubmitOnly,
    );
    let (large_stack_started_tx, large_stack_started_rx) = mpsc::channel::<()>();
    let (large_stack_release_tx, large_stack_release_rx) = mpsc::channel::<()>();
    let large_stack_holder_runtime = large_stack_runtime.clone();
    let large_stack_holder = rt.spawn(async move {
        large_stack_holder_runtime
            .run_blocking(
                TaskSpec::base(
                    TaskClass::new("retrieval"),
                    SubstrateHint::LargeStackCapability,
                )
                .operation("hold-large-stack"),
                move || {
                    large_stack_started_tx
                        .send(())
                        .expect("holder start receiver alive");
                    let _ = large_stack_release_rx.recv();
                    Ok::<(), Infallible>(())
                },
            )
            .await
    });
    large_stack_started_rx
        .recv()
        .expect("large-stack holder must acquire its slot");

    let background_runtime = build_runtime(
        TopologyConfig::new().maintenance_workers(1),
        CancellationPolicy::PreSubmitOnly,
    );
    let (background_started_tx, background_started_rx) = mpsc::channel::<()>();
    let (background_release_tx, background_release_rx) = mpsc::channel::<()>();
    let background_holder_runtime = background_runtime.clone();
    let background_holder = rt.spawn(async move {
        background_holder_runtime
            .run_blocking(
                TaskSpec::base(TaskClass::new("retrieval"), SubstrateHint::BackgroundOnly)
                    .operation("hold-background"),
                move || {
                    background_started_tx
                        .send(())
                        .expect("holder start receiver alive");
                    let _ = background_release_rx.recv();
                    Ok::<(), Infallible>(())
                },
            )
            .await
    });
    background_started_rx
        .recv()
        .expect("background holder must acquire its slot");

    let cpu_runtime = build_runtime(
        TopologyConfig::new().cpu_fixed(1),
        CancellationPolicy::PreSubmitOnly,
    );
    let (cpu_started_tx, cpu_started_rx) = mpsc::channel::<()>();
    let (cpu_release_tx, cpu_release_rx) = mpsc::channel::<()>();
    let cpu_holder_runtime = cpu_runtime.clone();
    let cpu_holder = rt.spawn(async move {
        cpu_holder_runtime
            .run_cpu(
                TaskSpec::cpu(TaskClass::new("retrieval")).operation("hold-cpu"),
                move || {
                    cpu_started_tx
                        .send(())
                        .expect("holder start receiver alive");
                    let _ = cpu_release_rx.recv();
                    Ok::<(), Infallible>(())
                },
            )
            .await
    });
    cpu_started_rx
        .recv()
        .expect("cpu holder must acquire its slot");

    let local_runtime = build_runtime(
        TopologyConfig::new().local_runtime_slots(1),
        CancellationPolicy::PreSubmitOnly,
    );
    let (local_started_tx, local_started_rx) = mpsc::channel::<()>();
    let (local_release_tx, local_release_rx) = mpsc::channel::<()>();
    let local_holder_runtime = local_runtime.clone();
    let local_holder = std::thread::spawn(move || {
        let holder_rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("local holder runtime");
        holder_rt.block_on(async move {
            local_holder_runtime
                .run_local(
                    TaskSpec::local(TaskClass::new("retrieval")).operation("hold-local"),
                    async move {
                        local_started_tx
                            .send(())
                            .expect("local holder start receiver alive");
                        let _ = local_release_rx.recv();
                        Ok::<(), Infallible>(())
                    },
                )
                .await
        })
    });
    local_started_rx
        .recv()
        .expect("local holder must acquire its slot");

    let mut gate_group = c.benchmark_group("host_substrate_gate_zero_wait");

    gate_group.bench_function("large_stack_timeout_zero_wait", |b| {
        b.to_async(&rt).iter(|| {
            let runtime = large_stack_runtime.clone();
            async move {
                let err = runtime
                    .run_blocking_with(
                        TaskSpec::base(
                            TaskClass::new("retrieval"),
                            SubstrateHint::LargeStackCapability,
                        )
                        .operation("large-stack-timeout"),
                        SubmitOptions::unbounded().with_acquire_timeout(Duration::ZERO),
                        || Ok::<(), Infallible>(()),
                    )
                    .await
                    .expect_err("full large-stack pool must reject immediately");
                assert_substrate_timeout(err);
            }
        });
    });

    gate_group.bench_function("background_timeout_zero_wait", |b| {
        b.to_async(&rt).iter(|| {
            let runtime = background_runtime.clone();
            async move {
                let err = runtime
                    .run_blocking_with(
                        TaskSpec::base(TaskClass::new("retrieval"), SubstrateHint::BackgroundOnly)
                            .operation("background-timeout"),
                        SubmitOptions::unbounded().with_acquire_timeout(Duration::ZERO),
                        || Ok::<(), Infallible>(()),
                    )
                    .await
                    .expect_err("full maintenance pool must reject immediately");
                assert_substrate_timeout(err);
            }
        });
    });

    gate_group.bench_function("cpu_timeout_zero_wait", |b| {
        b.to_async(&rt).iter(|| {
            let runtime = cpu_runtime.clone();
            async move {
                let err = runtime
                    .run_cpu_with(
                        TaskSpec::cpu(TaskClass::new("retrieval")).operation("cpu-timeout"),
                        SubmitOptions::unbounded().with_acquire_timeout(Duration::ZERO),
                        || Ok::<(), Infallible>(()),
                    )
                    .await
                    .expect_err("full cpu pool must reject immediately");
                assert_substrate_timeout(err);
            }
        });
    });

    gate_group.bench_function("local_runtime_timeout_zero_wait", |b| {
        b.to_async(&rt).iter(|| {
            let runtime = local_runtime.clone();
            async move {
                let err = runtime
                    .run_local_with(
                        TaskSpec::local(TaskClass::new("retrieval")).operation("local-timeout"),
                        SubmitOptions::unbounded().with_acquire_timeout(Duration::ZERO),
                        async { Ok::<(), Infallible>(()) },
                    )
                    .await
                    .expect_err("full local-runtime pool must reject immediately");
                assert_substrate_timeout(err);
            }
        });
    });

    gate_group.finish();

    large_stack_release_tx
        .send(())
        .expect("large-stack holder receiver alive");
    background_release_tx
        .send(())
        .expect("background holder receiver alive");
    cpu_release_tx.send(()).expect("cpu holder receiver alive");
    local_release_tx
        .send(())
        .expect("local holder receiver alive");

    rt.block_on(async {
        large_stack_holder
            .await
            .expect("large-stack holder join")
            .expect("large-stack holder finishes cleanly");
        background_holder
            .await
            .expect("background holder join")
            .expect("background holder finishes cleanly");
        cpu_holder
            .await
            .expect("cpu holder join")
            .expect("cpu holder finishes cleanly");
    });
    local_holder
        .join()
        .expect("local holder join")
        .expect("local holder finishes cleanly");
}

criterion_group!(benches, host_edge_paths);
criterion_main!(benches);
