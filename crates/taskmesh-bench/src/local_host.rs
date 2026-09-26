//! Caller-affine finite local-path diagnostic on one current-thread executor.
//!
//! The pacer, caller tasks and `!Send` payload stay on one thread. A delayed
//! pacer is recorded as lag or `not_submitted`; no hidden Send queue is added.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use taskmesh::{Runtime, TaskClass, TaskSpec};

use crate::host_load::{
    classify_response, since, ClassCounters, ResolvedHostTopology, ResponseOutcome,
};
use crate::host_scenarios::{
    HostBody, HostClass, HostLoadEnvelope, HostOffer, HostPath, HostScenario, HostTopology,
};

pub const LOCAL_HOST_VERSION: u32 = 2;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalHostScenario {
    pub schema_version: u32,
    pub id: String,
    pub load: HostLoadEnvelope,
    pub topology: HostTopology,
    pub classes: Vec<HostClass>,
    pub offers: Vec<LocalOffer>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalOffer {
    pub send_time_ns: u64,
    pub class: String,
    pub body: HostBody,
}

impl LocalHostScenario {
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let scenario: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        scenario.validate()?;
        Ok(scenario)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != LOCAL_HOST_VERSION {
            return Err("unsupported local host scenario version".into());
        }
        if self.load.snapshot_ms != 0 {
            return Err("local host Snapshot sampling is unavailable".into());
        }
        self.host_shape().validate()
    }

    pub fn resolved_topology(&self) -> Result<ResolvedHostTopology, String> {
        self.validate()?;
        Ok(ResolvedHostTopology::from_runtime(
            &self.host_shape().build_runtime()?,
        ))
    }

    // Reuse class/topology/arrival/body validation. `Io` here is only the
    // preflight shape; the measured dispatch below is TaskSpec::local.
    fn host_shape(&self) -> HostScenario {
        HostScenario {
            schema_version: 1,
            id: self.id.clone(),
            load: self.load.clone(),
            topology: self.topology.clone(),
            classes: self.classes.clone(),
            offers: self
                .offers
                .iter()
                .map(|offer| HostOffer {
                    send_time_ns: offer.send_time_ns,
                    class: offer.class.clone(),
                    path: HostPath::Io,
                    body: offer.body.clone(),
                    stack_size_bytes: None,
                    deadline_ms: None,
                    cancel_after_ms: None,
                    drop_after_ms: None,
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalRecord {
    pub id: usize,
    pub class: String,
    pub intended_ns: u64,
    pub pacer_observed_ns: u64,
    pub scheduled_lag_ns: u64,
    pub submitted_ns: Option<u64>,
    pub body_started_ns: Option<u64>,
    pub body_finished_ns: Option<u64>,
    pub response_ns: Option<u64>,
    pub outcome: Option<ResponseOutcome>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalHostRun {
    pub schema_version: u32,
    pub mode: String,
    pub scenario_id: String,
    pub injection_window_ns: u64,
    pub records: Vec<LocalRecord>,
    pub class_counters: BTreeMap<String, ClassCounters>,
    pub final_capabilities: BTreeMap<String, u32>,
    pub drain_ok: bool,
    pub conservation_ok: bool,
    pub invalid_reason: Option<String>,
}

impl LocalHostRun {
    pub fn validate_against(&self, scenario: &LocalHostScenario) -> Result<(), String> {
        scenario.validate()?;
        if self.schema_version != LOCAL_HOST_VERSION
            || self.mode != "caller_affine_local_open_loop"
            || self.scenario_id != scenario.id
            || self.injection_window_ns != scenario.load.injection_ms * 1_000_000
            || self.records.len() != scenario.offers.len()
        {
            return Err("local raw identity or population differs".into());
        }
        if let Some(reason) = &self.invalid_reason {
            return Err(format!("invalid local host run: {reason}"));
        }
        let mut started_by_class = BTreeMap::<&str, u128>::new();
        for (id, (row, offer)) in self.records.iter().zip(&scenario.offers).enumerate() {
            if row.id != id || row.class != offer.class || row.intended_ns != offer.send_time_ns {
                return Err(format!("local row {id}: offer identity differs"));
            }
            if row.pacer_observed_ns < row.intended_ns
                || row.scheduled_lag_ns != row.pacer_observed_ns - row.intended_ns
            {
                return Err(format!("local row {id}: invalid pacer observation or lag"));
            }
            match row.submitted_ns {
                None => {
                    if row.body_started_ns.is_some()
                        || row.body_finished_ns.is_some()
                        || row.response_ns.is_some()
                        || row.outcome.is_some()
                    {
                        return Err(format!("local row {id}: unsubmitted offer has activity"));
                    }
                }
                Some(submit) => {
                    if submit != row.pacer_observed_ns || submit >= self.injection_window_ns {
                        return Err(format!("local row {id}: invalid submit or lag"));
                    }
                    let response = row
                        .response_ns
                        .ok_or_else(|| format!("local row {id}: missing caller terminal"))?;
                    if response < submit {
                        return Err(format!("local row {id}: response precedes submission"));
                    }
                    match row.outcome.as_ref() {
                        Some(ResponseOutcome::Success) => {
                            let start = row.body_started_ns.ok_or_else(|| {
                                format!("local row {id}: success lacks body start")
                            })?;
                            let finish = row.body_finished_ns.ok_or_else(|| {
                                format!("local row {id}: success lacks body finish")
                            })?;
                            if start < submit || finish < start || response < finish {
                                return Err(format!("local row {id}: impossible body order"));
                            }
                        }
                        Some(ResponseOutcome::Rejected { .. }) => {
                            if row.body_started_ns.is_some() || row.body_finished_ns.is_some() {
                                return Err(format!("local row {id}: rejected body started"));
                            }
                        }
                        _ => return Err(format!("local row {id}: unexpected caller outcome")),
                    }
                    if row.body_started_ns.is_some() {
                        *started_by_class.entry(&row.class).or_default() += 1;
                    }
                }
            }
        }
        if !self.drain_ok
            || !self.conservation_ok
            || self.class_counters.len() != scenario.classes.len()
            || self.final_capabilities.is_empty()
            || self.final_capabilities.values().any(|held| *held != 0)
        {
            return Err("local host did not settle or class catalog differs".into());
        }
        for class in &scenario.classes {
            let counters = self
                .class_counters
                .get(&class.name)
                .ok_or_else(|| format!("local class {}: missing counters", class.name))?;
            if counters.started != *started_by_class.get(class.name.as_str()).unwrap_or(&0)
                || counters.admitted != counters.started
                || counters.terminated != counters.admitted
                || counters.inflight != 0
                || counters.queued != 0
            {
                return Err(format!(
                    "local class {}: counters differ from rows",
                    class.name
                ));
            }
        }
        Ok(())
    }
}

pub async fn run_local_host_with_topology(
    scenario: &LocalHostScenario,
) -> Result<(LocalHostRun, ResolvedHostTopology), String> {
    scenario.validate()?;
    tokio::task::LocalSet::new()
        .run_until(run_inner(scenario))
        .await
}

async fn run_inner(
    scenario: &LocalHostScenario,
) -> Result<(LocalHostRun, ResolvedHostTopology), String> {
    let runtime = scenario.host_shape().build_runtime()?;
    let topology = ResolvedHostTopology::from_runtime(&runtime);
    let warmed_classes: BTreeSet<_> = scenario.offers.iter().map(|offer| &offer.class).collect();
    for class in warmed_classes {
        let marker = Rc::new(Cell::new(0_u8));
        let payload = Rc::clone(&marker);
        runtime
            .run_local(
                TaskSpec::local(TaskClass::new(class.clone())).operation("local-warmup"),
                async move {
                    payload.set(1);
                    Ok::<(), ()>(())
                },
            )
            .await
            .map_err(|error| format!("local warmup failed: {error:?}"))?;
        if marker.get() != 1 {
            return Err("local warmup payload did not run".into());
        }
    }
    tokio::time::sleep(Duration::from_millis(scenario.load.warmup_ms)).await;
    let baseline = runtime.snapshot();
    if baseline.conservation_violation().is_some()
        || baseline
            .classes
            .values()
            .any(|class| class.inflight != 0 || class.queued != 0)
    {
        return Err("local warmup did not settle".into());
    }
    let rows: Rc<Vec<RefCell<LocalRecord>>> = Rc::new(
        scenario
            .offers
            .iter()
            .enumerate()
            .map(|(id, offer)| {
                RefCell::new(LocalRecord {
                    id,
                    class: offer.class.clone(),
                    intended_ns: offer.send_time_ns,
                    pacer_observed_ns: 0,
                    scheduled_lag_ns: 0,
                    submitted_ns: None,
                    body_started_ns: None,
                    body_finished_ns: None,
                    response_ns: None,
                    outcome: None,
                })
            })
            .collect(),
    );
    let active = Rc::new(Cell::new(0_usize));
    let origin = Instant::now();
    let cut_ns = scenario.load.injection_ms * 1_000_000;
    let mut jobs = Vec::with_capacity(scenario.offers.len());
    for (id, offer) in scenario.offers.iter().enumerate() {
        tokio::time::sleep_until(tokio::time::Instant::from_std(
            origin + Duration::from_nanos(offer.send_time_ns),
        ))
        .await;
        let observed_ns = since(origin);
        {
            let mut row = rows[id].borrow_mut();
            row.pacer_observed_ns = observed_ns;
            row.scheduled_lag_ns = observed_ns.saturating_sub(offer.send_time_ns);
        }
        if observed_ns >= cut_ns || active.get() >= scenario.load.max_outstanding {
            continue;
        }
        active.set(active.get() + 1);
        rows[id].borrow_mut().submitted_ns = Some(observed_ns);
        let runtime = runtime.clone();
        let rows = Rc::clone(&rows);
        let active = Rc::clone(&active);
        let class = TaskClass::new(offer.class.clone());
        let sleep_ms = match &offer.body {
            HostBody::Noop => 0,
            HostBody::AsyncSleep { millis } => *millis,
            _ => unreachable!("local preflight accepts only async bodies"),
        };
        jobs.push(tokio::task::spawn_local(async move {
            let payload = Rc::new(Cell::new(0_u8));
            let local = Rc::clone(&payload);
            let body_rows = Rc::clone(&rows);
            let result = runtime
                .run_local(
                    TaskSpec::local(class).operation(format!("local-{id}")),
                    async move {
                        local.set(1);
                        body_rows[id].borrow_mut().body_started_ns = Some(since(origin));
                        if sleep_ms != 0 {
                            tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                        }
                        body_rows[id].borrow_mut().body_finished_ns = Some(since(origin));
                        Ok::<(), ()>(())
                    },
                )
                .await;
            let outcome = classify_response(result);
            if matches!(outcome, ResponseOutcome::Success) && payload.get() != 1 {
                panic!("local payload did not retain caller affinity");
            }
            let mut row = rows[id].borrow_mut();
            row.response_ns = Some(since(origin));
            row.outcome = Some(outcome);
            active.set(active.get() - 1);
        }));
    }
    tokio::time::sleep_until(tokio::time::Instant::from_std(
        origin + Duration::from_nanos(cut_ns),
    ))
    .await;
    let mut issues = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(scenario.load.settlement_ms);
    for job in &mut jobs {
        match tokio::time::timeout_at(deadline, job).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => issues.push("local caller task failed"),
            Err(_) => {
                issues.push("local caller settlement expired");
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
    let drain_ok = runtime
        .drain(Duration::from_millis(scenario.load.settlement_ms))
        .await
        .is_ok();
    let final_snapshot = runtime.snapshot();
    if !drain_ok {
        issues.push("local host drain failed");
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
    Ok((
        LocalHostRun {
            schema_version: LOCAL_HOST_VERSION,
            mode: "caller_affine_local_open_loop".into(),
            scenario_id: scenario.id.clone(),
            injection_window_ns: cut_ns,
            records: rows.iter().map(|row| row.borrow().clone()).collect(),
            class_counters,
            final_capabilities: final_snapshot
                .capabilities
                .iter()
                .map(|(pool, usage)| (pool.clone(), usage.in_use))
                .collect(),
            drain_ok,
            conservation_ok: final_snapshot.conservation_violation().is_none(),
            invalid_reason: if issues.is_empty() {
                None
            } else {
                Some(issues.join("; "))
            },
        },
        topology,
    ))
}
