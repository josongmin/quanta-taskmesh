//! Lock-contention scaling of the governor's single global mutex (ADR 9000 / P3).
//!
//! One bench per concurrency level; the per-level throughput feeds the USL fit in
//! `examples/usl_probe.rs`. Uses a counter clock and allocation-free request keys
//! so we measure lock + grant cost, not `SystemTime` or key allocation.

use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use taskmesh_bench::loadgen::contention_throughput;

fn contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("admit_release_contention");
    for &threads in &[1usize, 2, 4, 8] {
        // No `throughput()`: with `iter_custom` returning a whole-batch duration,
        // a fixed `Elements(n)` would distort the reported rate. Report raw
        // per-op time; the USL fit consumes `contention_throughput` directly.
        group.bench_with_input(BenchmarkId::from_parameter(threads), &threads, |b, &t| {
            b.iter_custom(|iters| {
                let ops_per_thread = (iters as usize / t).max(1);
                let total = (ops_per_thread * t) as f64;
                let tput = contention_throughput(t, ops_per_thread);
                // tput = total / elapsed  ⇒  elapsed = total / tput (the real run time).
                Duration::from_secs_f64(total / tput)
            });
        });
    }
    group.finish();
}

criterion_group!(benches, contention);
criterion_main!(benches);
