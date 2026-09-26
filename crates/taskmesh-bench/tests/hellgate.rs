//! Hell-gate: deterministic end-to-end invariants for the benchmark harness.
//!
//! These are not timings — they assert that the open-loop simulator, the
//! raw latency recorder, and the control-plane metrics behave correctly and
//! *fail closed* across seeds and load regimes. If any of these break, the
//! headline benchmark numbers are not trustworthy, so they gate.

use taskmesh_bench::loadgen::{simulate, SimResult};
use taskmesh_bench::metrics::reject_ratio;
use taskmesh_bench::workload::{fixture, generate, retrieval_policy, WorkloadConfig};

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
    let arrivals = generate(&cfg).expect("valid workload config");
    // Aggregate capacity = inflight / service_time.
    let service_ns = (inflight as f64 * 1.0e9 / capacity) as u64;
    let fx = fixture(vec![("retrieval", retrieval_policy(inflight, depth))], 0, 0);
    let (lat, res) = simulate(&fx, &arrivals, service_ns).expect("valid schedule");
    (lat.p50(), lat.p99(), lat.p999(), lat.dropped(), res)
}

fn check_regimes(seed: u64) {
    let capacity = 50_000.0;
    let depth = 128u32;
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

#[test]
fn invariants_hold_across_regimes_seed_one() {
    check_regimes(1);
}

#[test]
fn invariants_hold_across_regimes_seed_seven() {
    check_regimes(7);
}

#[test]
fn invariants_hold_across_regimes_seed_forty_two() {
    check_regimes(42);
}

#[test]
fn invariants_hold_across_regimes_seed_coffee() {
    check_regimes(0xC0FFEE);
}

#[test]
fn invariants_hold_across_regimes_seed_deadbeef() {
    check_regimes(0xDEADBEEF);
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
fn multiclass_multiseed_population_has_no_unaccounted_request() {
    let classes = ["retrieval", "rank", "index"];
    for seed in [1_u64, 7, 42, 0xC0FFEE] {
        let cfg = WorkloadConfig {
            classes: classes.into_iter().map(str::to_owned).collect(),
            lambda: 120_000.0,
            zipf_exponent: 1.1,
            count: 3_000,
            seed,
        };
        let arrivals = generate(&cfg).expect("valid multiclass schedule");
        for class in classes {
            assert!(
                arrivals
                    .as_slice()
                    .iter()
                    .any(|arrival| arrival.class == class),
                "seed {seed} must exercise {class}"
            );
        }
        let fixture = fixture(
            vec![
                ("retrieval", retrieval_policy(8, 32)),
                ("rank", retrieval_policy(4, 16)),
                ("index", retrieval_policy(2, 8)),
            ],
            14,
            0,
        );
        let (latency, result) = simulate(&fixture, &arrivals, 200_000).expect("bounded simulation");
        assert!(result.is_conserved(), "seed {seed}: {result:?}");
        assert_eq!(latency.len(), result.started() as u64);
        assert!(result.max_queue_observed <= 32 + 16 + 8);
        assert_eq!(result.leftover_queued, 0);
    }
}
