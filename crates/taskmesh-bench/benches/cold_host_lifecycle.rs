//! Named public-host lifecycle and fixed-batch costs (B01). These cases are
//! separate populations: construction, first call, steady call, quiescent drain,
//! active IO release-to-settled drain,
//! and one-versus-ten completed IO calls on the same runtime/body. They do not
//! establish sustained capacity or an open-loop tail latency.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use taskmesh::{Builder, ClassPolicy, ResourceBudget, Runtime, TaskClass, TaskSpec, TokioRuntime};

fn build_runtime() -> TokioRuntime {
    Builder::new()
        .resources(
            ResourceBudget::new()
                .cpu_units(1_000_000)
                .memory_units(1_000_000),
        )
        .class_policy(
            TaskClass::new("bench"),
            ClassPolicy::new()
                .max_inflight(1_000_000)
                .cpu_units(1)
                .memory_units(1),
        )
        .build()
        .expect("lifecycle runtime must build")
}

fn spec() -> TaskSpec {
    TaskSpec::io(TaskClass::new("bench")).operation("io-noop")
}

fn blocking_spec() -> TaskSpec {
    TaskSpec::blocking(TaskClass::new("bench")).operation("blocking-noop")
}

async fn one_io(runtime: &TokioRuntime) {
    runtime
        .run_io(spec(), async { Ok::<(), Infallible>(()) })
        .await
        .expect("one IO call must complete");
}

async fn one_blocking(runtime: &TokioRuntime) {
    runtime
        .run_blocking(blocking_spec(), || Ok::<(), Infallible>(()))
        .await
        .expect("one blocking call must complete");
}

fn cold_host_lifecycle(c: &mut Criterion) {
    let tokio = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("Tokio benchmark runtime");

    c.bench_function("host_construction_only", |b| {
        b.iter_batched(|| (), |_| build_runtime(), BatchSize::PerIteration);
    });

    c.bench_function("fresh_host_first_io_call", |b| {
        b.to_async(&tokio).iter_batched(
            build_runtime,
            |runtime| async move {
                one_io(&runtime).await;
                runtime
            },
            BatchSize::PerIteration,
        );
    });

    let steady_runtime = build_runtime();
    tokio.block_on(one_io(&steady_runtime));
    c.bench_function("warmed_host_steady_io_call", |b| {
        b.to_async(&tokio).iter_batched(
            || steady_runtime.clone(),
            |runtime| async move { one_io(&runtime).await },
            BatchSize::PerIteration,
        );
    });

    c.bench_function("fresh_host_quiescent_drain", |b| {
        b.to_async(&tokio).iter_batched(
            build_runtime,
            |runtime| async move {
                runtime
                    .drain(Duration::from_secs(2))
                    .await
                    .expect("quiescent host must drain");
                runtime
            },
            BatchSize::PerIteration,
        );
    });

    c.bench_function("active_io_release_to_settled_drain", |b| {
        b.iter_custom(|iterations| {
            tokio.block_on(async {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iterations {
                    let runtime = build_runtime();
                    let held_runtime = runtime.clone();
                    let release = Arc::new(tokio::sync::Notify::new());
                    let held_release = Arc::clone(&release);
                    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
                    let caller = tokio::spawn(async move {
                        held_runtime
                            .run_io(spec(), async move {
                                started_tx
                                    .send(())
                                    .expect("started observer must remain live");
                                held_release.notified().await;
                                Ok::<(), Infallible>(())
                            })
                            .await
                            .expect("active IO root must complete");
                    });
                    started_rx
                        .await
                        .expect("active root must start before timing");
                    let before = runtime.snapshot();
                    assert_eq!(before.classes[&TaskClass::new("bench")].running, 1);
                    let origin = Instant::now();
                    release.notify_one();
                    runtime
                        .drain(Duration::from_secs(2))
                        .await
                        .expect("active drain must settle");
                    elapsed += origin.elapsed();
                    caller.await.expect("active caller task must join");
                    let after = runtime.snapshot();
                    assert_eq!(after.classes[&TaskClass::new("bench")].inflight, 0);
                    assert!(after.conservation_violation().is_none());
                }
                elapsed
            })
        });
    });

    let mut batch_group = c.benchmark_group("warmed_host_io_noop_completed_batch");
    for batch in [1_u64, 10] {
        batch_group.throughput(Throughput::Elements(batch));
        batch_group.bench_with_input(BenchmarkId::from_parameter(batch), &batch, |b, &count| {
            b.to_async(&tokio).iter_batched(
                || steady_runtime.clone(),
                |runtime| async move {
                    for _ in 0..count {
                        one_io(&runtime).await;
                    }
                },
                BatchSize::PerIteration,
            );
        });
    }
    batch_group.finish();

    tokio.block_on(one_blocking(&steady_runtime));
    let mut blocking_batch_group = c.benchmark_group("warmed_host_blocking_noop_completed_batch");
    for batch in [1_u64, 10] {
        blocking_batch_group.throughput(Throughput::Elements(batch));
        blocking_batch_group.bench_with_input(
            BenchmarkId::from_parameter(batch),
            &batch,
            |b, &count| {
                b.to_async(&tokio).iter_batched(
                    || steady_runtime.clone(),
                    |runtime| async move {
                        for _ in 0..count {
                            one_blocking(&runtime).await;
                        }
                    },
                    BatchSize::PerIteration,
                );
            },
        );
    }
    blocking_batch_group.finish();
}

criterion_group!(benches, cold_host_lifecycle);
criterion_main!(benches);
