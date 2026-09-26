//! Named blocking-facade costs (ADR 9000 / B01). The governed full-facade case
//! constructs TaskSpec inside the timed span; its difference from raw Tokio,
//! Semaphore or Tower includes spec construction, admission and host dispatch.
//! The prebuilt-spec case excludes spec/runtime cloning from the timed span;
//! it remains a full host submission and is not a pure governor cost.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use taskmesh::{
    AdmissionVerdict, Builder, CancellationToken, ClassPolicy, GovernorError, ResourceBudget,
    RunError, Runtime, SubmitOptions, TaskClass, TaskSpec, TokioRuntime, TopologyConfig,
};
use tokio::sync::Semaphore;
use tower::limit::ConcurrencyLimit;
use tower::{service_fn, Service, ServiceExt};

fn build_runtime() -> TokioRuntime {
    Builder::new()
        .resources(
            ResourceBudget::new()
                .cpu_units(1_000_000)
                .memory_units(1_000_000),
        )
        .class_policy(
            TaskClass::new("retrieval"),
            ClassPolicy::new()
                .max_inflight(1_000_000)
                .cpu_units(1)
                .memory_units(1),
        )
        .build()
        .expect("runtime must build")
}

fn build_blocking_gate_runtime() -> TokioRuntime {
    Builder::new()
        .topology(TopologyConfig::new().blocking_threads(1))
        .resources(
            ResourceBudget::new()
                .cpu_units(1_000_000)
                .memory_units(1_000_000),
        )
        .class_policy(
            TaskClass::new("retrieval"),
            ClassPolicy::new()
                .max_inflight(1_000_000)
                .cpu_units(1)
                .memory_units(1),
        )
        .build()
        .expect("runtime must build")
}

fn governance_tax(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("tokio runtime");
    let runtime = build_runtime();
    let prebuilt_spec = TaskSpec::blocking(TaskClass::new("retrieval")).operation("noop");

    let mut group = c.benchmark_group("governance_tax_blocking_noop");

    group.bench_function("governed_full_facade", |b| {
        b.to_async(&rt).iter_batched(
            || runtime.clone(),
            |runtime| async move {
                let spec = TaskSpec::blocking(TaskClass::new("retrieval")).operation("noop");
                runtime
                    .run_blocking(spec, || Ok::<(), std::convert::Infallible>(()))
                    .await
                    .expect("governed noop");
            },
            BatchSize::PerIteration,
        );
    });

    group.bench_function("governed_prebuilt_spec", |b| {
        b.to_async(&rt).iter_batched(
            || (runtime.clone(), prebuilt_spec.clone()),
            |(runtime, spec)| async move {
                runtime
                    .run_blocking(spec, || Ok::<(), Infallible>(()))
                    .await
                    .expect("governed noop with prebuilt spec");
            },
            BatchSize::PerIteration,
        );
    });

    group.bench_function("raw_spawn_blocking", |b| {
        b.to_async(&rt)
            .iter(|| async { tokio::task::spawn_blocking(|| ()).await.expect("join") });
    });

    let sem = Arc::new(Semaphore::new(1_000_000));
    group.bench_function("semaphore_only", |b| {
        b.to_async(&rt).iter(|| {
            let sem = sem.clone();
            async move {
                let permit = sem.acquire_owned().await.expect("permit");
                tokio::task::spawn_blocking(|| ()).await.expect("join");
                drop(permit);
            }
        });
    });

    // Tower ConcurrencyLimit is a narrower mechanism control, not an equivalent
    // governed-execution peer.
    let limited = ConcurrencyLimit::new(
        service_fn(|_: ()| async {
            tokio::task::spawn_blocking(|| ()).await.expect("join");
            Ok::<(), Infallible>(())
        }),
        1_000_000,
    );
    group.bench_function("tower_concurrency_limit", |b| {
        b.to_async(&rt).iter(|| {
            let mut svc = limited.clone();
            async move {
                svc.ready()
                    .await
                    .expect("ready")
                    .call(())
                    .await
                    .expect("call");
            }
        });
    });

    group.bench_function("governed_pre_submit_cancelled", |b| {
        b.to_async(&rt).iter(|| {
            let runtime = runtime.clone();
            async move {
                let token = CancellationToken::new();
                token.cancel();
                let spec = TaskSpec::blocking(TaskClass::new("retrieval")).operation("cancelled");
                let err = runtime
                    .run_blocking_with(spec, SubmitOptions::unbounded().with_cancel(token), || {
                        Ok::<(), Infallible>(())
                    })
                    .await
                    .expect_err("cancelled-before-submit must reject");
                assert!(matches!(
                    err,
                    RunError::Governor(GovernorError::Rejected(
                        AdmissionVerdict::CancelledBeforeSubmit
                    ))
                ));
            }
        });
    });

    let gated_runtime = build_blocking_gate_runtime();
    let (holder_started_tx, holder_started_rx) = std::sync::mpsc::channel::<()>();
    let (release_holder, hold_holder) = std::sync::mpsc::channel::<()>();
    let holder_runtime = gated_runtime.clone();
    let holder = rt.spawn(async move {
        holder_runtime
            .run_blocking(
                TaskSpec::blocking(TaskClass::new("retrieval")).operation("hold-gate"),
                move || {
                    holder_started_tx
                        .send(())
                        .expect("holder start receiver alive");
                    let _ = hold_holder.recv();
                    Ok::<(), Infallible>(())
                },
            )
            .await
    });
    holder_started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("holder must acquire the blocking slot");

    // A saturated capability pool sheds a non-queueing class at admission —
    // there is no gate queue to time out of, so this measures the immediate
    // rejection path, not a zero-length wait.
    group.bench_function("governed_substrate_saturated_shed", |b| {
        b.to_async(&rt).iter(|| {
            let runtime = gated_runtime.clone();
            async move {
                let spec = TaskSpec::blocking(TaskClass::new("retrieval")).operation("blocked");
                let err = runtime
                    .run_blocking_with(
                        spec,
                        SubmitOptions::unbounded().with_acquire_timeout(Duration::ZERO),
                        || Ok::<(), Infallible>(()),
                    )
                    .await
                    .expect_err("a full capability pool must shed immediately");
                assert!(
                    matches!(
                        err,
                        RunError::Governor(GovernorError::Rejected(
                            AdmissionVerdict::SubstrateSaturated { .. }
                        ))
                    ),
                    "expected SubstrateSaturated, got {err:?}"
                );
            }
        });
    });

    group.finish();
    release_holder.send(()).expect("holder receiver alive");
    rt.block_on(async {
        holder
            .await
            .expect("holder join")
            .expect("holder finishes cleanly");
    });
}

criterion_group!(benches, governance_tax);
criterion_main!(benches);
