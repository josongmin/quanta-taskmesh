//! Fixed caller concurrency for completion-capacity diagnostics.
//!
//! Each slot submits its next call only after the previous caller response.
//! There is no exogenous intended-arrival schedule, so this mode cannot supply
//! open-loop overload tails or intended-arrival SLO-goodput.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use taskmesh::{Runtime, TaskClass};

use crate::host_load::{
    classify_response, execute, since, warmup_runtime, ClassCounters, ResolvedHostTopology,
    ResponseOutcome,
};
use crate::host_scenarios::{
    HostBody, HostClass, HostLoadEnvelope, HostOffer, HostPath, HostScenario, HostTopology,
};

pub const CLOSED_LOOP_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClosedLoopScenario {
    pub schema_version: u32,
    pub id: String,
    pub warmup_ms: u64,
    pub max_run_ms: u64,
    pub settlement_ms: u64,
    pub concurrency: usize,
    pub iterations_per_slot: usize,
    pub topology: HostTopology,
    pub classes: Vec<HostClass>,
    pub template: ClosedLoopTemplate,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClosedLoopTemplate {
    pub class: String,
    pub path: HostPath,
    pub body: HostBody,
    #[serde(default)]
    pub stack_size_bytes: Option<u64>,
}

impl ClosedLoopScenario {
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let scenario: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        scenario.validate()?;
        Ok(scenario)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != CLOSED_LOOP_VERSION {
            return Err("unsupported closed-loop scenario version".into());
        }
        let total = self
            .concurrency
            .checked_mul(self.iterations_per_slot)
            .ok_or("closed-loop request population overflow")?;
        if self.concurrency == 0
            || self.iterations_per_slot == 0
            || total > 1_000_000
            || self.concurrency > 10_000
            || self.max_run_ms == 0
            || self.max_run_ms > 3_600_000
        {
            return Err("closed-loop caller slots or request population are out of bounds".into());
        }
        self.host_shape().validate()
    }

    pub fn resolved_topology(&self) -> Result<ResolvedHostTopology, String> {
        self.validate()?;
        Ok(ResolvedHostTopology::from_runtime(
            &self.host_shape().build_runtime()?,
        ))
    }

    fn offer(&self) -> HostOffer {
        HostOffer {
            send_time_ns: 0,
            class: self.template.class.clone(),
            path: self.template.path,
            body: self.template.body.clone(),
            stack_size_bytes: self.template.stack_size_bytes,
            deadline_ms: None,
            cancel_after_ms: None,
            drop_after_ms: None,
        }
    }

    fn host_shape(&self) -> HostScenario {
        HostScenario {
            schema_version: 1,
            id: self.id.clone(),
            load: HostLoadEnvelope {
                warmup_ms: self.warmup_ms,
                injection_ms: 1,
                interval_ms: 1,
                snapshot_ms: 0,
                settlement_ms: self.settlement_ms,
                max_outstanding: self.concurrency,
                max_records: self.concurrency,
            },
            topology: self.topology.clone(),
            classes: self.classes.clone(),
            offers: vec![self.offer()],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClosedLoopRecord {
    pub id: usize,
    pub caller_slot: usize,
    pub iteration: usize,
    pub submitted_ns: u64,
    pub response_ns: u64,
    pub outcome: ResponseOutcome,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClosedLoopRun {
    pub schema_version: u32,
    pub mode: String,
    pub scenario_id: String,
    pub concurrency: usize,
    pub iterations_per_slot: usize,
    pub elapsed_ns: u64,
    pub records: Vec<Option<ClosedLoopRecord>>,
    pub class_counters: BTreeMap<String, ClassCounters>,
    pub final_capabilities: BTreeMap<String, u32>,
    pub drain_ok: bool,
    pub conservation_ok: bool,
    pub invalid_reason: Option<String>,
}

impl ClosedLoopRun {
    pub fn validate_against(&self, scenario: &ClosedLoopScenario) -> Result<(), String> {
        scenario.validate()?;
        scenario
            .resolved_topology()?
            .validate_capability_usage(&self.final_capabilities, "closed-loop final inventory")?;
        let total = scenario.concurrency * scenario.iterations_per_slot;
        if self.schema_version != CLOSED_LOOP_VERSION
            || self.mode != "closed_loop_fixed_concurrency"
            || self.scenario_id != scenario.id
            || self.concurrency != scenario.concurrency
            || self.iterations_per_slot != scenario.iterations_per_slot
            || self.records.len() != total
            || self.elapsed_ns == 0
            || self.elapsed_ns > scenario.max_run_ms * 1_000_000
        {
            return Err("closed-loop raw identity or population differs".into());
        }
        if let Some(reason) = &self.invalid_reason {
            return Err(format!("invalid closed-loop run: {reason}"));
        }
        let mut prior_response = vec![None; scenario.concurrency];
        let mut successes = 0_u128;
        for (id, record) in self.records.iter().enumerate() {
            let row = record
                .as_ref()
                .ok_or_else(|| format!("closed-loop row {id}: missing caller terminal"))?;
            let slot = id / scenario.iterations_per_slot;
            let iteration = id % scenario.iterations_per_slot;
            if row.id != id
                || row.caller_slot != slot
                || row.iteration != iteration
                || row.submitted_ns > row.response_ns
                || row.response_ns > self.elapsed_ns
                || prior_response[slot].is_some_and(|previous| row.submitted_ns < previous)
            {
                return Err(format!("closed-loop row {id}: invalid caller sequence"));
            }
            match &row.outcome {
                ResponseOutcome::Success => successes += 1,
                ResponseOutcome::Rejected { .. } => {}
                _ => return Err(format!("closed-loop row {id}: unexpected caller outcome")),
            }
            prior_response[slot] = Some(row.response_ns);
        }
        if !self.drain_ok
            || !self.conservation_ok
            || self.class_counters.values().any(|class| {
                class.inflight != 0 || class.queued != 0 || class.admitted != class.terminated
            })
            || self.final_capabilities.is_empty()
            || self.final_capabilities.values().any(|held| *held != 0)
        {
            return Err("closed-loop run did not settle owned capacity".into());
        }
        if self.class_counters.len() != scenario.classes.len()
            || scenario
                .classes
                .iter()
                .any(|class| !self.class_counters.contains_key(&class.name))
        {
            return Err("closed-loop class catalog differs".into());
        }
        let observed_successes = self
            .class_counters
            .values()
            .map(|class| class.started)
            .sum::<u128>();
        if observed_successes != successes
            || self
                .class_counters
                .values()
                .any(|class| class.admitted != class.started)
        {
            return Err("closed-loop class counters differ from caller terminals".into());
        }
        Ok(())
    }
}

/// The returned raw artifact is retained even after a caller task failure.
/// It is diagnostic until an external source-bound receipt qualifies it.
pub async fn run_closed_loop_with_topology(
    scenario: &ClosedLoopScenario,
) -> Result<(ClosedLoopRun, ResolvedHostTopology), String> {
    scenario.validate()?;
    let host_shape = scenario.host_shape();
    let runtime = host_shape.build_runtime()?;
    let topology = ResolvedHostTopology::from_runtime(&runtime);
    warmup_runtime(&host_shape, &runtime).await?;
    let baseline = runtime.snapshot();
    if baseline.conservation_violation().is_some()
        || baseline
            .classes
            .values()
            .any(|class| class.inflight != 0 || class.queued != 0)
    {
        return Err("closed-loop warmup did not settle".into());
    }
    let total = scenario.concurrency * scenario.iterations_per_slot;
    let slots: Arc<Vec<Mutex<Option<ClosedLoopRecord>>>> =
        Arc::new((0..total).map(|_| Mutex::new(None)).collect());
    let origin = Instant::now();
    let mut jobs = Vec::with_capacity(scenario.concurrency);
    for caller_slot in 0..scenario.concurrency {
        let runtime = runtime.clone();
        let offer = scenario.offer();
        let slots = Arc::clone(&slots);
        let iterations = scenario.iterations_per_slot;
        jobs.push(tokio::spawn(async move {
            for iteration in 0..iterations {
                let id = caller_slot * iterations + iteration;
                let submitted_ns = since(origin);
                let submitted_at = origin + Duration::from_nanos(submitted_ns);
                let response = execute(
                    runtime.clone(),
                    offer.clone(),
                    None,
                    id,
                    origin,
                    submitted_at,
                    None,
                )
                .await;
                let response_ns = since(origin);
                *slots[id].lock().expect("closed-loop record lock") = Some(ClosedLoopRecord {
                    id,
                    caller_slot,
                    iteration,
                    submitted_ns,
                    response_ns,
                    outcome: classify_response(response),
                });
            }
        }));
    }
    let mut problems = Vec::new();
    let deadline =
        tokio::time::Instant::from_std(origin + Duration::from_millis(scenario.max_run_ms));
    for job in &mut jobs {
        match tokio::time::timeout_at(deadline, job).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => problems.push("closed-loop caller task failed"),
            Err(_) => {
                problems.push("closed-loop maximum run time exceeded");
                break;
            }
        }
    }
    for job in jobs {
        if !job.is_finished() {
            job.abort();
            let _ = job.await;
        }
    }
    let elapsed_ns = since(origin);
    if elapsed_ns > scenario.max_run_ms * 1_000_000 {
        problems.push("closed-loop maximum run time exceeded");
    }
    let drain_ok = runtime
        .drain(Duration::from_millis(scenario.settlement_ms))
        .await
        .is_ok();
    let final_snapshot = runtime.snapshot();
    if !drain_ok {
        problems.push("closed-loop host drain failed");
    }
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
    let run = ClosedLoopRun {
        schema_version: CLOSED_LOOP_VERSION,
        mode: "closed_loop_fixed_concurrency".into(),
        scenario_id: scenario.id.clone(),
        concurrency: scenario.concurrency,
        iterations_per_slot: scenario.iterations_per_slot,
        elapsed_ns,
        records: slots
            .iter()
            .map(|slot| slot.lock().expect("closed-loop record lock").clone())
            .collect(),
        class_counters,
        final_capabilities: final_snapshot
            .capabilities
            .iter()
            .map(|(pool, usage)| (pool.clone(), usage.in_use))
            .collect(),
        drain_ok,
        conservation_ok: final_snapshot.conservation_violation().is_none(),
        invalid_reason: if problems.is_empty() {
            None
        } else {
            Some(problems.join("; "))
        },
    };
    Ok((run, topology))
}
