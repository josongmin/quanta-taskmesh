//! Workload generation (ADR 9000 / P4): Poisson arrivals + Zipfian class
//! popularity, fully seeded for reproducibility. Shared single source so bench
//! and proof fixtures agree.

use std::collections::BTreeMap;
use std::sync::Arc;

use rand::rngs::StdRng;
use rand::SeedableRng;
use rand_distr::{Distribution, Exp, Zipf};
use taskmesh_contract::{
    ClassPolicy, ManualClock, OverflowPolicy, ResourceBudget, RetryAfterPolicy, TaskClass, TaskSpec,
};
use taskmesh_engine::{Governor, PolicySet};

/// One arrival in an open-loop schedule: an *intended send time* and the class
/// to admit at that time.
#[derive(Debug, Clone, PartialEq)]
pub struct Arrival {
    pub send_time_secs: f64,
    pub class: String,
}

/// Synthetic workload configuration.
#[derive(Debug, Clone)]
pub struct WorkloadConfig {
    /// Candidate class names (index 0 is the hottest under Zipf).
    pub classes: Vec<String>,
    /// Poisson arrival rate (requests/sec).
    pub lambda: f64,
    /// Zipf exponent for class popularity (higher = more skewed).
    pub zipf_exponent: f64,
    pub count: usize,
    pub seed: u64,
}

impl Default for WorkloadConfig {
    fn default() -> Self {
        Self {
            classes: vec!["retrieval".into(), "rank".into(), "index".into()],
            lambda: 10_000.0,
            zipf_exponent: 1.1,
            count: 10_000,
            seed: 0xC0FFEE,
        }
    }
}

/// Generate an open-loop arrival schedule. Inter-arrival times are
/// `Exp(lambda)` (Poisson process); class selection is Zipfian over `classes`.
pub fn generate(cfg: &WorkloadConfig) -> Vec<Arrival> {
    assert!(!cfg.classes.is_empty(), "workload needs at least one class");
    let mut rng = StdRng::seed_from_u64(cfg.seed);
    let inter = Exp::new(cfg.lambda).expect("lambda > 0");
    let zipf = Zipf::new(cfg.classes.len() as u64, cfg.zipf_exponent).expect("valid zipf");

    let mut t = 0.0_f64;
    let mut out = Vec::with_capacity(cfg.count);
    for _ in 0..cfg.count {
        t += inter.sample(&mut rng);
        // Zipf yields a value in [1, n]; map to a 0-based class index.
        let rank = zipf.sample(&mut rng) as usize;
        let idx = rank.saturating_sub(1).min(cfg.classes.len() - 1);
        out.push(Arrival {
            send_time_secs: t,
            class: cfg.classes[idx].clone(),
        });
    }
    out
}

/// Mean inter-arrival interval in nanoseconds for a Poisson rate of `lambda`
/// requests/sec — the expected interval the latency recorder corrects against.
pub fn mean_interval_ns(lambda: f64) -> u64 {
    if lambda <= 0.0 {
        return 1;
    }
    (1.0e9 / lambda).round().max(1.0) as u64
}

/// A governor under test together with the manual clock that drives its virtual
/// time. The clock is held so the simulator can advance it deterministically
/// (ADR 9000 / P1: a counter clock, no `SystemTime` syscall noise).
pub struct Fixture {
    pub governor: Arc<Governor>,
    pub clock: Arc<ManualClock>,
}

/// A queueable retrieval-style class: bounded inflight, bounded queue, adaptive
/// backpressure — the canonical README workload.
pub fn retrieval_policy(max_inflight: u32, max_queue_depth: u32) -> ClassPolicy {
    ClassPolicy::new()
        .max_inflight(max_inflight)
        .max_queue_depth(max_queue_depth)
        .cpu_units(1)
        .memory_units(1)
        .overflow_policy(OverflowPolicy::QueueWithinDepth)
        .retry_after_policy(RetryAfterPolicy::Adaptive)
}

/// Build a fixture from `(class_name, policy)` pairs and global budgets. The
/// clock starts at 0; budgets are the global CPU/memory unit ceilings.
pub fn fixture(classes: Vec<(&str, ClassPolicy)>, cpu_budget: u32, mem_budget: u32) -> Fixture {
    let map: BTreeMap<TaskClass, ClassPolicy> = classes
        .into_iter()
        .map(|(name, policy)| (TaskClass::new(name.to_string()), policy))
        .collect();
    let resources = ResourceBudget::new()
        .cpu_units(cpu_budget)
        .memory_units(mem_budget);
    let clock = Arc::new(ManualClock::new(0));
    let governor = Arc::new(Governor::new(PolicySet::new(resources, map), clock.clone()));
    Fixture { governor, clock }
}

/// A root blocking task for `class`, keyed by a unique `op` (callers must keep
/// `op` unique among concurrently-inflight requests so the recursion guard does
/// not treat a second same-keyed admit as a recursive loop).
pub fn root_spec(class: &str, op: &str) -> TaskSpec {
    TaskSpec::blocking(TaskClass::new(class.to_string())).operation(op.to_string())
}

// ---- trace record / replay (ADR 9000 / P4) --------------------------------

/// Serialize an arrival schedule to a stable CSV trace: one
/// `send_time_secs,class` record per line. `f64` uses the shortest
/// round-trippable representation, so `trace_from_csv ∘ trace_to_csv` is the
/// identity. Class names must not contain `,` or newlines (they are stable
/// product-neutral identifiers, so they do not).
pub fn trace_to_csv(arrivals: &[Arrival]) -> String {
    let mut out = String::from("# send_time_secs,class\n");
    for a in arrivals {
        out.push_str(&format!("{},{}\n", a.send_time_secs, a.class));
    }
    out
}

/// Parse a CSV trace produced by [`trace_to_csv`] (or hand-authored / recorded
/// elsewhere). Blank lines and `#` comments are ignored. Returns the offending
/// line on malformed input rather than panicking, so a bad trace fails loudly.
pub fn trace_from_csv(text: &str) -> Result<Vec<Arrival>, String> {
    let mut out = Vec::new();
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (secs, class) = line
            .split_once(',')
            .ok_or_else(|| format!("line {}: missing comma: {raw:?}", lineno + 1))?;
        let send_time_secs = secs
            .trim()
            .parse::<f64>()
            .map_err(|e| format!("line {}: bad send_time {secs:?}: {e}", lineno + 1))?;
        let class = class.trim();
        if class.is_empty() {
            return Err(format!("line {}: empty class", lineno + 1));
        }
        out.push(Arrival {
            send_time_secs,
            class: class.to_string(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_deterministic_for_seed() {
        let cfg = WorkloadConfig::default();
        assert_eq!(generate(&cfg), generate(&cfg));
    }

    #[test]
    fn send_times_are_monotonic() {
        let arrivals = generate(&WorkloadConfig::default());
        assert!(arrivals
            .windows(2)
            .all(|w| w[1].send_time_secs >= w[0].send_time_secs));
    }

    #[test]
    fn hottest_class_dominates_under_skew() {
        let cfg = WorkloadConfig {
            count: 20_000,
            zipf_exponent: 1.3,
            ..Default::default()
        };
        let arrivals = generate(&cfg);
        let hot = arrivals.iter().filter(|a| a.class == "retrieval").count();
        let cold = arrivals.iter().filter(|a| a.class == "index").count();
        assert!(hot > cold, "hot={hot} cold={cold}");
    }

    #[test]
    fn trace_csv_round_trips_exactly() {
        let arrivals = generate(&WorkloadConfig::default());
        let csv = trace_to_csv(&arrivals);
        let parsed = trace_from_csv(&csv).expect("valid trace");
        assert_eq!(parsed, arrivals, "trace must round-trip without loss");
    }

    #[test]
    fn trace_parser_ignores_comments_and_blanks_and_reports_errors() {
        let text = "# header\n\n0.5,retrieval\n  1.25 , rank \n";
        let parsed = trace_from_csv(text).expect("valid");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1].class, "rank");
        assert!(trace_from_csv("not_a_number,retrieval").is_err());
        assert!(trace_from_csv("0.5").is_err()); // missing comma
        assert!(trace_from_csv("0.5,").is_err()); // empty class
    }

    #[test]
    fn replaying_a_recorded_trace_reproduces_the_simulation() {
        use crate::loadgen::simulate;
        // Record a workload, persist to CSV, replay, and confirm the governed
        // simulation is byte-identical — traces are a faithful replay source.
        let cfg = WorkloadConfig {
            classes: vec!["retrieval".into()],
            lambda: 80_000.0,
            count: 3_000,
            ..Default::default()
        };
        let recorded = generate(&cfg);
        let replayed = trace_from_csv(&trace_to_csv(&recorded)).expect("replay");

        let interval = mean_interval_ns(cfg.lambda);
        let service_ns = 32 * interval;
        let run = |arr: &[Arrival]| {
            let fx = fixture(vec![("retrieval", retrieval_policy(32, 128))], 0, 0);
            simulate(&fx, arr, service_ns, interval).1
        };
        assert_eq!(run(&recorded), run(&replayed));
    }
}
