//! H6 caller-orchestrated fan-out over separately governed public submissions.
//!
//! Taskmesh validates the reduce declaration and governs each child. The
//! caller awaits all children and performs the keyed reduction itself.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use taskmesh::{
    DeterministicReducePolicy, RunError, Runtime, SubstrateHint, TaskClass, TaskSpec, TaskStage,
};

use crate::host_load::{
    classify_response, since, warmup_runtime, ClassCounters, ResolvedHostTopology, ResponseOutcome,
};
use crate::host_scenarios::{
    HostBody, HostClass, HostLoadEnvelope, HostOffer, HostPath, HostScenario, HostTopology,
};

pub const COMPOSITE_HOST_VERSION: u32 = 1;
const UNSET: u64 = u64::MAX;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompositeScenario {
    pub schema_version: u32,
    pub id: String,
    pub warmup_ms: u64,
    pub settlement_ms: u64,
    #[serde(default)]
    pub parent_timeout_ms: Option<u64>,
    pub topology: HostTopology,
    pub classes: Vec<HostClass>,
    pub parent_class: String,
    pub child_class: String,
    pub io_delay_ms: u64,
    pub blocking_delay_ms: u64,
    pub cpu_iterations: u64,
    pub fail_child_key: Option<u8>,
}

impl CompositeScenario {
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let scenario: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        scenario.validate()?;
        Ok(scenario)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != COMPOSITE_HOST_VERSION
            || self.parent_class == self.child_class
            || self
                .fail_child_key
                .is_some_and(|key| !(1..=3).contains(&key))
        {
            return Err("invalid composite version, class split or failure key".into());
        }
        if self
            .parent_timeout_ms
            .is_some_and(|millis| millis == 0 || millis > self.settlement_ms)
        {
            return Err("parent timeout must fit settlement window".into());
        }
        self.host_shape().validate()
    }

    pub fn resolved_topology(&self) -> Result<ResolvedHostTopology, String> {
        self.validate()?;
        Ok(ResolvedHostTopology::from_runtime(
            &self.host_shape().build_runtime()?,
        ))
    }

    fn host_shape(&self) -> HostScenario {
        let offers = [
            (self.parent_class.clone(), HostPath::Io, HostBody::Noop),
            (
                self.child_class.clone(),
                HostPath::Io,
                HostBody::AsyncSleep {
                    millis: self.io_delay_ms,
                },
            ),
            (
                self.child_class.clone(),
                HostPath::Blocking,
                HostBody::BlockingSleep {
                    millis: self.blocking_delay_ms,
                },
            ),
            (
                self.child_class.clone(),
                HostPath::Cpu,
                HostBody::CpuSpin {
                    iterations: self.cpu_iterations,
                },
            ),
        ]
        .into_iter()
        .map(|(class, path, body)| HostOffer {
            send_time_ns: 0,
            class,
            path,
            body,
            stack_size_bytes: None,
            deadline_ms: None,
            cancel_after_ms: None,
            drop_after_ms: None,
        })
        .collect();
        HostScenario {
            schema_version: 1,
            id: self.id.clone(),
            load: HostLoadEnvelope {
                warmup_ms: self.warmup_ms,
                injection_ms: 1,
                interval_ms: 1,
                snapshot_ms: 0,
                settlement_ms: self.settlement_ms,
                max_outstanding: 4,
                max_records: 4,
            },
            topology: self.topology.clone(),
            classes: self.classes.clone(),
            offers,
        }
    }
}

struct Markers {
    submitted: AtomicU64,
    started: AtomicU64,
    finished: AtomicU64,
    responded: AtomicU64,
}

impl Markers {
    fn new() -> Self {
        Self {
            submitted: AtomicU64::new(UNSET),
            started: AtomicU64::new(UNSET),
            finished: AtomicU64::new(UNSET),
            responded: AtomicU64::new(UNSET),
        }
    }

    fn pair(&self) -> (Option<u64>, Option<u64>) {
        let value = |atomic: &AtomicU64| match atomic.load(Ordering::Acquire) {
            UNSET => None,
            nanos => Some(nanos),
        };
        (value(&self.started), value(&self.finished))
    }

    fn observation(&self, key: u8, path: HostPath) -> CompositePartialChild {
        let value = |atomic: &AtomicU64| match atomic.load(Ordering::Acquire) {
            UNSET => None,
            nanos => Some(nanos),
        };
        CompositePartialChild {
            key,
            path,
            submitted_ns: value(&self.submitted),
            body_started_ns: value(&self.started),
            body_finished_ns: value(&self.finished),
            response_ns: value(&self.responded),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompositePartialChild {
    pub key: u8,
    pub path: HostPath,
    pub submitted_ns: Option<u64>,
    pub body_started_ns: Option<u64>,
    pub body_finished_ns: Option<u64>,
    pub response_ns: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompositeFailureRaw {
    pub schema_version: u32,
    pub status: String,
    pub mode: String,
    pub scenario_id: String,
    pub reason: String,
    pub parent_submitted_ns: u64,
    pub observed_ns: u64,
    pub children: Vec<CompositePartialChild>,
    pub root_attribution_cleared: bool,
    pub class_counters: BTreeMap<String, ClassCounters>,
    pub final_capabilities: BTreeMap<String, u32>,
    pub drain_ok: bool,
    pub conservation_ok: bool,
}

impl CompositeFailureRaw {
    pub fn validate_against(&self, scenario: &CompositeScenario) -> Result<(), String> {
        scenario.validate()?;
        if self.schema_version != COMPOSITE_HOST_VERSION
            || self.status != "invalid"
            || self.mode != "caller_orchestrated_composite"
            || self.scenario_id != scenario.id
            || self.reason.is_empty()
            || self.parent_submitted_ns > self.observed_ns
            || self.children.len() != 3
        {
            return Err("invalid composite failure identity or population".into());
        }
        for (index, (row, path)) in self
            .children
            .iter()
            .zip([HostPath::Io, HostPath::Blocking, HostPath::Cpu])
            .enumerate()
        {
            if row.key != (index + 1) as u8 || row.path != path {
                return Err(format!(
                    "composite failure child {index}: key or path differs"
                ));
            }
            let timestamps = [
                row.submitted_ns,
                row.body_started_ns,
                row.body_finished_ns,
                row.response_ns,
            ];
            let mut previous = Some(self.parent_submitted_ns);
            for timestamp in timestamps.into_iter().flatten() {
                if previous.is_some_and(|earlier| timestamp < earlier)
                    || timestamp > self.observed_ns
                {
                    return Err(format!("composite failure child {index}: time differs"));
                }
                previous = Some(timestamp);
            }
            if row.submitted_ns.is_none()
                && (row.body_started_ns.is_some()
                    || row.body_finished_ns.is_some()
                    || row.response_ns.is_some())
                || row.body_started_ns.is_none() && row.body_finished_ns.is_some()
            {
                return Err(format!(
                    "composite failure child {index}: event order differs"
                ));
            }
        }
        if self.class_counters.len() != scenario.classes.len() || self.final_capabilities.is_empty()
        {
            return Err("composite failure governance inventory differs".into());
        }
        for class in &scenario.classes {
            let counters = self
                .class_counters
                .get(&class.name)
                .ok_or_else(|| format!("composite failure class {} missing", class.name))?;
            let maximum = if class.name == scenario.parent_class {
                1
            } else if class.name == scenario.child_class {
                3
            } else {
                0
            };
            if counters.admitted > maximum
                || counters.started > counters.admitted
                || counters.terminated > counters.admitted
            {
                return Err(format!(
                    "composite failure class {} counters differ",
                    class.name
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct CompositeRunFailure {
    pub reason: String,
    pub raw: Option<CompositeFailureRaw>,
    pub topology: Option<ResolvedHostTopology>,
}

impl CompositeRunFailure {
    fn early(reason: String) -> Self {
        Self {
            reason,
            raw: None,
            topology: None,
        }
    }
}

impl fmt::Display for CompositeRunFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.reason.fmt(formatter)
    }
}

impl Error for CompositeRunFailure {}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompositeChildRow {
    pub key: u8,
    pub path: HostPath,
    pub submitted_ns: u64,
    pub body_started_ns: Option<u64>,
    pub body_finished_ns: Option<u64>,
    pub response_ns: u64,
    pub outcome: ResponseOutcome,
    pub value: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompositeRaw {
    pub schema_version: u32,
    pub mode: String,
    pub scenario_id: String,
    pub parent_submitted_ns: u64,
    pub parent_response_ns: u64,
    pub children: Vec<CompositeChildRow>,
    pub reduce_started_ns: u64,
    pub reduce_finished_ns: u64,
    pub reduced_keys: Vec<u8>,
    pub checksum: u64,
    pub root_attribution_cleared: bool,
    pub class_counters: BTreeMap<String, ClassCounters>,
    pub final_capabilities: BTreeMap<String, u32>,
    pub drain_ok: bool,
    pub conservation_ok: bool,
}

impl CompositeRaw {
    pub fn validate_against(&self, scenario: &CompositeScenario) -> Result<(), String> {
        scenario.validate()?;
        if self.schema_version != COMPOSITE_HOST_VERSION
            || self.mode != "caller_orchestrated_composite"
            || self.scenario_id != scenario.id
            || self.children.len() != 3
            || self.parent_submitted_ns > self.parent_response_ns
            || self.reduce_started_ns > self.reduce_finished_ns
            || self.reduce_finished_ns > self.parent_response_ns
        {
            return Err("composite raw identity, population or parent time differs".into());
        }
        let paths = [HostPath::Io, HostPath::Blocking, HostPath::Cpu];
        let mut expected_keys = Vec::new();
        let mut checksum = 0_u64;
        for (index, row) in self.children.iter().enumerate() {
            let key = (index + 1) as u8;
            if row.key != key
                || row.path != paths[index]
                || row.submitted_ns < self.parent_submitted_ns
                || row.response_ns > self.reduce_started_ns
                || row.body_started_ns.is_none()
                || row.body_finished_ns.is_none()
                || row.body_started_ns.unwrap() < row.submitted_ns
                || row.body_finished_ns.unwrap() < row.body_started_ns.unwrap()
                || row.response_ns < row.body_finished_ns.unwrap()
            {
                return Err(format!(
                    "composite child {key}: invalid attribution or time"
                ));
            }
            if scenario.fail_child_key == Some(key) {
                if !matches!(row.outcome, ResponseOutcome::TaskError) || row.value.is_some() {
                    return Err(format!("composite child {key}: failure outcome differs"));
                }
            } else if !matches!(row.outcome, ResponseOutcome::Success)
                || row.value != Some(key as u64)
            {
                return Err(format!("composite child {key}: success outcome differs"));
            } else {
                expected_keys.push(key);
                checksum += key as u64;
            }
        }
        if self.reduced_keys != expected_keys || self.checksum != checksum {
            return Err("caller-owned keyed reduction differs".into());
        }
        if !self.root_attribution_cleared
            || !self.drain_ok
            || !self.conservation_ok
            || self.final_capabilities.is_empty()
            || self.final_capabilities.values().any(|held| *held != 0)
            || self.class_counters.len() != scenario.classes.len()
        {
            return Err("composite governance did not settle".into());
        }
        for class in &scenario.classes {
            let counters = self
                .class_counters
                .get(&class.name)
                .ok_or_else(|| format!("composite class {} missing", class.name))?;
            let expected = if class.name == scenario.parent_class {
                1
            } else if class.name == scenario.child_class {
                3
            } else {
                0
            };
            if counters.admitted != expected
                || counters.started != expected
                || counters.terminated != expected
                || counters.inflight != 0
                || counters.queued != 0
            {
                return Err(format!("composite class {} counters differ", class.name));
            }
        }
        Ok(())
    }
}

fn child_result(
    key: u8,
    path: HostPath,
    submitted_ns: u64,
    response_ns: u64,
    markers: &Markers,
    result: Result<u64, RunError<()>>,
) -> CompositeChildRow {
    markers.responded.store(response_ns, Ordering::Release);
    let (outcome, value) = match result {
        Ok(value) => (ResponseOutcome::Success, Some(value)),
        Err(error) => (classify_response(Err(error)), None),
    };
    let (body_started_ns, body_finished_ns) = markers.pair();
    CompositeChildRow {
        key,
        path,
        submitted_ns,
        body_started_ns,
        body_finished_ns,
        response_ns,
        outcome,
        value,
    }
}

struct ParentResult {
    children: Vec<CompositeChildRow>,
    reduce_started_ns: u64,
    reduce_finished_ns: u64,
    reduced_keys: Vec<u8>,
    checksum: u64,
}

pub async fn run_composite_with_topology(
    scenario: &CompositeScenario,
) -> Result<(CompositeRaw, ResolvedHostTopology), CompositeRunFailure> {
    scenario.validate().map_err(CompositeRunFailure::early)?;
    let host_shape = scenario.host_shape();
    let runtime = host_shape
        .build_runtime()
        .map_err(CompositeRunFailure::early)?;
    let topology = ResolvedHostTopology::from_runtime(&runtime);
    warmup_runtime(&host_shape, &runtime)
        .await
        .map_err(CompositeRunFailure::early)?;
    let baseline = runtime.snapshot();
    if baseline.conservation_violation().is_some()
        || baseline
            .classes
            .values()
            .any(|class| class.inflight != 0 || class.queued != 0)
    {
        return Err(CompositeRunFailure::early(
            "composite warmup did not settle".into(),
        ));
    }
    let origin = Instant::now();
    let parent_submitted_ns = since(origin);
    let root_id = format!("h6-{}", scenario.id);
    let parent_class = TaskClass::new(scenario.parent_class.clone());
    let child_class = TaskClass::new(scenario.child_class.clone());
    let parent_spec = TaskSpec::io(parent_class)
        .operation(root_id.clone())
        .reduce_stage(
            TaskStage::new("reduce"),
            SubstrateHint::AsyncIo,
            DeterministicReducePolicy::keyed("child_key"),
        );
    let io_runtime = runtime.clone();
    let blocking_runtime = runtime.clone();
    let cpu_runtime = runtime.clone();
    let io_class = child_class.clone();
    let blocking_class = child_class.clone();
    let io_root = root_id.clone();
    let blocking_root = root_id.clone();
    let cpu_root = root_id.clone();
    let io_delay = scenario.io_delay_ms;
    let blocking_delay = scenario.blocking_delay_ms;
    let cpu_iterations = scenario.cpu_iterations;
    let fail_key = scenario.fail_child_key;
    let io_markers = Arc::new(Markers::new());
    let blocking_markers = Arc::new(Markers::new());
    let cpu_markers = Arc::new(Markers::new());
    let retained_markers = [
        Arc::clone(&io_markers),
        Arc::clone(&blocking_markers),
        Arc::clone(&cpu_markers),
    ];
    let parent = runtime.run_io(parent_spec, async move {
        let io = async {
            let markers = io_markers;
            let body_markers = Arc::clone(&markers);
            let submitted = since(origin);
            markers.submitted.store(submitted, Ordering::Release);
            let result = io_runtime
                .run_io(
                    TaskSpec::io(io_class)
                        .child_of(&io_root, &io_root, TaskStage::new("io"))
                        .operation("h6-io"),
                    async move {
                        body_markers.started.store(since(origin), Ordering::Release);
                        tokio::time::sleep(Duration::from_millis(io_delay)).await;
                        body_markers
                            .finished
                            .store(since(origin), Ordering::Release);
                        if fail_key == Some(1) {
                            Err(())
                        } else {
                            Ok(1_u64)
                        }
                    },
                )
                .await;
            child_result(1, HostPath::Io, submitted, since(origin), &markers, result)
        };
        let blocking = async {
            let markers = blocking_markers;
            let body_markers = Arc::clone(&markers);
            let submitted = since(origin);
            markers.submitted.store(submitted, Ordering::Release);
            let result = blocking_runtime
                .run_blocking(
                    TaskSpec::blocking(blocking_class)
                        .child_of(&blocking_root, &blocking_root, TaskStage::new("blocking"))
                        .operation("h6-blocking"),
                    move || {
                        body_markers.started.store(since(origin), Ordering::Release);
                        std::thread::sleep(Duration::from_millis(blocking_delay));
                        body_markers
                            .finished
                            .store(since(origin), Ordering::Release);
                        if fail_key == Some(2) {
                            Err(())
                        } else {
                            Ok(2_u64)
                        }
                    },
                )
                .await;
            child_result(
                2,
                HostPath::Blocking,
                submitted,
                since(origin),
                &markers,
                result,
            )
        };
        let cpu = async {
            let markers = cpu_markers;
            let body_markers = Arc::clone(&markers);
            let submitted = since(origin);
            markers.submitted.store(submitted, Ordering::Release);
            let result = cpu_runtime
                .run_cpu(
                    TaskSpec::cpu(child_class)
                        .child_of(&cpu_root, &cpu_root, TaskStage::new("cpu"))
                        .operation("h6-cpu"),
                    move || {
                        body_markers.started.store(since(origin), Ordering::Release);
                        let mut value = 0_u64;
                        for index in 0..cpu_iterations {
                            value = value.wrapping_add(black_box(index).rotate_left(7));
                        }
                        black_box(value);
                        body_markers
                            .finished
                            .store(since(origin), Ordering::Release);
                        if fail_key == Some(3) {
                            Err(())
                        } else {
                            Ok(3_u64)
                        }
                    },
                )
                .await;
            child_result(3, HostPath::Cpu, submitted, since(origin), &markers, result)
        };
        let (io, blocking, cpu) = tokio::join!(io, blocking, cpu);
        let children = vec![io, blocking, cpu];
        let reduce_started_ns = since(origin);
        let keyed: BTreeMap<_, _> = children
            .iter()
            .filter_map(|row| row.value.map(|value| (row.key, value)))
            .collect();
        let reduced_keys = keyed.keys().copied().collect();
        let checksum = keyed.values().copied().sum();
        let reduce_finished_ns = since(origin);
        Ok::<_, ()>(ParentResult {
            children,
            reduce_started_ns,
            reduce_finished_ns,
            reduced_keys,
            checksum,
        })
    });
    let parent_result = tokio::time::timeout(
        Duration::from_millis(scenario.parent_timeout_ms.unwrap_or(scenario.settlement_ms)),
        parent,
    )
    .await;
    let parent_response_ns = since(origin);
    let drain_ok = runtime
        .drain(Duration::from_millis(scenario.settlement_ms))
        .await
        .is_ok();
    let final_snapshot = runtime.snapshot();
    let root_attribution_cleared = runtime.governor().root_attribution(&root_id).is_none();
    let class_counters = scenario
        .classes
        .iter()
        .map(|class| {
            let key = TaskClass::new(class.name.clone());
            let before = &baseline.classes[&key];
            let after = &final_snapshot.classes[&key];
            (
                class.name.clone(),
                ClassCounters {
                    admitted: after.admitted_total - before.admitted_total,
                    started: after.started_total - before.started_total,
                    terminated: after.terminated_total - before.terminated_total,
                    inflight: after.inflight,
                    queued: after.queued,
                },
            )
        })
        .collect();
    let result = match parent_result {
        Ok(Ok(result)) => result,
        result => {
            let reason = match result {
                Err(_) => "composite parent exceeded settlement bound".to_string(),
                Ok(Err(error)) => format!("composite parent failed: {error:?}"),
                Ok(Ok(_)) => unreachable!(),
            };
            let children = retained_markers
                .iter()
                .zip([HostPath::Io, HostPath::Blocking, HostPath::Cpu])
                .enumerate()
                .map(|(index, (markers, path))| markers.observation((index + 1) as u8, path))
                .collect();
            let raw = CompositeFailureRaw {
                schema_version: COMPOSITE_HOST_VERSION,
                status: "invalid".into(),
                mode: "caller_orchestrated_composite".into(),
                scenario_id: scenario.id.clone(),
                reason: reason.clone(),
                parent_submitted_ns,
                observed_ns: since(origin),
                children,
                root_attribution_cleared,
                class_counters,
                final_capabilities: final_snapshot
                    .capabilities
                    .iter()
                    .map(|(pool, usage)| (pool.clone(), usage.in_use))
                    .collect(),
                drain_ok,
                conservation_ok: final_snapshot.conservation_violation().is_none(),
            };
            return Err(CompositeRunFailure {
                reason,
                raw: Some(raw),
                topology: Some(topology),
            });
        }
    };
    Ok((
        CompositeRaw {
            schema_version: COMPOSITE_HOST_VERSION,
            mode: "caller_orchestrated_composite".into(),
            scenario_id: scenario.id.clone(),
            parent_submitted_ns,
            parent_response_ns,
            children: result.children,
            reduce_started_ns: result.reduce_started_ns,
            reduce_finished_ns: result.reduce_finished_ns,
            reduced_keys: result.reduced_keys,
            checksum: result.checksum,
            root_attribution_cleared,
            class_counters,
            final_capabilities: final_snapshot
                .capabilities
                .iter()
                .map(|(pool, usage)| (pool.clone(), usage.in_use))
                .collect(),
            drain_ok,
            conservation_ok: final_snapshot.conservation_violation().is_none(),
        },
        topology,
    ))
}
