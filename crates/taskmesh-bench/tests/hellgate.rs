//! Hell-gate: deterministic end-to-end invariants for the benchmark harness.
//!
//! These are not timings — they assert that the open-loop simulator, the
//! coordinated-omission latency recorder, the control-plane metrics, and the USL
//! contention path all behave correctly and *fail closed* across seeds and load
//! regimes. If any of these break, the headline benchmark numbers are not
//! trustworthy, so they gate.

use taskmesh_bench::loadgen::{contention_throughput, simulate, SimResult};
use taskmesh_bench::metrics::{argmax_throughput, fit_usl, reject_ratio};
use taskmesh_bench::workload::{
    fixture, generate, mean_interval_ns, retrieval_policy, WorkloadConfig,
};

/// One open-loop run of a single retrieval class at a given offered load and
/// aggregate capacity. Returns (p50, p99, p999, dropped, result).
fn scenario(
    lambda: f64,
    seed: u64,
    inflight: u32,
    depth: u32,
    capacity: f64,
) -> (u64, u64, u64, u64, SimResult) {
    let cfg = WorkloadConfig {
        classes: vec!["retrieval".into()],
        lambda,
        count: 5_000,
        seed,
        ..Default::default()
    };
    let arrivals = generate(&cfg);
    let interval = mean_interval_ns(lambda);
    // Aggregate capacity = inflight / service_time.
    let service_ns = (inflight as f64 * 1.0e9 / capacity) as u64;
    let fx = fixture(vec![("retrieval", retrieval_policy(inflight, depth))], 0, 0);
    let (lat, res) = simulate(&fx, &arrivals, service_ns, interval);
    (lat.p50(), lat.p99(), lat.p999(), lat.dropped(), res)
}

#[test]
fn invariants_hold_across_seeds_and_regimes() {
    let capacity = 50_000.0;
    let depth = 128u32;
    for &seed in &[1u64, 7, 42, 0xC0FFEE, 0xDEADBEEF] {
        for &lambda in &[5_000.0_f64, 25_000.0, 50_000.0, 120_000.0, 300_000.0] {
            let (_p50, _p99, _p999, dropped, res) = scenario(lambda, seed, 32, depth, capacity);
            assert!(
                res.is_conserved(),
                "conservation violated (seed={seed}, λ={lambda}): {res:?}"
            );
            assert_eq!(
                dropped, 0,
                "latency samples dropped (seed={seed}, λ={lambda})"
            );
            assert!(
                res.max_queue_observed <= depth as usize,
                "queue exceeded its bound (seed={seed}, λ={lambda}): {} > {depth}",
                res.max_queue_observed
            );
            // Single class with a free budget always fully drains.
            assert_eq!(res.leftover_queued, 0, "leftover (seed={seed}, λ={lambda})");
        }
    }
}

#[test]
fn overload_fails_closed_not_unbounded() {
    let depth = 64u32;
    // 6× the aggregate capacity.
    let (_p50, _p99, _p999, dropped, res) = scenario(300_000.0, 42, 32, depth, 50_000.0);
    assert_eq!(dropped, 0);
    assert!(res.rejected > 0, "overload must reject: {res:?}");
    assert_eq!(
        res.max_queue_observed, depth as usize,
        "queue must pin at its bound under heavy overload: {res:?}"
    );
    assert!(
        reject_ratio(res.rejected, res.offered) > 0.5,
        "≥half of a 6× overload should be shed: {res:?}"
    );
}

#[test]
fn undersaturation_has_no_backpressure() {
    // A tenth of capacity: nothing should queue or reject.
    let (p50, p99, _p999, _dropped, res) = scenario(5_000.0, 42, 32, 128, 50_000.0);
    assert_eq!(res.rejected, 0);
    assert_eq!(res.max_queue_observed, 0);
    assert_eq!(res.admitted_immediately, res.offered);
    // Admission wait is ~0 for everyone (most requests pay no governance tax).
    assert_eq!(p50, 0);
    assert_eq!(p99, 0);
}

#[test]
fn reject_ratio_and_tail_grow_monotonically_with_load() {
    let capacity = 50_000.0;
    let depth = 128u32;
    let mut last_reject = -1.0_f64;
    let mut last_p99 = 0u64;
    for &lambda in &[
        10_000.0_f64,
        30_000.0,
        50_000.0,
        90_000.0,
        160_000.0,
        300_000.0,
    ] {
        let (_p50, p99, _p999, _d, res) = scenario(lambda, 42, 32, depth, capacity);
        let rr = reject_ratio(res.rejected, res.offered);
        // Reject ratio IS structurally monotone: with fixed capacity/depth, more
        // offered load can only shed a larger fraction.
        assert!(
            rr + 1e-9 >= last_reject,
            "reject ratio must not fall as load rises: {rr} < {last_reject} at λ={lambda}"
        );
        // p99 admission-wait rises only while the queue is still filling. Once the
        // queue saturates (pinned at depth), the wait distribution is depth-bounded
        // and the coordinated-omission backfill (which scales with offered load)
        // can wiggle p99 — so require non-decreasing ONLY pre-saturation.
        let saturated = res.max_queue_observed >= depth as usize;
        assert!(
            p99 >= last_p99 || saturated,
            "p99 must not fall before saturation: {p99} < {last_p99} at λ={lambda}"
        );
        last_reject = rr;
        if !saturated {
            last_p99 = p99;
        }
    }
}

#[test]
fn simulation_is_deterministic_for_a_seed() {
    let a = scenario(120_000.0, 99, 32, 128, 50_000.0);
    let b = scenario(120_000.0, 99, 32, 128, 50_000.0);
    assert_eq!(a.0, b.0, "p50 must be identical");
    assert_eq!(a.1, b.1, "p99 must be identical");
    assert_eq!(a.2, b.2, "p999 must be identical");
    assert_eq!(a.4, b.4, "SimResult must be identical");
}

#[test]
fn usl_contention_sweep_is_structurally_sound() {
    // Real-thread sweep: timing is non-deterministic, so assert only structure —
    // every level produces positive throughput, the fit is recoverable, and the
    // empirical peak is at a real concurrency level.
    let mut samples = Vec::new();
    for &t in &[1usize, 2, 4] {
        let tput = contention_throughput(t, 50_000);
        assert!(tput > 0.0, "throughput must be positive at {t} threads");
        samples.push((t as f64, tput));
    }
    let fit = fit_usl(&samples).expect("USL fit must be recoverable from 3 points");
    assert!(fit.x1 > 0.0);
    let (peak_n, peak_tput) = argmax_throughput(&samples).expect("empirical peak");
    assert!(peak_n >= 1.0 && peak_tput > 0.0);
}
