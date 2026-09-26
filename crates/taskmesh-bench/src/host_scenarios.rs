//! Strict, bench-owned public-host workload declaration.
//!
//! This schema is deliberately separate from product `TaskSpec`: body duration,
//! pacing, and measurement windows are benchmark concerns.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use taskmesh::{
    Builder, CancellationPolicy, ClassPolicy, FairnessPolicy, OverflowPolicy, PhysicalDomainMode,
    ResourceBudget, TaskClass, TokioRuntime, TopologyConfig,
};

use crate::workload::{Arrival, ValidatedArrivals};

pub const HOST_SCENARIO_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostScenario {
    pub schema_version: u32,
    pub id: String,
    pub load: HostLoadEnvelope,
    pub topology: HostTopology,
    pub classes: Vec<HostClass>,
    pub offers: Vec<HostOffer>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostLoadEnvelope {
    pub warmup_ms: u64,
    pub injection_ms: u64,
    pub interval_ms: u64,
    pub snapshot_ms: u64,
    pub settlement_ms: u64,
    pub max_outstanding: usize,
    pub max_records: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostTopology {
    pub cpu_workers: usize,
    pub blocking_threads: usize,
    pub shared_blocking_limit: usize,
    #[serde(default)]
    pub large_stack_slots: usize,
    pub cpu_units: u32,
    pub memory_units: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostClass {
    pub name: String,
    pub slo_ms: u64,
    pub max_inflight: u32,
    pub max_queue_depth: u32,
    pub cpu_units: u32,
    pub memory_units: u32,
    pub overflow: HostOverflow,
    #[serde(default)]
    pub fairness: HostFairness,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostFairness {
    #[default]
    Fifo,
    WeightedFair {
        weight: u32,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostOverflow {
    Reject,
    QueueWithinDepth,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostOffer {
    pub send_time_ns: u64,
    pub class: String,
    pub path: HostPath,
    pub body: HostBody,
    #[serde(default)]
    pub stack_size_bytes: Option<u64>,
    #[serde(default)]
    pub deadline_ms: Option<u64>,
    #[serde(default)]
    pub cancel_after_ms: Option<u64>,
    #[serde(default)]
    pub drop_after_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostPath {
    Io,
    Blocking,
    Cpu,
    RequestedStackBlocking,
    RequestedStackAsync,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostBody {
    Noop,
    AsyncSleep { millis: u64 },
    BlockingSleep { millis: u64 },
    CpuSpin { iterations: u64 },
}

impl HostScenario {
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let scenario: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        scenario.validate()?;
        Ok(scenario)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != HOST_SCENARIO_VERSION {
            return Err(format!(
                "unsupported host scenario version {}",
                self.schema_version
            ));
        }
        if self.id.is_empty() || self.id.len() > 128 {
            return Err("scenario id must have 1..=128 bytes".into());
        }
        let load = &self.load;
        if load.injection_ms == 0 || load.interval_ms == 0 || load.settlement_ms == 0 {
            return Err("injection_ms, interval_ms and settlement_ms must be nonzero".into());
        }
        if load.injection_ms % load.interval_ms != 0 {
            return Err("interval_ms must divide injection_ms".into());
        }
        if load.injection_ms / load.interval_ms > 10_000 {
            return Err("scenario may emit at most 10000 report intervals".into());
        }
        if load.snapshot_ms != 0
            && (load.injection_ms % load.snapshot_ms != 0
                || load.injection_ms / load.snapshot_ms > 10_000)
        {
            return Err("snapshot_ms must divide injection_ms with at most 10000 samples".into());
        }
        if load.max_outstanding == 0 || load.max_records == 0 {
            return Err("max_outstanding and max_records must be nonzero".into());
        }
        if load.max_records > 1_000_000 || load.max_outstanding > load.max_records {
            return Err("record/outstanding bounds are invalid".into());
        }
        if [load.warmup_ms, load.injection_ms, load.settlement_ms]
            .into_iter()
            .any(|millis| millis > 3_600_000)
        {
            return Err("each run phase must be at most one hour".into());
        }
        if self.offers.is_empty() || self.offers.len() > load.max_records {
            return Err("offers must be nonempty and within max_records".into());
        }
        if self.topology.cpu_workers == 0
            || self.topology.blocking_threads == 0
            || self.topology.shared_blocking_limit == 0
        {
            return Err("fixed physical topology capacities must be nonzero".into());
        }
        if self.offers.iter().any(|offer| {
            matches!(
                offer.path,
                HostPath::RequestedStackBlocking | HostPath::RequestedStackAsync
            )
        }) && self.topology.large_stack_slots == 0
        {
            return Err("requested-stack offers require a finite large_stack_slots limit".into());
        }
        if self.classes.is_empty() {
            return Err("at least one registered class is required".into());
        }
        let mut names = BTreeSet::new();
        for class in &self.classes {
            TaskClass::new(class.name.clone())
                .validate()
                .map_err(|error| format!("invalid class {:?}: {error}", class.name))?;
            if !names.insert(class.name.as_str()) {
                return Err(format!("duplicate class {:?}", class.name));
            }
            if class.slo_ms == 0 || class.slo_ms > load.settlement_ms {
                return Err(format!(
                    "class {:?}: SLO must fit settlement window",
                    class.name
                ));
            }
            if matches!(class.fairness, HostFairness::WeightedFair { weight: 0 }) {
                return Err(format!(
                    "class {:?}: WFQ weight must be positive",
                    class.name
                ));
            }
        }
        let last_allowed = load
            .injection_ms
            .checked_mul(1_000_000)
            .ok_or("injection window does not fit nanoseconds")?;
        let mut arrivals = Vec::with_capacity(self.offers.len());
        let mut previous_ns = None;
        for (index, offer) in self.offers.iter().enumerate() {
            if !names.contains(offer.class.as_str()) {
                return Err(format!("offer {index}: unknown class {:?}", offer.class));
            }
            if offer.send_time_ns >= last_allowed {
                return Err(format!("offer {index}: outside injection window"));
            }
            if previous_ns.is_some_and(|previous| offer.send_time_ns < previous) {
                return Err(format!("offer {index}: intended times are not ordered"));
            }
            previous_ns = Some(offer.send_time_ns);
            if offer.cancel_after_ms.is_some() && offer.drop_after_ms.is_some() {
                return Err(format!("offer {index}: cancel and drop conflict"));
            }
            let requested_stack = matches!(
                offer.path,
                HostPath::RequestedStackBlocking | HostPath::RequestedStackAsync
            );
            match (requested_stack, offer.stack_size_bytes) {
                (true, Some(bytes))
                    if bytes > 0 && bytes <= 1 << 34 && bytes <= usize::MAX as u64 => {}
                (true, _) => return Err(format!("offer {index}: invalid requested stack size")),
                (false, Some(_)) => {
                    return Err(format!("offer {index}: stack size on non-stack path"))
                }
                (false, None) => {}
            }
            if [
                offer.deadline_ms,
                offer.cancel_after_ms,
                offer.drop_after_ms,
            ]
            .into_iter()
            .flatten()
            .any(|millis| millis > 3_600_000)
            {
                return Err(format!("offer {index}: timer exceeds one hour"));
            }
            let body_matches = matches!(
                (offer.path, &offer.body),
                (HostPath::Io, HostBody::Noop | HostBody::AsyncSleep { .. })
                    | (
                        HostPath::RequestedStackAsync,
                        HostBody::Noop | HostBody::AsyncSleep { .. }
                    )
                    | (
                        HostPath::Blocking,
                        HostBody::Noop | HostBody::BlockingSleep { .. }
                    )
                    | (
                        HostPath::RequestedStackBlocking,
                        HostBody::Noop | HostBody::BlockingSleep { .. }
                    )
                    | (HostPath::Cpu, HostBody::Noop | HostBody::CpuSpin { .. })
            );
            if !body_matches {
                return Err(format!("offer {index}: path/body mismatch"));
            }
            match &offer.body {
                HostBody::AsyncSleep { millis } | HostBody::BlockingSleep { millis }
                    if *millis > load.settlement_ms =>
                {
                    return Err(format!("offer {index}: body exceeds settlement window"));
                }
                HostBody::CpuSpin { iterations } if *iterations > 100_000_000 => {
                    return Err(format!("offer {index}: cpu work exceeds fixture bound"));
                }
                _ => {}
            }
            arrivals.push(Arrival {
                send_time_secs: offer.send_time_ns as f64 / 1e9,
                class: offer.class.clone(),
            });
        }
        let validated = ValidatedArrivals::new(arrivals).map_err(|error| error.to_string())?;
        if validated
            .send_times_ns()
            .iter()
            .zip(&self.offers)
            .any(|(converted, offer)| *converted != offer.send_time_ns)
        {
            return Err("intended nanoseconds cannot round-trip through simulator schedule".into());
        }
        self.topology_config()
            .validate()
            .map_err(|error| error.to_string())
    }

    fn topology_config(&self) -> TopologyConfig {
        TopologyConfig::new()
            .cpu_fixed(self.topology.cpu_workers)
            .blocking_threads(self.topology.blocking_threads)
            .large_stack_slots(self.topology.large_stack_slots)
            .shared_blocking_domain(PhysicalDomainMode::Fixed(
                self.topology.shared_blocking_limit,
            ))
    }

    pub fn resolved_topology(&self) -> Result<crate::host_load::ResolvedHostTopology, String> {
        self.validate()?;
        Ok(crate::host_load::ResolvedHostTopology::from_runtime(
            &self.build_runtime()?,
        ))
    }

    pub fn build_runtime(&self) -> Result<TokioRuntime, String> {
        let topology = self.topology_config();
        let resources = ResourceBudget::new()
            .cpu_units(self.topology.cpu_units)
            .memory_units(self.topology.memory_units);
        let mut builder = Builder::new().topology(topology).resources(resources);
        for class in &self.classes {
            let overflow = match class.overflow {
                HostOverflow::Reject => OverflowPolicy::Reject,
                HostOverflow::QueueWithinDepth => OverflowPolicy::QueueWithinDepth,
            };
            let fairness = match class.fairness {
                HostFairness::Fifo => FairnessPolicy::Fifo,
                HostFairness::WeightedFair { weight } => {
                    FairnessPolicy::WeightedFairQueue { weight, burst: 0 }
                }
            };
            builder = builder.class_policy(
                TaskClass::new(class.name.clone()),
                ClassPolicy::new()
                    .max_inflight(class.max_inflight)
                    .max_queue_depth(class.max_queue_depth)
                    .cpu_units(class.cpu_units)
                    .memory_units(class.memory_units)
                    .overflow_policy(overflow)
                    .fairness(fairness)
                    .cancellation_policy(CancellationPolicy::CooperativeWithDeadline),
            );
        }
        builder.build().map_err(|error| error.to_string())
    }
}
