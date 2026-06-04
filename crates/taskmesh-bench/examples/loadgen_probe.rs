//! Loadgen probe (ADR 9000 / P1 + P5): run the open-loop retrieval scenario at a
//! sweep of offered loads and print coordinated-omission-corrected admission-wait
//! latency, reject ratio (fail-closed signal), and the queue high-water mark.
//!
//! Run: `cargo run -p taskmesh-bench --example loadgen_probe --release`

use taskmesh_bench::loadgen::simulate;
use taskmesh_bench::metrics::reject_ratio;
use taskmesh_bench::workload::{
    fixture, generate, mean_interval_ns, retrieval_policy, WorkloadConfig,
};

fn main() {
    // Aggregate server capacity = inflight / service_time. Size the per-task
    // service so the 32 inflight slots sustain ~50k req/s in aggregate; the
    // offered-load sweep then crosses that knee (under- → over-saturated).
    let inflight = 32u32;
    let capacity = 50_000.0_f64;
    let service_ns = (inflight as f64 * 1.0e9 / capacity) as u64;

    println!("# Open-loop retrieval scenario — admission-wait latency (CO-corrected)");
    println!(
        "# aggregate capacity≈{:.0}k/s ({inflight} inflight), queue depth 256, 20k arrivals, seed 0xC0FFEE",
        capacity / 1000.0
    );
    println!(
        "{:>10} {:>9} {:>9} {:>10} {:>9} {:>7}",
        "offered/s", "p50_ns", "p99_ns", "p999_ns", "reject%", "max_q"
    );

    for &lambda in &[10_000.0_f64, 30_000.0, 50_000.0, 100_000.0, 200_000.0] {
        let cfg = WorkloadConfig {
            classes: vec!["retrieval".into()],
            lambda,
            count: 20_000,
            ..Default::default()
        };
        let arrivals = generate(&cfg);
        let interval = mean_interval_ns(lambda);
        let fx = fixture(vec![("retrieval", retrieval_policy(inflight, 256))], 0, 0);
        let (lat, res) = simulate(&fx, &arrivals, service_ns, interval);

        println!(
            "{:>10.0} {:>9} {:>9} {:>10} {:>8.1}% {:>7}",
            lambda,
            lat.p50(),
            lat.p99(),
            lat.p999(),
            reject_ratio(res.rejected, res.offered) * 100.0,
            res.max_queue_observed,
        );
    }
}
