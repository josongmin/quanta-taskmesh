//! Recorder-minimal Taskmesh host control.
//!
//! The public work and finite pacer match the full host runner. Per-request
//! response/body timestamps are omitted, so this artifact cannot report
//! latency, worker-finish timing, or an SLO-goodput numerator.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use taskmesh::{Runtime, TaskClass};

use crate::host_load::{
    classify_response, execute, pace_offer, since, warmup_runtime, ClassCounters, HostHarnessFault,
    HostRunStatus, OutstandingGuard, ProducerDecision, ResolvedHostTopology, ResponseOutcome,
};
use crate::host_scenarios::HostScenario;

pub const MINIMAL_HOST_VERSION: u32 = 1;
const UNSET_NS: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum MinimalDisposition {
    Pending = 0,
    NotSubmitted = 1,
    Submitted = 2,
    Success = 3,
    Rejected = 4,
    Deadline = 5,
    Cancelled = 6,
    TaskError = 7,
    GovernorError = 8,
    CallerDropped = 9,
    UnansweredAtSettlement = 10,
}

impl MinimalDisposition {
    fn from_byte(value: u8) -> Result<Self, String> {
        match value {
            0 => Ok(Self::Pending),
            1 => Ok(Self::NotSubmitted),
            2 => Ok(Self::Submitted),
            3 => Ok(Self::Success),
            4 => Ok(Self::Rejected),
            5 => Ok(Self::Deadline),
            6 => Ok(Self::Cancelled),
            7 => Ok(Self::TaskError),
            8 => Ok(Self::GovernorError),
            9 => Ok(Self::CallerDropped),
            10 => Ok(Self::UnansweredAtSettlement),
            _ => Err("unknown minimal recorder disposition".into()),
        }
    }

    fn from_response(outcome: ResponseOutcome) -> Self {
        match outcome {
            ResponseOutcome::Success => Self::Success,
            ResponseOutcome::Rejected { .. } => Self::Rejected,
            ResponseOutcome::Deadline => Self::Deadline,
            ResponseOutcome::Cancelled => Self::Cancelled,
            ResponseOutcome::TaskError => Self::TaskError,
            ResponseOutcome::GovernorError { .. } => Self::GovernorError,
        }
    }

    fn responded(self) -> bool {
        matches!(
            self,
            Self::Success
                | Self::Rejected
                | Self::Deadline
                | Self::Cancelled
                | Self::TaskError
                | Self::GovernorError
        )
    }
}

struct MinimalSlot {
    observed_ns: AtomicU64,
    submitted_ns: AtomicU64,
    disposition: AtomicU8,
}

impl MinimalSlot {
    fn new() -> Self {
        Self {
            observed_ns: AtomicU64::new(UNSET_NS),
            submitted_ns: AtomicU64::new(UNSET_NS),
            disposition: AtomicU8::new(MinimalDisposition::Pending as u8),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MinimalRecord {
    pub id: usize,
    pub intended_ns: u64,
    pub observed_ns: Option<u64>,
    pub submitted_ns: Option<u64>,
    pub disposition: MinimalDisposition,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MinimalCounts {
    pub intended: usize,
    pub submitted: usize,
    pub not_submitted: usize,
    pub responded: usize,
    pub caller_dropped: usize,
    pub unanswered_at_settlement: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MinimalHostRun {
    pub schema_version: u32,
    pub status: HostRunStatus,
    pub scenario_id: String,
    pub injection_window_ns: u64,
    pub recorder_mode: String,
    pub response_latency_available: bool,
    pub counts: MinimalCounts,
    pub completed_callers: usize,
    pub outstanding_final: usize,
    pub max_producer_lag_ns: Option<u64>,
    pub records: Vec<MinimalRecord>,
    pub class_counters: BTreeMap<String, ClassCounters>,
    pub final_capabilities: BTreeMap<String, u32>,
    pub drain_ok: bool,
    pub conservation_ok: bool,
}

impl MinimalHostRun {
    pub fn validate_against(&self, scenario: &HostScenario) -> Result<(), String> {
        scenario.validate()?;
        if self.status != HostRunStatus::Complete {
            return Err("minimal host run is invalid".into());
        }
        scenario
            .resolved_topology()?
            .validate_capability_usage(&self.final_capabilities, "minimal host final inventory")?;
        if self.schema_version != MINIMAL_HOST_VERSION
            || self.scenario_id != scenario.id
            || self.injection_window_ns != scenario.load.injection_ms * 1_000_000
            || self.recorder_mode != "minimal"
            || self.response_latency_available
            || scenario.load.snapshot_ms != 0
            || self.records.len() != scenario.offers.len()
        {
            return Err("minimal host scenario or mode differs".into());
        }
        let mut counts = MinimalCounts {
            intended: self.records.len(),
            ..MinimalCounts::default()
        };
        let mut max_lag = None;
        for (id, (row, offer)) in self.records.iter().zip(&scenario.offers).enumerate() {
            if row.id != id || row.intended_ns != offer.send_time_ns {
                return Err(format!("minimal row {id}: offer identity differs"));
            }
            let observed = row
                .observed_ns
                .ok_or(format!("minimal row {id}: never paced"))?;
            if observed < row.intended_ns {
                return Err(format!("minimal row {id}: observation precedes schedule"));
            }
            let lag = observed - row.intended_ns;
            max_lag = Some(max_lag.map_or(lag, |current: u64| current.max(lag)));
            match row.disposition {
                MinimalDisposition::NotSubmitted => {
                    if row.submitted_ns.is_some() {
                        return Err(format!("minimal row {id}: impossible submit"));
                    }
                    counts.not_submitted += 1;
                }
                MinimalDisposition::Pending | MinimalDisposition::Submitted => {
                    return Err(format!("minimal row {id}: no terminal"));
                }
                disposition => {
                    if !row.submitted_ns.is_some_and(|submit| {
                        submit >= observed && submit < self.injection_window_ns
                    }) {
                        return Err(format!("minimal row {id}: invalid submit time"));
                    }
                    counts.submitted += 1;
                    if disposition.responded() {
                        counts.responded += 1;
                    } else if disposition == MinimalDisposition::CallerDropped {
                        counts.caller_dropped += 1;
                    } else {
                        counts.unanswered_at_settlement += 1;
                    }
                }
            }
        }
        if counts != self.counts
            || counts.intended != counts.submitted + counts.not_submitted
            || counts.submitted
                != counts.responded + counts.caller_dropped + counts.unanswered_at_settlement
            || self.completed_callers != counts.responded + counts.caller_dropped
            || counts.unanswered_at_settlement != 0
            || self.outstanding_final != 0
            || self.max_producer_lag_ns != max_lag
            || !self.drain_ok
            || !self.conservation_ok
        {
            return Err("minimal host conservation or settlement failed".into());
        }
        let mut submitted_by_class = BTreeMap::<&str, u128>::new();
        let mut success_by_class = BTreeMap::<&str, u128>::new();
        for (offer, row) in scenario.offers.iter().zip(&self.records) {
            if row.submitted_ns.is_some() {
                *submitted_by_class.entry(offer.class.as_str()).or_default() += 1;
            }
            if row.disposition == MinimalDisposition::Success {
                *success_by_class.entry(offer.class.as_str()).or_default() += 1;
            }
        }
        if self.class_counters.len() != scenario.classes.len()
            || scenario.classes.iter().any(|class| {
                self.class_counters
                    .get(&class.name)
                    .map_or(true, |counters| {
                        let success = *success_by_class.get(class.name.as_str()).unwrap_or(&0);
                        let submitted = *submitted_by_class.get(class.name.as_str()).unwrap_or(&0);
                        counters.admitted < success
                            || counters.admitted > submitted
                            || counters.started < success
                            || counters.terminated < success
                            || counters.started > counters.admitted
                            || counters.terminated != counters.admitted
                            || counters.inflight != 0
                            || counters.queued != 0
                    })
            })
            || self.final_capabilities.is_empty()
            || self.final_capabilities.values().any(|held| *held != 0)
        {
            return Err("minimal host final governor ledger differs".into());
        }
        Ok(())
    }
}

fn nullable(value: u64) -> Option<u64> {
    (value != UNSET_NS).then_some(value)
}

pub async fn run_minimal_host_scenario(
    scenario: &HostScenario,
    fault: Option<HostHarnessFault>,
) -> Result<(MinimalHostRun, ResolvedHostTopology), String> {
    scenario.validate()?;
    if scenario.load.snapshot_ms != 0 {
        return Err("minimal recorder control requires Snapshot sampling off".into());
    }
    if fault == Some(HostHarnessFault::SamplerPanic) {
        return Err("minimal recorder control has no Snapshot sampler".into());
    }
    let runtime = scenario.build_runtime()?;
    let topology = ResolvedHostTopology::from_runtime(&runtime);
    warmup_runtime(scenario, &runtime).await?;
    let baseline = runtime.snapshot();
    if baseline.conservation_violation().is_some()
        || baseline
            .classes
            .values()
            .any(|class| class.inflight != 0 || class.queued != 0)
        || baseline
            .capabilities
            .values()
            .any(|usage| usage.in_use != 0)
    {
        return Err("minimal recorder warmup did not settle".into());
    }
    let slots: Vec<_> = (0..scenario.offers.len())
        .map(|_| Arc::new(MinimalSlot::new()))
        .collect();
    let producer_slots = slots.clone();
    let offers = scenario.offers.clone();
    let outstanding = Arc::new(AtomicUsize::new(0));
    let producer_outstanding = Arc::clone(&outstanding);
    let handle = tokio::runtime::Handle::current();
    let producer_runtime = runtime.clone();
    let injection = Duration::from_millis(scenario.load.injection_ms);
    let max_outstanding = scenario.load.max_outstanding;
    let origin = Instant::now();
    let (producer_tx, producer_rx) = tokio::sync::oneshot::channel();
    let producer = std::thread::spawn(move || {
        let mut jobs = Vec::with_capacity(offers.len());
        for (id, offer) in offers.into_iter().enumerate() {
            if fault == Some(HostHarnessFault::ProducerBeforeOffer(id)) {
                panic!("injected minimal recorder producer failure before offer {id}");
            }
            let slot = Arc::clone(&producer_slots[id]);
            let (observed_ns, decision) = pace_offer(
                origin,
                offer.send_time_ns,
                u64::try_from(injection.as_nanos()).unwrap_or(u64::MAX),
                &producer_outstanding,
                max_outstanding,
            );
            slot.observed_ns.store(observed_ns, Ordering::Release);
            match decision {
                ProducerDecision::NotSubmitted { .. } => {
                    slot.disposition
                        .store(MinimalDisposition::NotSubmitted as u8, Ordering::Release);
                    continue;
                }
                ProducerDecision::Submit { .. } => {}
            }
            producer_outstanding.fetch_add(1, Ordering::AcqRel);
            let submit_ns = since(origin);
            let submit_at = origin + Duration::from_nanos(submit_ns);
            slot.submitted_ns.store(submit_ns, Ordering::Release);
            slot.disposition
                .store(MinimalDisposition::Submitted as u8, Ordering::Release);
            let guard = OutstandingGuard(Arc::clone(&producer_outstanding));
            let task_runtime = producer_runtime.clone();
            jobs.push(handle.spawn(async move {
                let _guard = guard;
                let drop_after = offer.drop_after_ms;
                let run = execute(task_runtime, offer, None, id, origin, submit_at, None);
                let outcome = if let Some(ms) = drop_after {
                    tokio::select! {
                        result = run => MinimalDisposition::from_response(classify_response(result)),
                        () = tokio::time::sleep_until(tokio::time::Instant::from_std(
                            submit_at + Duration::from_millis(ms)
                        )) => MinimalDisposition::CallerDropped,
                    }
                } else {
                    MinimalDisposition::from_response(classify_response(run.await))
                };
                slot.disposition.store(outcome as u8, Ordering::Release);
            }));
        }
        if let Some(delay) = (origin + injection).checked_duration_since(Instant::now()) {
            std::thread::sleep(delay);
        }
        let _ = producer_tx.send(jobs);
    });
    let mut issues = Vec::new();
    let mut jobs = match producer_rx.await {
        Ok(jobs) => jobs,
        Err(_) => {
            issues.push("minimal recorder producer failed before reporting jobs");
            Vec::new()
        }
    };
    if producer.join().is_err() {
        issues.push("minimal recorder producer panicked");
    }
    let until = Instant::now() + Duration::from_millis(scenario.load.settlement_ms);
    while jobs.iter().any(|job| !job.is_finished()) && Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    for job in &mut jobs {
        if !job.is_finished() {
            job.abort();
        }
    }
    let mut completed_callers = 0;
    for job in jobs {
        match job.await {
            Ok(()) => completed_callers += 1,
            Err(_) => issues.push("minimal recorder caller failed or was aborted"),
        }
    }
    while outstanding.load(Ordering::Acquire) != 0 && Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    for slot in &slots {
        if slot.disposition.load(Ordering::Acquire) == MinimalDisposition::Submitted as u8 {
            slot.disposition.store(
                MinimalDisposition::UnansweredAtSettlement as u8,
                Ordering::Release,
            );
        }
    }
    let drain_ok = runtime
        .drain(Duration::from_millis(scenario.load.settlement_ms))
        .await
        .is_ok();
    let final_snapshot = runtime.snapshot();
    let conservation_ok = final_snapshot.conservation_violation().is_none();
    if !drain_ok {
        issues.push("minimal recorder host did not drain");
    }
    if !conservation_ok {
        issues.push("minimal recorder conservation failed");
    }
    let mut records = Vec::with_capacity(slots.len());
    for (id, slot) in slots.iter().enumerate() {
        records.push(MinimalRecord {
            id,
            intended_ns: scenario.offers[id].send_time_ns,
            observed_ns: nullable(slot.observed_ns.load(Ordering::Acquire)),
            submitted_ns: nullable(slot.submitted_ns.load(Ordering::Acquire)),
            disposition: MinimalDisposition::from_byte(slot.disposition.load(Ordering::Acquire))?,
        });
    }
    let counts = MinimalCounts {
        intended: records.len(),
        submitted: records
            .iter()
            .filter(|row| row.submitted_ns.is_some())
            .count(),
        not_submitted: records
            .iter()
            .filter(|row| row.disposition == MinimalDisposition::NotSubmitted)
            .count(),
        responded: records
            .iter()
            .filter(|row| row.disposition.responded())
            .count(),
        caller_dropped: records
            .iter()
            .filter(|row| row.disposition == MinimalDisposition::CallerDropped)
            .count(),
        unanswered_at_settlement: records
            .iter()
            .filter(|row| row.disposition == MinimalDisposition::UnansweredAtSettlement)
            .count(),
    };
    let max_producer_lag_ns = records
        .iter()
        .filter_map(|row| {
            row.observed_ns
                .map(|observed| observed.saturating_sub(row.intended_ns))
        })
        .max();
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
    let run = MinimalHostRun {
        schema_version: MINIMAL_HOST_VERSION,
        status: if issues.is_empty() {
            HostRunStatus::Complete
        } else {
            HostRunStatus::Invalid {
                reason: issues.join("; "),
            }
        },
        scenario_id: scenario.id.clone(),
        injection_window_ns: scenario.load.injection_ms * 1_000_000,
        recorder_mode: "minimal".into(),
        response_latency_available: false,
        counts,
        completed_callers,
        outstanding_final: outstanding.load(Ordering::Acquire),
        max_producer_lag_ns,
        records,
        class_counters,
        final_capabilities: final_snapshot
            .capabilities
            .iter()
            .map(|(name, usage)| (name.clone(), usage.in_use))
            .collect(),
        drain_ok,
        conservation_ok,
    };
    Ok((run, topology))
}
