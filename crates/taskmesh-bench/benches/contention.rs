//! Lock-contention scaling of the governor's single global mutex (ADR 9000 / P3).
//!
//! One bench per concurrency level; the per-level throughput feeds the USL fit in
//! `examples/usl_probe.rs`. Uses a counter clock and allocation-free request keys
//! so we measure lock + grant cost, not `SystemTime` or key allocation.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use taskmesh_bench::loadgen::contention_run;

fn contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("admit_release_contention");
    for &threads in &[1usize, 2, 4, 8] {
        // No `throughput()`: with `iter_custom` returning a whole-batch duration,
        // a fixed `Elements(n)` would distort the reported rate. Report raw
        // per-op time; the USL fit consumes `contention_run` directly.
        //
        // Criterion divides the returned duration by `iters`, so the run must
        // perform *exactly* `iters` ops. Rounding to a multiple of the thread
        // count skewed the per-op figure by up to `t×` at small batch sizes.
        group.bench_with_input(BenchmarkId::from_parameter(threads), &threads, |b, &t| {
            b.iter_custom(|iters| {
                let requested = usize::try_from(iters).expect("iteration count fits usize");
                let run = contention_run(t, requested);
                assert_eq!(
                    run.completed_ops, requested,
                    "denominator must equal the work timed"
                );
                run.elapsed
            });
        });
    }
    group.finish();
}

criterion_group!(benches, contention);
criterion_main!(benches);
