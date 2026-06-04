//! Control-plane behavioral metrics (ADR 9000 / P5). Deterministic, unit-tested.

/// Jain's fairness index over realized per-class shares: `1.0` is perfectly fair,
/// `1/n` is maximally unfair (one class gets everything).
pub fn jain_fairness_index(shares: &[f64]) -> f64 {
    if shares.is_empty() {
        return 1.0;
    }
    let sum: f64 = shares.iter().sum();
    let sum_sq: f64 = shares.iter().map(|x| x * x).sum();
    if sum_sq == 0.0 {
        return 1.0;
    }
    (sum * sum) / (shares.len() as f64 * sum_sq)
}

/// Goodput: admitted-and-completed requests per second (distinct from raw
/// offered throughput).
pub fn goodput(completed: usize, elapsed_secs: f64) -> f64 {
    if elapsed_secs <= 0.0 {
        return 0.0;
    }
    completed as f64 / elapsed_secs
}

/// Tail amplification: the p99/p50 latency ratio. Rises under overload.
pub fn tail_amplification(p50: f64, p99: f64) -> f64 {
    if p50 <= 0.0 {
        return 1.0;
    }
    p99 / p50
}

/// Reject ratio under load: fraction of offered requests that were rejected
/// (the fail-closed signal).
pub fn reject_ratio(rejected: usize, offered: usize) -> f64 {
    if offered == 0 {
        return 0.0;
    }
    rejected as f64 / offered as f64
}

/// Universal Scalability Law fit (ADR 9000 / P3).
///
/// Characterizes how throughput scales with concurrency under the governor's
/// single global mutex: `α` is the contention (serialization) share, `β` the
/// coherency (cross-talk) cost, and `n_max` the concurrency past which adding
/// workers *reduces* throughput. `None` for `n_max` means unbounded scaling
/// within the measured range (β ≤ 0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UslFit {
    pub alpha: f64,
    pub beta: f64,
    /// Throughput baseline `X(n1)` at the baseline concurrency.
    pub x1: f64,
    /// Baseline concurrency the fit is normalized against (usually 1).
    pub n1: f64,
    pub n_max: Option<f64>,
    pub peak_throughput: Option<f64>,
}

impl UslFit {
    /// Predicted throughput at concurrency `n`. The coefficients live in
    /// concurrency normalized to the baseline `n1`, so `n` is normalized here too
    /// — correct even when the sweep did not start at exactly 1.
    pub fn predict(&self, n: f64) -> f64 {
        let nn = n / self.n1;
        self.x1 * nn / (1.0 + self.alpha * (nn - 1.0) + self.beta * nn * (nn - 1.0))
    }

    /// Whether the recovered coefficients lie in the USL's valid domain
    /// (`0 ≤ α < 1`, `β ≥ 0`). Outside it — e.g. throughput that *drops* as
    /// workers are added (retrograde, contention-bound) — the parametric peak is
    /// meaningless and callers should fall back to the empirical peak.
    pub fn is_in_domain(&self) -> bool {
        (0.0..1.0).contains(&self.alpha) && self.beta >= 0.0
    }
}

/// The measured `(concurrency, throughput)` sample with the highest throughput —
/// the *empirical* scalability peak, independent of any model fit. Honest even
/// when the workload scales retrograde and the USL fit leaves its valid domain.
pub fn argmax_throughput(samples: &[(f64, f64)]) -> Option<(f64, f64)> {
    samples
        .iter()
        .copied()
        .reduce(|a, b| if b.1 > a.1 { b } else { a })
}

/// Fit the USL from `(concurrency, throughput)` samples. Requires the `n=1`
/// baseline plus at least two further points.
///
/// Method (Gunther): with relative capacity `C(N)=X(N)/X(1)`, the deviation
/// `y = N/C − 1` is linear in `(N−1)` and `N(N−1)`. Solve the two coefficients
/// by ordinary least squares with no intercept (a 2×2 normal-equation system).
pub fn fit_usl(samples: &[(f64, f64)]) -> Option<UslFit> {
    // Baseline = the smallest-concurrency sample (expected to be n=1).
    let (n1, x1) = samples
        .iter()
        .copied()
        .reduce(|a, b| if b.0 < a.0 { b } else { a })?;
    if x1 <= 0.0 || n1 <= 0.0 {
        return None;
    }

    let (mut s11, mut s12, mut s22, mut b1, mut b2) = (0.0, 0.0, 0.0, 0.0, 0.0);
    let mut used = 0usize;
    for &(n, x) in samples {
        if n <= n1 || x <= 0.0 {
            continue; // the baseline contributes y=0 with zero regressors
        }
        let nn = n / n1; // normalize so the baseline maps to 1
        let c = x / x1;
        let y = nn / c - 1.0;
        let u1 = nn - 1.0;
        let u2 = nn * (nn - 1.0);
        s11 += u1 * u1;
        s12 += u1 * u2;
        s22 += u2 * u2;
        b1 += u1 * y;
        b2 += u2 * y;
        used += 1;
    }
    if used < 2 {
        return None;
    }
    let det = s11 * s22 - s12 * s12;
    if det.abs() < 1e-12 {
        return None;
    }
    let alpha = (b1 * s22 - b2 * s12) / det;
    let beta = (s11 * b2 - s12 * b1) / det;

    let (n_max, peak) = if beta > 0.0 && alpha < 1.0 {
        let nm = ((1.0 - alpha) / beta).sqrt();
        let peak = x1 * nm / (1.0 + alpha * (nm - 1.0) + beta * nm * (nm - 1.0));
        (Some(nm * n1), Some(peak))
    } else {
        (None, None)
    };

    Some(UslFit {
        alpha,
        beta,
        x1,
        n1,
        n_max,
        peak_throughput: peak,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_shares_are_perfectly_fair() {
        assert!((jain_fairness_index(&[10.0, 10.0, 10.0]) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn single_hog_is_maximally_unfair() {
        // one class gets everything -> 1/n
        let idx = jain_fairness_index(&[30.0, 0.0, 0.0]);
        assert!((idx - 1.0 / 3.0).abs() < 1e-9, "got {idx}");
    }

    #[test]
    fn weighted_4_to_1_is_between() {
        let idx = jain_fairness_index(&[8.0, 2.0]);
        assert!(idx > 1.0 / 2.0 && idx < 1.0, "got {idx}");
    }

    #[test]
    fn goodput_and_reject_ratio() {
        assert_eq!(goodput(100, 2.0), 50.0);
        assert_eq!(reject_ratio(25, 100), 0.25);
        assert_eq!(tail_amplification(10.0, 50.0), 5.0);
    }

    #[test]
    fn usl_recovers_known_coefficients() {
        // Synthesize noiseless data from a known model, then recover α, β exactly.
        let (alpha, beta, x1) = (0.03_f64, 0.0015_f64, 1000.0_f64);
        let model = |n: f64| x1 * n / (1.0 + alpha * (n - 1.0) + beta * n * (n - 1.0));
        let samples: Vec<(f64, f64)> = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0]
            .iter()
            .map(|&n| (n, model(n)))
            .collect();

        let fit = fit_usl(&samples).expect("fit");
        assert!((fit.alpha - alpha).abs() < 1e-6, "alpha={}", fit.alpha);
        assert!((fit.beta - beta).abs() < 1e-9, "beta={}", fit.beta);
        // n_max = sqrt((1-α)/β).
        let expected_nmax = ((1.0 - alpha) / beta).sqrt();
        assert!((fit.n_max.unwrap() - expected_nmax).abs() < 1e-3);
        // Prediction round-trips against the source model.
        assert!((fit.predict(8.0) - model(8.0)).abs() < 1e-6);
    }

    #[test]
    fn usl_linear_scaling_has_no_peak() {
        // α=β=0 → perfect linear scaling → no throughput peak.
        let samples: Vec<(f64, f64)> = (1..=8).map(|n| (n as f64, 100.0 * n as f64)).collect();
        let fit = fit_usl(&samples).expect("fit");
        assert!(fit.alpha.abs() < 1e-9, "alpha={}", fit.alpha);
        assert!(fit.beta.abs() < 1e-9, "beta={}", fit.beta);
        assert!(fit.n_max.is_none());
    }

    #[test]
    fn usl_needs_enough_points() {
        assert!(fit_usl(&[(1.0, 100.0)]).is_none());
        assert!(fit_usl(&[(1.0, 100.0), (2.0, 180.0)]).is_none());
    }

    #[test]
    fn argmax_finds_empirical_peak() {
        // Retrograde data: throughput drops after n=1 (contention-bound).
        let s = [(1.0, 2.7e6), (2.0, 1.9e6), (4.0, 1.6e6), (8.0, 1.9e6)];
        assert_eq!(argmax_throughput(&s), Some((1.0, 2.7e6)));
        assert_eq!(argmax_throughput(&[]), None);
    }

    #[test]
    fn retrograde_scaling_is_flagged_out_of_domain() {
        // Throughput that *falls* as workers are added must NOT be reported as
        // near-linear: the fit leaves the USL's valid domain (α≥1 and/or β<0).
        let samples = [
            (1.0, 2.7e6),
            (2.0, 1.9e6),
            (4.0, 1.6e6),
            (8.0, 1.9e6),
            (16.0, 2.1e6),
        ];
        let fit = fit_usl(&samples).expect("fit");
        assert!(
            !fit.is_in_domain(),
            "retrograde data must be out-of-domain: α={} β={}",
            fit.alpha,
            fit.beta
        );
    }

    #[test]
    fn predict_is_consistent_when_baseline_is_not_one() {
        // Sweep that starts at n=2 (no n=1 sample). The fit normalizes to n1=2;
        // predict() must use the same normalization. Generate data from the
        // normalized model so an exact round-trip is the correctness criterion.
        let (alpha, beta, x1, n1) = (0.04_f64, 0.0008_f64, 700.0_f64, 2.0_f64);
        let model = |n: f64| {
            let nn = n / n1;
            x1 * nn / (1.0 + alpha * (nn - 1.0) + beta * nn * (nn - 1.0))
        };
        let samples: Vec<(f64, f64)> = [2.0, 4.0, 8.0, 16.0, 32.0]
            .iter()
            .map(|&n| (n, model(n)))
            .collect();
        let fit = fit_usl(&samples).expect("fit");
        assert_eq!(fit.n1, 2.0);
        assert!((fit.x1 - x1).abs() < 1e-6);
        // predict() reproduces the normalized curve (would fail with the old
        // raw-n formula that ignored n1).
        assert!(
            (fit.predict(2.0) - x1).abs() < 1e-6,
            "baseline must reproduce x1"
        );
        assert!((fit.predict(8.0) - model(8.0)).abs() / model(8.0) < 1e-6);
        assert!((fit.predict(32.0) - model(32.0)).abs() / model(32.0) < 1e-6);
    }

    #[test]
    fn healthy_scaling_is_in_domain() {
        let (alpha, beta, x1) = (0.05_f64, 0.001_f64, 1000.0_f64);
        let model = |n: f64| x1 * n / (1.0 + alpha * (n - 1.0) + beta * n * (n - 1.0));
        let samples: Vec<(f64, f64)> = [1.0, 2.0, 4.0, 8.0, 16.0]
            .iter()
            .map(|&n| (n, model(n)))
            .collect();
        let fit = fit_usl(&samples).expect("fit");
        assert!(fit.is_in_domain());
        assert!(fit.n_max.is_some());
    }
}
