//! Workload generation (ADR 9000 / P4): Poisson arrivals + Zipfian class
//! popularity, fully seeded for reproducibility. Shared single source so bench
//! and proof fixtures agree.
//!
//! # Validation is not optional
//!
//! A schedule is *input to a proof*. A `NaN` send time that silently becomes
//! zero, a negative time, or a trace that runs backwards does not fail a replay
//! — it quietly distorts what the replay measures, and the conservation checks
//! downstream cannot see it because every request still completes. So every
//! entry point that accepts arrivals validates them first, and every generator
//! validates its configuration before sampling. A configuration that cannot
//! make progress (a zero mean dwell, for instance) is a typed error, never a
//! loop that does not return.

use std::collections::BTreeMap;
use std::fmt;
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

/// Largest send time (seconds) a schedule may carry.
///
/// Times are converted to whole nanoseconds for the discrete-event simulator.
/// `2^53` ns (~104 days) is the largest range over which an `f64` still
/// represents every nanosecond exactly, so a schedule inside it converts
/// without rounding *and* leaves headroom for `t + service_ns` in `u64`.
pub const MAX_SEND_TIME_SECS: f64 = 9_007_199.254_740_992; // 2^53 ns

/// Why a schedule or a generator configuration was refused.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum WorkloadError {
    /// `send_time_secs` is NaN or infinite.
    NonFiniteSendTime { index: usize, value: f64 },
    /// `send_time_secs` is negative.
    NegativeSendTime { index: usize, value: f64 },
    /// `send_time_secs` exceeds [`MAX_SEND_TIME_SECS`].
    SendTimeOutOfRange { index: usize, value: f64 },
    /// The schedule runs backwards. Equal times are allowed (simultaneous
    /// arrivals); a decrease is not, and is never silently re-sorted.
    NonMonotonicSendTime {
        index: usize,
        previous: f64,
        value: f64,
    },
    /// An arrival names an empty class.
    EmptyClass { index: usize },
    /// A configuration value is not usable as stated.
    InvalidConfig { field: &'static str, reason: String },
    /// `t + service` does not fit the simulator's `u64` nanosecond clock.
    CompletionTimeOverflow { arrival_ns: u64, service_ns: u64 },
    /// A generator crossed too many phase boundaries without producing an
    /// arrival. Reachable only with a degenerate configuration that validation
    /// did not catch; reported rather than looped on.
    NoProgress { after_boundaries: usize },
}

impl fmt::Display for WorkloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteSendTime { index, value } => {
                write!(f, "arrival {index}: send time {value} is not finite")
            }
            Self::NegativeSendTime { index, value } => {
                write!(f, "arrival {index}: send time {value} is negative")
            }
            Self::SendTimeOutOfRange { index, value } => write!(
                f,
                "arrival {index}: send time {value}s exceeds the representable range ({MAX_SEND_TIME_SECS}s)"
            ),
            Self::NonMonotonicSendTime {
                index,
                previous,
                value,
            } => write!(
                f,
                "arrival {index}: send time {value} precedes the previous arrival at {previous}"
            ),
            Self::EmptyClass { index } => write!(f, "arrival {index}: empty class"),
            Self::InvalidConfig { field, reason } => write!(f, "config.{field}: {reason}"),
            Self::CompletionTimeOverflow {
                arrival_ns,
                service_ns,
            } => write!(
                f,
                "completion time {arrival_ns} + {service_ns} ns does not fit the u64 clock"
            ),
            Self::NoProgress { after_boundaries } => write!(
                f,
                "generator crossed {after_boundaries} phase boundaries without an arrival"
            ),
        }
    }
}

impl std::error::Error for WorkloadError {}

/// An arrival schedule that has been checked: finite, non-negative,
/// non-decreasing send times inside the representable range, non-empty classes.
///
/// This is the only thing the simulator accepts. A raw `&[Arrival]` cannot reach
/// the discrete-event loop without going through [`ValidatedArrivals::new`].
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedArrivals {
    arrivals: Vec<Arrival>,
    send_times_ns: Vec<u64>,
}

impl ValidatedArrivals {
    /// Validate a schedule. Returns the first offending arrival.
    pub fn new(arrivals: Vec<Arrival>) -> Result<Self, WorkloadError> {
        let mut send_times_ns = Vec::with_capacity(arrivals.len());
        let mut previous: Option<f64> = None;
        for (index, arrival) in arrivals.iter().enumerate() {
            let value = arrival.send_time_secs;
            if !value.is_finite() {
                return Err(WorkloadError::NonFiniteSendTime { index, value });
            }
            if value < 0.0 {
                return Err(WorkloadError::NegativeSendTime { index, value });
            }
            if value > MAX_SEND_TIME_SECS {
                return Err(WorkloadError::SendTimeOutOfRange { index, value });
            }
            if let Some(previous) = previous {
                if value < previous {
                    return Err(WorkloadError::NonMonotonicSendTime {
                        index,
                        previous,
                        value,
                    });
                }
            }
            if arrival.class.is_empty() {
                return Err(WorkloadError::EmptyClass { index });
            }
            previous = Some(value);
            // Exact for every value inside MAX_SEND_TIME_SECS (see its docs).
            send_times_ns.push((value * 1.0e9).round() as u64);
        }
        Ok(Self {
            arrivals,
            send_times_ns,
        })
    }

    pub fn from_slice(arrivals: &[Arrival]) -> Result<Self, WorkloadError> {
        Self::new(arrivals.to_vec())
    }

    pub fn as_slice(&self) -> &[Arrival] {
        &self.arrivals
    }

    /// Send times in whole nanoseconds, index-aligned with [`Self::as_slice`].
    pub fn send_times_ns(&self) -> &[u64] {
        &self.send_times_ns
    }

    pub fn len(&self) -> usize {
        self.arrivals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.arrivals.is_empty()
    }

    pub fn into_inner(self) -> Vec<Arrival> {
        self.arrivals
    }
}

fn require_finite_positive(field: &'static str, value: f64) -> Result<(), WorkloadError> {
    if !value.is_finite() {
        return Err(WorkloadError::InvalidConfig {
            field,
            reason: format!("{value} is not finite"),
        });
    }
    if value <= 0.0 {
        return Err(WorkloadError::InvalidConfig {
            field,
            reason: format!("{value} must be > 0"),
        });
    }
    Ok(())
}

fn require_classes(classes: &[String]) -> Result<(), WorkloadError> {
    if classes.is_empty() {
        return Err(WorkloadError::InvalidConfig {
            field: "classes",
            reason: "at least one class is required".into(),
        });
    }
    if let Some(index) = classes.iter().position(String::is_empty) {
        return Err(WorkloadError::InvalidConfig {
            field: "classes",
            reason: format!("class {index} is empty"),
        });
    }
    Ok(())
}

fn require_zipf(exponent: f64) -> Result<(), WorkloadError> {
    if !exponent.is_finite() || exponent < 0.0 {
        return Err(WorkloadError::InvalidConfig {
            field: "zipf_exponent",
            reason: format!("{exponent} must be finite and >= 0"),
        });
    }
    Ok(())
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

impl WorkloadConfig {
    pub fn validate(&self) -> Result<(), WorkloadError> {
        require_classes(&self.classes)?;
        require_finite_positive("lambda", self.lambda)?;
        require_zipf(self.zipf_exponent)
    }
}

/// Generate an open-loop arrival schedule. Inter-arrival times are
/// `Exp(lambda)` (Poisson process); class selection is Zipfian over `classes`.
///
/// The result is already validated: a generator that could emit a schedule its
/// own simulator would refuse is a generator with a bug.
pub fn generate(cfg: &WorkloadConfig) -> Result<ValidatedArrivals, WorkloadError> {
    cfg.validate()?;
    let mut rng = StdRng::seed_from_u64(cfg.seed);
    let inter = Exp::new(cfg.lambda).map_err(|e| WorkloadError::InvalidConfig {
        field: "lambda",
        reason: e.to_string(),
    })?;
    let zipf = Zipf::new(cfg.classes.len() as u64, cfg.zipf_exponent).map_err(|e| {
        WorkloadError::InvalidConfig {
            field: "zipf_exponent",
            reason: e.to_string(),
        }
    })?;

    let mut t = 0.0_f64;
    let mut out = Vec::with_capacity(cfg.count);
    for _ in 0..cfg.count {
        t += inter.sample(&mut rng);
        // Zipf yields a value in [1, n]; map to a 0-based class index.
        let rank = zipf.sample(&mut rng) as usize;
        let idx = rank
            .checked_sub(1)
            .expect("Zipf distribution returns ranks starting at one");
        out.push(Arrival {
            send_time_secs: t,
            class: cfg.classes[idx].clone(),
        });
    }
    ValidatedArrivals::new(out)
}

/// A Markov-modulated Poisson process (MMPP) for *bursty* arrivals (ADR 9000 /
/// P4). A 2-state CTMC alternates between an `off` phase (rate `low_lambda`) and
/// an `on` phase (rate `high_lambda`); dwell times in each phase are
/// exponentially distributed. Uniform Poisson gives false stability; real
/// retrieval/indexing load arrives in bursts, and MMPP reproduces that while
/// staying seeded and deterministic.
#[derive(Debug, Clone)]
pub struct BurstConfig {
    pub classes: Vec<String>,
    pub zipf_exponent: f64,
    pub count: usize,
    pub seed: u64,
    /// Baseline (off-phase) arrival rate, requests/sec.
    pub low_lambda: f64,
    /// Burst (on-phase) arrival rate, requests/sec (`>= low_lambda`).
    pub high_lambda: f64,
    /// Mean dwell time in the off phase, seconds.
    pub mean_off_secs: f64,
    /// Mean dwell time in the on phase, seconds.
    pub mean_on_secs: f64,
}

impl Default for BurstConfig {
    fn default() -> Self {
        Self {
            classes: vec!["retrieval".into(), "rank".into(), "index".into()],
            zipf_exponent: 1.1,
            count: 10_000,
            seed: 0xB0_0B5,
            low_lambda: 2_000.0,
            high_lambda: 50_000.0,
            mean_off_secs: 0.010,
            mean_on_secs: 0.002,
        }
    }
}

impl BurstConfig {
    pub fn validate(&self) -> Result<(), WorkloadError> {
        require_classes(&self.classes)?;
        require_zipf(self.zipf_exponent)?;
        require_finite_positive("low_lambda", self.low_lambda)?;
        require_finite_positive("high_lambda", self.high_lambda)?;
        if self.high_lambda < self.low_lambda {
            return Err(WorkloadError::InvalidConfig {
                field: "high_lambda",
                reason: format!(
                    "{} must be >= low_lambda {}",
                    self.high_lambda, self.low_lambda
                ),
            });
        }
        require_finite_positive("mean_off_secs", self.mean_off_secs)?;
        require_finite_positive("mean_on_secs", self.mean_on_secs)
    }

    /// The long-run offered rate of this MMPP, from the CTMC's stationary phase
    /// occupancy: `(λ_low·E[off] + λ_high·E[on]) / (E[off] + E[on])`.
    ///
    /// An independent reference — it is derived from the model, not from the
    /// generator — so a generator that skips phases shows up as a rate that
    /// does not converge to this.
    pub fn stationary_rate(&self) -> f64 {
        let total = self.mean_off_secs + self.mean_on_secs;
        (self.low_lambda * self.mean_off_secs + self.high_lambda * self.mean_on_secs) / total
    }

    /// Stationary fraction of time spent in the burst (`on`) phase.
    pub fn on_occupancy(&self) -> f64 {
        self.mean_on_secs / (self.mean_off_secs + self.mean_on_secs)
    }
}

/// Bounded number of consecutive phase boundaries the generator may cross
/// without producing an arrival before it reports [`WorkloadError::NoProgress`].
const MAX_BOUNDARIES_WITHOUT_ARRIVAL: usize = 1_000_000;

/// Generate a bursty open-loop schedule via [`BurstConfig`]'s MMPP. Arrivals
/// cluster during on-phases and thin out during off-phases; class selection is
/// Zipfian as in [`generate`]. Send times are monotonic and seed-deterministic.
///
/// # Event ordering
///
/// The next event is whichever comes first: the next arrival *at the current
/// phase's rate*, or the phase boundary. When the boundary comes first, time
/// advances to the boundary, the rate switches, and a fresh inter-arrival is
/// drawn at the new rate. Exponential inter-arrivals are memoryless, so
/// redrawing at the boundary is exactly a Poisson process with a piecewise-
/// constant rate — which is what an MMPP is.
///
/// The previous implementation drew the whole inter-arrival at the *old* rate
/// and only then noticed it had crossed one or more boundaries. A long off-phase
/// draw therefore skipped entire high-rate phases, and the realized rate came
/// out ~200× below the configured stationary rate.
///
/// # Generator version
///
/// `GENERATOR_VERSION` is bumped when the sampling order changes. Two traces
/// with different generator versions are not the same workload even for the
/// same seed, and must not be compared as such.
pub fn generate_bursty(cfg: &BurstConfig) -> Result<ValidatedArrivals, WorkloadError> {
    cfg.validate()?;
    let mut rng = StdRng::seed_from_u64(cfg.seed);
    let config_error = |field: &'static str| {
        move |e: rand_distr::ExpError| WorkloadError::InvalidConfig {
            field,
            reason: e.to_string(),
        }
    };
    let off = Exp::new(cfg.low_lambda).map_err(config_error("low_lambda"))?;
    let on = Exp::new(cfg.high_lambda).map_err(config_error("high_lambda"))?;
    let off_dwell = Exp::new(1.0 / cfg.mean_off_secs).map_err(config_error("mean_off_secs"))?;
    let on_dwell = Exp::new(1.0 / cfg.mean_on_secs).map_err(config_error("mean_on_secs"))?;
    let zipf = Zipf::new(cfg.classes.len() as u64, cfg.zipf_exponent).map_err(|e| {
        WorkloadError::InvalidConfig {
            field: "zipf_exponent",
            reason: e.to_string(),
        }
    })?;

    let mut t = 0.0_f64;
    let mut in_burst = false;
    let mut phase_end = t + off_dwell.sample(&mut rng);
    let mut out = Vec::with_capacity(cfg.count);
    while out.len() < cfg.count {
        let mut boundaries_crossed = 0usize;
        loop {
            let inter = if in_burst {
                on.sample(&mut rng)
            } else {
                off.sample(&mut rng)
            };
            let candidate = t + inter;
            if arrival_precedes_boundary(candidate, phase_end) {
                t = candidate;
                break;
            }
            // The boundary comes first. Advance to it, switch phase, and draw
            // again at the new rate.
            t = phase_end;
            in_burst = !in_burst;
            let dwell = if in_burst { &on_dwell } else { &off_dwell };
            phase_end = t + dwell.sample(&mut rng);
            boundaries_crossed = count_crossed_boundary(boundaries_crossed)?;
        }
        let rank = zipf.sample(&mut rng) as usize;
        let idx = rank
            .checked_sub(1)
            .expect("Zipf distribution returns ranks starting at one");
        out.push(Arrival {
            send_time_secs: t,
            class: cfg.classes[idx].clone(),
        });
    }
    ValidatedArrivals::new(out)
}

fn arrival_precedes_boundary(candidate: f64, phase_end: f64) -> bool {
    candidate < phase_end
}

fn count_crossed_boundary(current: usize) -> Result<usize, WorkloadError> {
    let next = current.checked_add(1).ok_or(WorkloadError::NoProgress {
        after_boundaries: usize::MAX,
    })?;
    if next > MAX_BOUNDARIES_WITHOUT_ARRIVAL {
        return Err(WorkloadError::NoProgress {
            after_boundaries: next,
        });
    }
    Ok(next)
}

/// Sampling-order version of [`generate`] / [`generate_bursty`]. See the
/// generator docs for why this matters when comparing traces.
pub const GENERATOR_VERSION: u32 = 2;

/// Coefficient of variation of inter-arrival times: `1.0` for a pure Poisson
/// process, `> 1.0` for bursty (over-dispersed) arrivals. A burstiness statistic.
pub fn interarrival_cv(arrivals: &[Arrival]) -> f64 {
    if arrivals.len() < 3 {
        return 0.0;
    }
    let gaps: Vec<f64> = arrivals
        .windows(2)
        .map(|w| w[1].send_time_secs - w[0].send_time_secs)
        .collect();
    let n = gaps.len() as f64;
    let mean = gaps.iter().sum::<f64>() / n;
    if mean <= 0.0 {
        return 0.0;
    }
    let var = gaps.iter().map(|g| (g - mean).powi(2)).sum::<f64>() / n;
    var.sqrt() / mean
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
    let governor = Arc::new(Governor::new_unchecked(
        PolicySet::new(resources, map),
        clock.clone(),
    ));
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
///
/// "Parses as a float" is not "is a valid event time": `NaN`, `inf`, negative,
/// and out-of-order times all parse. They are rejected here with the line
/// number, and a reversed trace is *never* re-sorted — that would hide the
/// recording error the trace is supposed to expose.
pub fn trace_from_csv(text: &str) -> Result<ValidatedArrivals, String> {
    let mut out = Vec::new();
    let mut line_of: Vec<usize> = Vec::new();
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line_number = human_line_number(lineno);
        let (secs, class) = line
            .split_once(',')
            .ok_or_else(|| format!("line {line_number}: missing comma: {raw:?}"))?;
        let send_time_secs = secs
            .trim()
            .parse::<f64>()
            .map_err(|e| format!("line {line_number}: bad send_time {secs:?}: {e}"))?;
        let class = class.trim();
        if class.is_empty() {
            return Err(format!("line {line_number}: empty class"));
        }
        out.push(Arrival {
            send_time_secs,
            class: class.to_string(),
        });
        line_of.push(line_number);
    }
    ValidatedArrivals::new(out).map_err(|error| {
        let index = match &error {
            WorkloadError::NonFiniteSendTime { index, .. }
            | WorkloadError::NegativeSendTime { index, .. }
            | WorkloadError::SendTimeOutOfRange { index, .. }
            | WorkloadError::NonMonotonicSendTime { index, .. }
            | WorkloadError::EmptyClass { index } => Some(*index),
            _ => None,
        };
        match index.and_then(|i| line_of.get(i)) {
            Some(line) => format!("line {line}: {error}"),
            None => error.to_string(),
        }
    })
}

fn human_line_number(zero_based: usize) -> usize {
    zero_based
        .checked_add(1)
        .expect("a Rust string cannot contain usize::MAX lines")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_deterministic_for_seed() {
        let cfg = WorkloadConfig::default();
        assert_eq!(generate(&cfg).unwrap(), generate(&cfg).unwrap());
    }

    #[test]
    fn validated_arrival_accessors_preserve_empty_and_owned_values() {
        let empty = ValidatedArrivals::new(vec![]).expect("empty schedule is valid");
        assert!(empty.is_empty());
        let arrivals = vec![Arrival {
            send_time_secs: 0.25,
            class: "c".to_owned(),
        }];
        let validated = ValidatedArrivals::from_slice(&arrivals).expect("valid");
        assert!(!validated.is_empty());
        assert_eq!(validated.into_inner(), arrivals);
    }

    #[test]
    fn config_validation_owns_class_and_zipf_boundaries() {
        let no_classes = WorkloadConfig {
            classes: vec![],
            ..WorkloadConfig::default()
        };
        assert!(matches!(
            no_classes.validate(),
            Err(WorkloadError::InvalidConfig {
                field: "classes",
                ..
            })
        ));
        let empty_class = WorkloadConfig {
            classes: vec![String::new()],
            ..WorkloadConfig::default()
        };
        assert!(matches!(
            empty_class.validate(),
            Err(WorkloadError::InvalidConfig {
                field: "classes",
                ..
            })
        ));
        assert!(WorkloadConfig {
            zipf_exponent: 0.0,
            ..WorkloadConfig::default()
        }
        .validate()
        .is_ok());
        for exponent in [-1.0, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                WorkloadConfig {
                    zipf_exponent: exponent,
                    ..WorkloadConfig::default()
                }
                .validate(),
                Err(WorkloadError::InvalidConfig {
                    field: "zipf_exponent",
                    ..
                })
            ));
        }
    }

    #[test]
    fn interval_helpers_have_exact_small_boundaries() {
        assert_eq!(interarrival_cv(&[]), 0.0);
        assert_eq!(
            interarrival_cv(&[
                Arrival {
                    send_time_secs: 0.0,
                    class: "c".into()
                },
                Arrival {
                    send_time_secs: 1.0,
                    class: "c".into()
                },
            ]),
            0.0
        );
        let varied = [
            Arrival {
                send_time_secs: 0.0,
                class: "c".into(),
            },
            Arrival {
                send_time_secs: 1.0,
                class: "c".into(),
            },
            Arrival {
                send_time_secs: 3.0,
                class: "c".into(),
            },
        ];
        assert!(interarrival_cv(&varied) > 0.0);
        assert_eq!(mean_interval_ns(0.0), 1);
        assert_eq!(mean_interval_ns(-1.0), 1);
        assert_eq!(mean_interval_ns(2.0), 500_000_000);
        assert_eq!(mean_interval_ns(2.0e9), 1);
    }

    #[test]
    fn burst_event_order_and_progress_boundaries_are_exact() {
        assert!(arrival_precedes_boundary(0.9, 1.0));
        assert!(!arrival_precedes_boundary(1.0, 1.0));
        assert!(!arrival_precedes_boundary(1.1, 1.0));

        assert_eq!(count_crossed_boundary(0).expect("first boundary"), 1);
        assert_eq!(
            count_crossed_boundary(MAX_BOUNDARIES_WITHOUT_ARRIVAL - 1)
                .expect("last permitted boundary"),
            MAX_BOUNDARIES_WITHOUT_ARRIVAL
        );
        assert!(matches!(
            count_crossed_boundary(MAX_BOUNDARIES_WITHOUT_ARRIVAL),
            Err(WorkloadError::NoProgress { after_boundaries })
                if after_boundaries == MAX_BOUNDARIES_WITHOUT_ARRIVAL + 1
        ));
    }

    #[test]
    fn seeded_generators_have_versioned_golden_prefixes() {
        let poisson = generate(&WorkloadConfig {
            classes: vec!["a".into(), "b".into()],
            lambda: 10.0,
            zipf_exponent: 1.0,
            count: 4,
            seed: 7,
        })
        .expect("valid poisson");
        let poisson_prefix: Vec<_> = poisson
            .as_slice()
            .iter()
            .map(|arrival| (arrival.send_time_secs.to_bits(), arrival.class.as_str()))
            .collect();
        assert_eq!(
            poisson_prefix,
            vec![
                (4_568_936_400_497_789_564, "a"),
                (4_590_247_640_846_338_216, "a"),
                (4_591_202_390_223_696_143, "a"),
                (4_597_518_169_406_491_358, "b"),
            ],
            "update only with an intentional GENERATOR_VERSION bump"
        );

        let burst = generate_bursty(&BurstConfig {
            classes: vec!["a".into(), "b".into()],
            zipf_exponent: 1.0,
            count: 4,
            seed: 7,
            low_lambda: 10.0,
            high_lambda: 100.0,
            mean_off_secs: 0.1,
            mean_on_secs: 0.02,
        })
        .expect("valid burst");
        let burst_prefix: Vec<_> = burst
            .as_slice()
            .iter()
            .map(|arrival| (arrival.send_time_secs.to_bits(), arrival.class.as_str()))
            .collect();
        assert_eq!(
            burst_prefix,
            vec![
                (4_587_824_009_267_638_135, "a"),
                (4_592_769_853_735_162_850, "b"),
                (4_598_458_304_146_356_875, "a"),
                (4_599_724_429_072_124_686, "a"),
            ],
            "update only with an intentional GENERATOR_VERSION bump"
        );
    }

    #[test]
    fn send_times_are_monotonic() {
        let arrivals = generate(&WorkloadConfig::default()).unwrap();
        assert!(arrivals
            .as_slice()
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
        let arrivals = generate(&cfg).unwrap();
        let hot = arrivals
            .as_slice()
            .iter()
            .filter(|a| a.class == "retrieval")
            .count();
        let cold = arrivals
            .as_slice()
            .iter()
            .filter(|a| a.class == "index")
            .count();
        assert!(hot > cold, "hot={hot} cold={cold}");
    }

    #[test]
    fn trace_csv_round_trips_exactly() {
        let arrivals = generate(&WorkloadConfig::default()).unwrap();
        let csv = trace_to_csv(arrivals.as_slice());
        let parsed = trace_from_csv(&csv).expect("valid trace");
        assert_eq!(parsed, arrivals, "trace must round-trip without loss");
    }

    #[test]
    fn trace_parser_ignores_comments_and_blanks_and_reports_errors() {
        let text = "# header\n\n0.5,retrieval\n  1.25 , rank \n";
        let parsed = trace_from_csv(text).expect("valid");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed.as_slice()[1].class, "rank");
        assert!(trace_from_csv("not_a_number,retrieval").is_err());
        assert!(trace_from_csv("0.5").is_err()); // missing comma
        assert!(trace_from_csv("0.5,").is_err()); // empty class
        assert!(trace_from_csv("# header\n\nmissing")
            .is_err_and(|error| { error.starts_with("line 3: missing comma") }));
        assert_eq!(human_line_number(0), 1);
        assert_eq!(human_line_number(41), 42);
    }

    // ---- TM16-027: a float that parses is not a valid event time ----------

    #[test]
    fn invalid_event_times_are_rejected_by_the_parser_with_the_line() {
        for (text, needle) in [
            ("NaN,retrieval", "not finite"),
            ("inf,retrieval", "not finite"),
            ("-inf,retrieval", "not finite"),
            ("-1,retrieval", "negative"),
            ("2,retrieval\n1,retrieval", "precedes"),
            ("1e300,retrieval", "representable range"),
        ] {
            let error = trace_from_csv(text).expect_err(text);
            assert!(
                error.contains(needle),
                "{text:?}: expected {needle:?} in {error:?}"
            );
            assert!(error.starts_with("line "), "{text:?}: {error}");
        }
    }

    #[test]
    fn invalid_event_times_are_rejected_at_the_direct_api_too() {
        // The parser is not the only door. A hand-built schedule reaches the
        // simulator through the same validation, so the CSV path cannot be
        // bypassed by constructing `Arrival`s directly.
        let arrival = |t: f64| Arrival {
            send_time_secs: t,
            class: "retrieval".into(),
        };
        // NaN never equals itself, so the NaN case matches on shape only.
        assert!(matches!(
            ValidatedArrivals::new(vec![arrival(f64::NAN)]),
            Err(WorkloadError::NonFiniteSendTime { index: 0, value }) if value.is_nan()
        ));
        assert!(matches!(
            ValidatedArrivals::new(vec![arrival(f64::INFINITY)]),
            Err(WorkloadError::NonFiniteSendTime { index: 0, .. })
        ));
        assert_eq!(
            ValidatedArrivals::new(vec![arrival(-1.0)]),
            Err(WorkloadError::NegativeSendTime {
                index: 0,
                value: -1.0
            })
        );
        assert_eq!(
            ValidatedArrivals::new(vec![arrival(2.0), arrival(1.0)]),
            Err(WorkloadError::NonMonotonicSendTime {
                index: 1,
                previous: 2.0,
                value: 1.0
            })
        );
        assert!(matches!(
            ValidatedArrivals::new(vec![arrival(MAX_SEND_TIME_SECS * 2.0)]),
            Err(WorkloadError::SendTimeOutOfRange { index: 0, .. })
        ));
        // Simultaneous arrivals are legitimate.
        let same = ValidatedArrivals::new(vec![arrival(1.0), arrival(1.0)]).expect("equal ok");
        assert_eq!(same.send_times_ns(), &[1_000_000_000, 1_000_000_000]);
    }

    #[test]
    fn nanosecond_conversion_is_exact_inside_the_range() {
        let arrival = |t: f64| Arrival {
            send_time_secs: t,
            class: "c".into(),
        };
        let schedule = ValidatedArrivals::new(vec![
            arrival(0.0),
            arrival(0.000_000_001),
            arrival(1.5),
            arrival(MAX_SEND_TIME_SECS),
        ])
        .expect("in range");
        assert_eq!(schedule.send_times_ns(), &[0, 1, 1_500_000_000, 1u64 << 53]);
    }

    // ---- TM16-028: the MMPP must be the MMPP it was configured as ---------

    #[test]
    fn bursty_arrivals_are_deterministic_monotonic_and_overdispersed() {
        let cfg = BurstConfig::default();
        let a = generate_bursty(&cfg).unwrap();
        assert_eq!(a.len(), cfg.count);
        assert_eq!(
            a,
            generate_bursty(&cfg).unwrap(),
            "MMPP must be seed-deterministic"
        );
        assert!(a
            .as_slice()
            .windows(2)
            .all(|w| w[1].send_time_secs >= w[0].send_time_secs));

        // MMPP is over-dispersed: its inter-arrival CV exceeds a pure Poisson
        // process of the same count (whose CV ~= 1.0).
        let poisson = generate(&WorkloadConfig {
            classes: cfg.classes.clone(),
            lambda: 10_000.0,
            count: cfg.count,
            ..Default::default()
        })
        .unwrap();
        let burst_cv = interarrival_cv(a.as_slice());
        let poisson_cv = interarrival_cv(poisson.as_slice());
        assert!(
            burst_cv > 1.3 && burst_cv > poisson_cv,
            "MMPP should be burstier: burst_cv={burst_cv:.3} poisson_cv={poisson_cv:.3}"
        );
    }

    /// Realized long-run rate of a schedule: count / span.
    fn realized_rate(schedule: &ValidatedArrivals) -> f64 {
        let slice = schedule.as_slice();
        let span = slice.last().map_or(0.0, |a| a.send_time_secs);
        slice.len() as f64 / span
    }

    #[test]
    fn fast_equal_dwell_switching_converges_to_the_stationary_rate() {
        // Off at 1/s, on at 1000/s, both phases 1ms on average. The CTMC's
        // stationary rate is (1 + 1000)/2 = 500.5/s. The old generator produced
        // ~2.6/s here because a single off-phase draw jumped over hundreds of
        // burst phases. Several seeds, statistical tolerance — one seed pinned
        // to a golden number would only re-encode whatever the generator does.
        let base = BurstConfig {
            classes: vec!["retrieval".into()],
            count: 20_000,
            low_lambda: 1.0,
            high_lambda: 1_000.0,
            mean_off_secs: 0.001,
            mean_on_secs: 0.001,
            ..Default::default()
        };
        let expected = base.stationary_rate();
        assert!((expected - 500.5).abs() < 1e-9);
        for seed in [1u64, 42, 7_777, 0xDEAD_BEEF] {
            let schedule = generate_bursty(&BurstConfig {
                seed,
                ..base.clone()
            })
            .unwrap();
            let rate = realized_rate(&schedule);
            let error = (rate - expected).abs() / expected;
            assert!(
                error < 0.08,
                "seed {seed}: realized {rate:.1}/s vs stationary {expected:.1}/s ({:.1}% off)",
                error * 100.0
            );
        }
    }

    #[test]
    fn equal_phase_rates_reduce_to_a_plain_poisson_process() {
        let cfg = BurstConfig {
            classes: vec!["c".into()],
            count: 20_000,
            low_lambda: 500.0,
            high_lambda: 500.0,
            mean_off_secs: 0.003,
            mean_on_secs: 0.001,
            ..Default::default()
        };
        let schedule = generate_bursty(&cfg).unwrap();
        let rate = realized_rate(&schedule);
        assert!((rate - 500.0).abs() / 500.0 < 0.05, "rate {rate}");
        let cv = interarrival_cv(schedule.as_slice());
        assert!((cv - 1.0).abs() < 0.1, "Poisson CV ~ 1, got {cv}");
    }

    #[test]
    fn asymmetric_dwells_weight_the_rate_by_occupancy() {
        // 90% of the time in the slow phase: the stationary rate is dominated by
        // the low rate even though the high rate is 100× larger.
        let cfg = BurstConfig {
            classes: vec!["c".into()],
            count: 30_000,
            low_lambda: 100.0,
            high_lambda: 10_000.0,
            mean_off_secs: 0.009,
            mean_on_secs: 0.001,
            ..Default::default()
        };
        let expected = cfg.stationary_rate(); // 0.9*100 + 0.1*10000 = 1090
        assert!((expected - 1_090.0).abs() < 1e-9);
        assert!((cfg.on_occupancy() - 0.1).abs() < 1e-12);
        let schedule = generate_bursty(&cfg).unwrap();
        let rate = realized_rate(&schedule);
        assert!(
            (rate - expected).abs() / expected < 0.1,
            "realized {rate:.1} vs {expected:.1}"
        );
    }

    #[test]
    fn degenerate_burst_configs_are_typed_errors_not_hangs() {
        let base = BurstConfig {
            count: 1,
            ..Default::default()
        };
        let cases: Vec<(&str, BurstConfig)> = vec![
            (
                "zero dwell",
                BurstConfig {
                    mean_off_secs: 0.0,
                    mean_on_secs: 0.0,
                    ..base.clone()
                },
            ),
            (
                "negative dwell",
                BurstConfig {
                    mean_on_secs: -1.0,
                    ..base.clone()
                },
            ),
            (
                "nan rate",
                BurstConfig {
                    low_lambda: f64::NAN,
                    ..base.clone()
                },
            ),
            (
                "inf rate",
                BurstConfig {
                    high_lambda: f64::INFINITY,
                    ..base.clone()
                },
            ),
            (
                "inverted rates",
                BurstConfig {
                    low_lambda: 10.0,
                    high_lambda: 1.0,
                    ..base.clone()
                },
            ),
            (
                "no classes",
                BurstConfig {
                    classes: vec![],
                    ..base.clone()
                },
            ),
        ];
        for (what, cfg) in cases {
            let started = std::time::Instant::now();
            let result = generate_bursty(&cfg);
            assert!(
                matches!(result, Err(WorkloadError::InvalidConfig { .. })),
                "{what}: expected a config error, got {result:?}"
            );
            assert!(
                started.elapsed() < std::time::Duration::from_secs(1),
                "{what}: validation must not enter the sampling loop"
            );
        }
        // Positive control: the same shape with valid dwells produces a request.
        assert_eq!(generate_bursty(&base).unwrap().len(), 1);
    }

    #[test]
    fn poisson_config_is_validated() {
        let bad = WorkloadConfig {
            lambda: 0.0,
            ..Default::default()
        };
        assert!(matches!(
            generate(&bad),
            Err(WorkloadError::InvalidConfig {
                field: "lambda",
                ..
            })
        ));
        let bad = WorkloadConfig {
            zipf_exponent: f64::NAN,
            ..Default::default()
        };
        assert!(matches!(
            generate(&bad),
            Err(WorkloadError::InvalidConfig {
                field: "zipf_exponent",
                ..
            })
        ));
    }

    #[test]
    fn bursty_trace_round_trips_and_replays() {
        let a = generate_bursty(&BurstConfig {
            count: 1_000,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(trace_from_csv(&trace_to_csv(a.as_slice())).unwrap(), a);
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
        let recorded = generate(&cfg).unwrap();
        let replayed = trace_from_csv(&trace_to_csv(recorded.as_slice())).expect("replay");

        let interval = mean_interval_ns(cfg.lambda);
        let service_ns = 32 * interval;
        let run = |arr: &ValidatedArrivals| {
            let fx = fixture(vec![("retrieval", retrieval_policy(32, 128))], 0, 0);
            simulate(&fx, arr, service_ns).unwrap().1
        };
        assert_eq!(run(&recorded), run(&replayed));
    }
}
