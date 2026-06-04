//! Fail-closed stability under overload (ADR 9000 / P5). Offered load far exceeds
//! capacity; the queue must stay bounded and rejection must be the pressure valve.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use taskmesh_bench::loadgen::simulate;
use taskmesh_bench::metrics::reject_ratio;
use taskmesh_bench::workload::{
    fixture, generate, mean_interval_ns, retrieval_policy, WorkloadConfig,
};

fn bench(c: &mut Criterion) {
    let cfg = WorkloadConfig {
        classes: vec!["retrieval".into()],
        lambda: 200_000.0, // heavy offered load
        count: 5_000,
        ..Default::default()
    };
    let arrivals = generate(&cfg);
    let interval = mean_interval_ns(cfg.lambda);
    // Service much slower than arrivals -> sustained overload.
    let service_ns = interval * 20;
    let max_queue_depth = 64;

    c.bench_function("overload_stability_sim", |b| {
        b.iter(|| {
            let fx = fixture(
                vec![("retrieval", retrieval_policy(4, max_queue_depth))],
                0,
                0,
            );
            let (_lat, res) = simulate(&fx, &arrivals, service_ns, interval);
            // Invariant the bench documents: the queue never exceeds its bound,
            // and overload is shed via rejection (fail-closed), not unbounded growth.
            assert!(res.max_queue_observed <= max_queue_depth as usize);
            black_box(reject_ratio(res.rejected, res.offered));
        });
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
