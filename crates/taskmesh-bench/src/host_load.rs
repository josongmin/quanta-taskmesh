//! Bounded, paced measurements of the public Taskmesh host.
//!
//! The producer keeps one slot per *intended* offer. Worker completion is a
//! separate ledger from the caller's response or intentional drop.

use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use taskmesh::{
    AdmissionVerdict, RunError, Runtime, RuntimeConfig, SubmitOptions, SubstrateHint, TaskClass,
    TaskSpec, TokioRuntime,
};
use tokio_util::sync::CancellationToken;

use crate::host_scenarios::{HostBody, HostOffer, HostPath, HostScenario};

pub const HOST_RAW_VERSION: u32 = 2;

/// Installed facts from the actual measured runtime, outside the timed span.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedHostTopology {
    pub schema_version: u32,
    pub runtime_config: RuntimeConfig,
    pub capability_limits: BTreeMap<String, u32>,
    pub cpu_executor: CpuExecutorReceipt,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CpuExecutorReceipt {
    pub nonblocking_submit: bool,
    pub declared_workers: Option<u32>,
    pub exclusive_pool: bool,
    pub physical_domain: Option<String>,
    pub requires_tokio_context: bool,
}

impl ResolvedHostTopology {
    pub fn from_runtime(runtime: &TokioRuntime) -> Self {
        let capabilities = runtime.executor_capabilities();
        Self {
            schema_version: 1,
            runtime_config: runtime.config().clone(),
            capability_limits: runtime
                .snapshot()
                .capabilities
                .into_iter()
                .map(|(name, usage)| (name, usage.limit))
                .collect(),
            cpu_executor: CpuExecutorReceipt {
                nonblocking_submit: capabilities.nonblocking_submit,
                declared_workers: capabilities.declared_workers,
                exclusive_pool: capabilities.exclusive_pool,
                physical_domain: capabilities.physical_domain.map(str::to_owned),
                requires_tokio_context: capabilities.requires_tokio_context,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostRunStatus {
    Complete,
    Invalid { reason: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostHarnessFault {
    ProducerBeforeOffer(usize),
    SamplerPanic,
}

/// Test-only control for one blocking offer. It is never serialized into a
/// measured scenario or used by the diagnostic CLI.
pub struct HostHarnessTestGate {
    offer_id: usize,
    started: CancellationToken,
    drop_now: CancellationToken,
    caller_dropped: CancellationToken,
    release: (Mutex<bool>, Condvar),
}

impl HostHarnessTestGate {
    pub fn new(offer_id: usize) -> Self {
        Self {
            offer_id,
            started: CancellationToken::new(),
            drop_now: CancellationToken::new(),
            caller_dropped: CancellationToken::new(),
            release: (Mutex::new(false), Condvar::new()),
        }
    }

    pub async fn wait_started(&self) {
        self.started.cancelled().await;
    }

    pub fn trigger_drop(&self) {
        self.drop_now.cancel();
    }

    pub async fn wait_caller_dropped(&self) {
        self.caller_dropped.cancelled().await;
    }

    pub fn release_worker(&self) {
        let (lock, changed) = &self.release;
        *lock.lock().expect("test gate lock") = true;
        changed.notify_all();
    }

    fn wait_for_release(&self) {
        self.started.cancel();
        let (lock, changed) = &self.release;
        let (ready, _) = changed
            .wait_timeout_while(
                lock.lock().expect("test gate lock"),
                Duration::from_secs(10),
                |ready| !*ready,
            )
            .expect("test gate wait");
        assert!(*ready, "test gate worker was never released");
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CallerDisposition {
    Waiting,
    NotSubmitted,
    Responded { outcome: ResponseOutcome },
    CallerDropped,
    UnansweredAtSettlement,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResponseOutcome {
    Success,
    Rejected { verdict: AdmissionVerdict },
    Deadline,
    Cancelled,
    TaskError,
    GovernorError { error: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawHostRecord {
    pub id: usize,
    pub class: String,
    pub path: HostPath,
    pub intended_ns: u64,
    pub scheduled_lag_ns: Option<u64>,
    pub submitted_ns: Option<u64>,
    pub body_started_ns: Option<u64>,
    pub caller_response_ns: Option<u64>,
    pub caller_drop_ns: Option<u64>,
    pub body_finished_ns: Option<u64>,
    pub disposition: CallerDisposition,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CallerCounts {
    pub intended: usize,
    pub submitted: usize,
    pub not_submitted: usize,
    pub responded: usize,
    pub caller_dropped: usize,
    pub waiting: usize,
    pub unanswered_at_settlement: usize,
}

impl CallerCounts {
    pub fn from_records(records: &[RawHostRecord]) -> Result<Self, String> {
        let mut counts = Self {
            intended: records.len(),
            ..Self::default()
        };
        for (expected_id, row) in records.iter().enumerate() {
            if row.id != expected_id {
                return Err(format!(
                    "row {expected_id}: missing, duplicated or reordered id"
                ));
            }
            if row.submitted_ns.is_some() {
                counts.submitted += 1;
            }
            match &row.disposition {
                CallerDisposition::Waiting => counts.waiting += 1,
                CallerDisposition::NotSubmitted => counts.not_submitted += 1,
                CallerDisposition::Responded { .. } => counts.responded += 1,
                CallerDisposition::CallerDropped => counts.caller_dropped += 1,
                CallerDisposition::UnansweredAtSettlement => counts.unanswered_at_settlement += 1,
            }
            row.validate()?;
        }
        if counts.intended != counts.submitted + counts.not_submitted {
            return Err("intended != submitted + not_submitted".into());
        }
        if counts.submitted
            != counts.responded
                + counts.caller_dropped
                + counts.waiting
                + counts.unanswered_at_settlement
        {
            return Err("submitted caller dispositions do not reconcile".into());
        }
        Ok(counts)
    }

    pub fn at_cut(records: &[RawHostRecord], cut_ns: u64) -> Result<Self, String> {
        let settled = Self::from_records(records)?;
        let mut cut = Self {
            intended: settled.intended,
            submitted: settled.submitted,
            not_submitted: settled.not_submitted,
            ..Self::default()
        };
        for row in records {
            if row.submitted_ns.is_none() {
                continue;
            }
            if row.submitted_ns.is_some_and(|submit| submit >= cut_ns) {
                return Err(format!("row {}: submitted after injection cut", row.id));
            }
            match (&row.disposition, row.caller_response_ns, row.caller_drop_ns) {
                (CallerDisposition::Responded { .. }, Some(response), _) if response < cut_ns => {
                    cut.responded += 1;
                }
                (CallerDisposition::CallerDropped, _, Some(drop)) if drop < cut_ns => {
                    cut.caller_dropped += 1;
                }
                _ => cut.waiting += 1,
            }
        }
        Ok(cut)
    }
}

impl RawHostRecord {
    fn validate(&self) -> Result<(), String> {
        let id = self.id;
        if self.class.is_empty() {
            return Err(format!("row {id}: empty class"));
        }
        if self.scheduled_lag_ns.is_none() {
            return Err(format!("row {id}: missing producer lag"));
        }
        if let Some(submit) = self.submitted_ns {
            if submit < self.intended_ns {
                return Err(format!("row {id}: submit precedes intended arrival"));
            }
            if self
                .scheduled_lag_ns
                .is_some_and(|lag| lag > submit - self.intended_ns)
            {
                return Err(format!("row {id}: producer lag exceeds submit delay"));
            }
        }
        match self.disposition {
            CallerDisposition::NotSubmitted => {
                if self.submitted_ns.is_some()
                    || self.body_started_ns.is_some()
                    || self.caller_response_ns.is_some()
                    || self.caller_drop_ns.is_some()
                    || self.body_finished_ns.is_some()
                {
                    return Err(format!("row {id}: not-submitted offer has activity"));
                }
            }
            CallerDisposition::Responded { .. } => {
                if self.submitted_ns.is_none()
                    || self.caller_response_ns.is_none()
                    || self.caller_drop_ns.is_some()
                {
                    return Err(format!("row {id}: invalid responded timestamps"));
                }
            }
            CallerDisposition::CallerDropped => {
                if self.submitted_ns.is_none()
                    || self.caller_drop_ns.is_none()
                    || self.caller_response_ns.is_some()
                {
                    return Err(format!("row {id}: invalid drop timestamps"));
                }
            }
            CallerDisposition::Waiting | CallerDisposition::UnansweredAtSettlement => {
                if self.submitted_ns.is_none()
                    || self.caller_response_ns.is_some()
                    || self.caller_drop_ns.is_some()
                {
                    return Err(format!("row {id}: invalid outstanding timestamps"));
                }
            }
        }
        if self.body_finished_ns.is_some() && self.body_started_ns.is_none() {
            return Err(format!("row {id}: body finished without start"));
        }
        if let (Some(start), Some(finish)) = (self.body_started_ns, self.body_finished_ns) {
            if finish < start {
                return Err(format!("row {id}: body finish precedes start"));
            }
        }
        if let (Some(submit), Some(start)) = (self.submitted_ns, self.body_started_ns) {
            if start < submit {
                return Err(format!("row {id}: body start precedes submit"));
            }
        }
        if let (Some(submit), Some(response)) = (self.submitted_ns, self.caller_response_ns) {
            if response < submit {
                return Err(format!("row {id}: response precedes submit"));
            }
        }
        if let (Some(submit), Some(drop)) = (self.submitted_ns, self.caller_drop_ns) {
            if drop < submit {
                return Err(format!("row {id}: caller drop precedes submit"));
            }
        }
        if matches!(
            self.disposition,
            CallerDisposition::Responded {
                outcome: ResponseOutcome::Success
            }
        ) && self.body_finished_ns.is_none()
        {
            return Err(format!("row {id}: success without body finish"));
        }
        if let CallerDisposition::Responded { outcome } = &self.disposition {
            match outcome {
                ResponseOutcome::Success if self.body_finished_ns > self.caller_response_ns => {
                    return Err(format!("row {id}: success responds before body finish"));
                }
                ResponseOutcome::Rejected { .. } if self.body_started_ns.is_some() => {
                    return Err(format!("row {id}: rejected work started"));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClassCounters {
    pub admitted: u128,
    pub started: u128,
    pub terminated: u128,
    pub inflight: u32,
    pub queued: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SampledClass {
    pub inflight: u32,
    pub queued: u32,
    pub accepted: u32,
    pub running: u32,
    pub cpu_units_held: String,
    pub memory_units_held: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotSample {
    pub intended_ns: u64,
    pub observed_ns: u64,
    pub classes: BTreeMap<String, SampledClass>,
    pub capabilities: BTreeMap<String, u32>,
    pub conservation_ok: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawHostRun {
    pub schema_version: u32,
    pub status: HostRunStatus,
    pub scenario_id: String,
    pub injection_window_ns: u64,
    pub interval_ns: u64,
    pub snapshot_cadence_ns: u64,
    pub snapshots: Vec<SnapshotSample>,
    pub cut: CallerCounts,
    pub settlement: CallerCounts,
    pub records: Vec<RawHostRecord>,
    pub class_counters: BTreeMap<String, ClassCounters>,
    pub final_capabilities: BTreeMap<String, u32>,
    pub drain_ok: bool,
    pub conservation_ok: bool,
}

impl RawHostRun {
    pub fn validate(&self) -> Result<(), String> {
        if let HostRunStatus::Invalid { reason } = &self.status {
            return Err(format!("invalid host run: {reason}"));
        }
        if self.schema_version != HOST_RAW_VERSION
            || self.injection_window_ns == 0
            || self.interval_ns == 0
            || self.injection_window_ns % self.interval_ns != 0
        {
            return Err("invalid raw host schema/window".into());
        }
        if self.snapshot_cadence_ns == 0 {
            if !self.snapshots.is_empty() {
                return Err("snapshot samples exist with cadence disabled".into());
            }
        } else {
            if self.injection_window_ns % self.snapshot_cadence_ns != 0
                || self.snapshots.len()
                    != (self.injection_window_ns / self.snapshot_cadence_ns) as usize
            {
                return Err("snapshot cadence/sample population differs".into());
            }
            for (index, sample) in self.snapshots.iter().enumerate() {
                if sample.intended_ns != index as u64 * self.snapshot_cadence_ns
                    || sample.observed_ns < sample.intended_ns
                    || !sample.conservation_ok
                {
                    return Err(format!("snapshot sample {index} is invalid"));
                }
            }
        }
        let actual = CallerCounts::from_records(&self.records)?;
        if actual != self.settlement {
            return Err("settlement counts differ from raw rows".into());
        }
        if self.cut != CallerCounts::at_cut(&self.records, self.injection_window_ns)? {
            return Err("window-cut counts do not reconcile".into());
        }
        if actual.waiting != 0 {
            return Err("settlement contains unclassified waiting callers".into());
        }
        if self.drain_ok
            && (!self.conservation_ok
                || self
                    .class_counters
                    .values()
                    .any(|class| class.inflight != 0 || class.queued != 0)
                || self.final_capabilities.values().any(|in_use| *in_use != 0))
        {
            return Err("drained run still owns engine capacity".into());
        }
        Ok(())
    }

    pub fn validate_against(&self, scenario: &HostScenario) -> Result<(), String> {
        self.validate()?;
        scenario.validate()?;
        if self.scenario_id != scenario.id || self.records.len() != scenario.offers.len() {
            return Err("raw scenario identity or offer population differs".into());
        }
        if self.injection_window_ns != scenario.load.injection_ms * 1_000_000 {
            return Err("raw injection window differs from scenario".into());
        }
        if self.interval_ns != scenario.load.interval_ms * 1_000_000 {
            return Err("raw interval differs from scenario".into());
        }
        if self.snapshot_cadence_ns != scenario.load.snapshot_ms * 1_000_000 {
            return Err("raw snapshot cadence differs from scenario".into());
        }
        for (index, sample) in self.snapshots.iter().enumerate() {
            if sample.classes.len() != scenario.classes.len()
                || scenario
                    .classes
                    .iter()
                    .any(|class| !sample.classes.contains_key(&class.name))
            {
                return Err(format!("snapshot {index}: class catalog differs"));
            }
            if sample.classes.values().any(|class| {
                class.accepted.saturating_add(class.running) > class.inflight
                    || class.cpu_units_held.parse::<u128>().is_err()
                    || class.memory_units_held.parse::<u128>().is_err()
            }) {
                return Err(format!("snapshot {index}: invalid class gauges"));
            }
        }
        for (row, offer) in self.records.iter().zip(&scenario.offers) {
            if row.class != offer.class
                || row.path != offer.path
                || row.intended_ns != offer.send_time_ns
            {
                return Err(format!(
                    "row {}: offer identity differs from scenario",
                    row.id
                ));
            }
        }
        if self.class_counters.len() != scenario.classes.len()
            || scenario
                .classes
                .iter()
                .any(|class| !self.class_counters.contains_key(&class.name))
        {
            return Err("class counter population differs from scenario".into());
        }
        for class in &scenario.classes {
            let counters = &self.class_counters[&class.name];
            let class_rows = self.records.iter().filter(|row| row.class == class.name);
            let started_rows = class_rows
                .clone()
                .filter(|row| row.body_started_ns.is_some())
                .count() as u128;
            if counters.started != started_rows
                || counters.admitted < counters.started
                || counters.terminated > counters.admitted
            {
                return Err(format!(
                    "class {}: counters differ from raw rows",
                    class.name
                ));
            }
            if self.drain_ok && counters.terminated != counters.admitted {
                return Err(format!(
                    "class {}: drained admission did not settle",
                    class.name
                ));
            }
        }
        Ok(())
    }
}

pub(crate) type Slot = Arc<Mutex<RawHostRecord>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProducerDecision {
    Submit { lag_ns: u64 },
    NotSubmitted { lag_ns: u64 },
}

/// The pure pacing/cap decision. The runner injects the monotonic reading;
/// tests cover boundary behavior without scheduler-sensitive time assertions.
pub fn producer_decision(
    intended_ns: u64,
    observed_ns: u64,
    outstanding: usize,
    cap: usize,
) -> ProducerDecision {
    let lag_ns = observed_ns.saturating_sub(intended_ns);
    if outstanding >= cap {
        ProducerDecision::NotSubmitted { lag_ns }
    } else {
        ProducerDecision::Submit { lag_ns }
    }
}

pub(crate) fn since(origin: Instant) -> u64 {
    u64::try_from(origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// The only wall-clock pacing and cap decision used by host and null-work
/// controls. The absolute intended time remains the authority for both.
pub(crate) fn pace_offer(
    origin: Instant,
    intended_ns: u64,
    injection_ns: u64,
    outstanding: &AtomicUsize,
    cap: usize,
) -> (u64, ProducerDecision) {
    let due = origin + Duration::from_nanos(intended_ns);
    while let Some(delay) = due.checked_duration_since(Instant::now()) {
        if delay.is_zero() {
            break;
        }
        std::thread::sleep(delay);
    }
    let observed_ns = since(origin);
    let decision = if observed_ns >= injection_ns {
        ProducerDecision::NotSubmitted {
            lag_ns: observed_ns.saturating_sub(intended_ns),
        }
    } else {
        producer_decision(
            intended_ns,
            observed_ns,
            outstanding.load(Ordering::Acquire),
            cap,
        )
    };
    (observed_ns, decision)
}

fn mutate(slot: &Slot, f: impl FnOnce(&mut RawHostRecord)) {
    let mut row = slot.lock().expect("individual record lock");
    f(&mut row);
}

pub(crate) struct OutstandingGuard(pub(crate) Arc<AtomicUsize>);

impl Drop for OutstandingGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct CancelTimer(tokio::task::JoinHandle<()>);

impl Drop for CancelTimer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub fn classify_response(result: Result<(), RunError<()>>) -> ResponseOutcome {
    match result {
        Ok(()) => ResponseOutcome::Success,
        Err(RunError::Task(())) => ResponseOutcome::TaskError,
        Err(RunError::Governor(error)) => match error {
            taskmesh::GovernorError::Rejected(verdict) => ResponseOutcome::Rejected { verdict },
            taskmesh::GovernorError::DeadlineExceeded => ResponseOutcome::Deadline,
            taskmesh::GovernorError::Cancelled => ResponseOutcome::Cancelled,
            other => ResponseOutcome::GovernorError {
                error: format!("{other:?}"),
            },
        },
    }
}

fn options(offer: &HostOffer, cancel: CancellationToken) -> SubmitOptions {
    let mut opts = SubmitOptions::unbounded().with_cancel(cancel);
    if let Some(ms) = offer.deadline_ms {
        opts = opts.with_deadline(Duration::from_millis(ms));
    }
    opts
}

pub(crate) async fn execute(
    runtime: TokioRuntime,
    offer: HostOffer,
    slot: Option<Slot>,
    offer_id: usize,
    origin: Instant,
    submit_at: Instant,
    test_gate: Option<Arc<HostHarnessTestGate>>,
) -> Result<(), RunError<()>> {
    let class = TaskClass::new(offer.class.clone());
    let cancel = CancellationToken::new();
    let _cancel_timer = if let Some(ms) = offer.cancel_after_ms {
        let token = cancel.clone();
        Some(CancelTimer(tokio::spawn(async move {
            tokio::time::sleep_until(tokio::time::Instant::from_std(
                submit_at + Duration::from_millis(ms),
            ))
            .await;
            token.cancel();
        })))
    } else {
        None
    };
    let opts = options(&offer, cancel);
    let spec = match offer.path {
        HostPath::Io => TaskSpec::io(class),
        HostPath::Blocking => TaskSpec::blocking(class),
        HostPath::Cpu => TaskSpec::cpu(class),
        HostPath::RequestedStackBlocking => TaskSpec::blocking(class)
            .stack_size_bytes(offer.stack_size_bytes.expect("validated stack size")),
        HostPath::RequestedStackAsync => TaskSpec::base(class, SubstrateHint::LargeStackCapability)
            .stack_size_bytes(offer.stack_size_bytes.expect("validated stack size")),
    }
    .operation(format!("bench-{offer_id}"));
    match (offer.path, &offer.body) {
        (HostPath::Io, HostBody::Noop | HostBody::AsyncSleep { .. }) => {
            let sleep = match &offer.body {
                HostBody::AsyncSleep { millis } => *millis,
                _ => 0,
            };
            runtime
                .run_io_with(spec, opts, async move {
                    if let Some(slot) = &slot {
                        mutate(slot, |row| row.body_started_ns = Some(since(origin)));
                    }
                    if sleep != 0 {
                        tokio::time::sleep(Duration::from_millis(sleep)).await;
                    }
                    if let Some(slot) = &slot {
                        mutate(slot, |row| row.body_finished_ns = Some(since(origin)));
                    }
                    Ok(())
                })
                .await
        }
        (HostPath::Blocking, HostBody::Noop | HostBody::BlockingSleep { .. }) => {
            let sleep = match &offer.body {
                HostBody::BlockingSleep { millis } => *millis,
                _ => 0,
            };
            let gate = test_gate.filter(|gate| gate.offer_id == offer_id);
            runtime
                .run_blocking_with(spec, opts, move || {
                    if let Some(slot) = &slot {
                        mutate(slot, |row| row.body_started_ns = Some(since(origin)));
                    }
                    if let Some(gate) = gate {
                        gate.wait_for_release();
                    }
                    if sleep != 0 {
                        std::thread::sleep(Duration::from_millis(sleep));
                    }
                    if let Some(slot) = &slot {
                        mutate(slot, |row| row.body_finished_ns = Some(since(origin)));
                    }
                    Ok(())
                })
                .await
        }
        (HostPath::RequestedStackBlocking, HostBody::Noop | HostBody::BlockingSleep { .. }) => {
            let sleep = match &offer.body {
                HostBody::BlockingSleep { millis } => *millis,
                _ => 0,
            };
            runtime
                .run_blocking_with(spec, opts, move || {
                    if let Some(slot) = &slot {
                        mutate(slot, |row| row.body_started_ns = Some(since(origin)));
                    }
                    if sleep != 0 {
                        std::thread::sleep(Duration::from_millis(sleep));
                    }
                    if let Some(slot) = &slot {
                        mutate(slot, |row| row.body_finished_ns = Some(since(origin)));
                    }
                    Ok(())
                })
                .await
        }
        (HostPath::RequestedStackAsync, HostBody::Noop | HostBody::AsyncSleep { .. }) => {
            let sleep = match &offer.body {
                HostBody::AsyncSleep { millis } => *millis,
                _ => 0,
            };
            runtime
                .run_async_with_requested_stack_with(spec, opts, move || async move {
                    if let Some(slot) = &slot {
                        mutate(slot, |row| row.body_started_ns = Some(since(origin)));
                    }
                    if sleep != 0 {
                        tokio::time::sleep(Duration::from_millis(sleep)).await;
                    }
                    if let Some(slot) = &slot {
                        mutate(slot, |row| row.body_finished_ns = Some(since(origin)));
                    }
                    Ok(())
                })
                .await
        }
        (HostPath::Cpu, HostBody::Noop | HostBody::CpuSpin { .. }) => {
            let iterations = match &offer.body {
                HostBody::CpuSpin { iterations } => *iterations,
                _ => 0,
            };
            runtime
                .run_cpu_with(spec, opts, move || {
                    if let Some(slot) = &slot {
                        mutate(slot, |row| row.body_started_ns = Some(since(origin)));
                    }
                    let mut value = 0_u64;
                    for i in 0..iterations {
                        value = value.wrapping_add(black_box(i).rotate_left(7));
                    }
                    black_box(value);
                    if let Some(slot) = &slot {
                        mutate(slot, |row| row.body_finished_ns = Some(since(origin)));
                    }
                    Ok(())
                })
                .await
        }
        _ => unreachable!("scenario validation checks path/body"),
    }
}

fn snapshot_rows(slots: &[Slot]) -> Vec<RawHostRecord> {
    slots
        .iter()
        .map(|slot| slot.lock().expect("individual record lock").clone())
        .collect()
}

fn snapshot_sample(runtime: &TokioRuntime, origin: Instant, intended_ns: u64) -> SnapshotSample {
    let snapshot = runtime.snapshot();
    SnapshotSample {
        intended_ns,
        observed_ns: since(origin),
        classes: snapshot
            .classes
            .iter()
            .map(|(class, state)| {
                (
                    class.as_str().to_owned(),
                    SampledClass {
                        inflight: state.inflight,
                        queued: state.queued,
                        accepted: state.accepted,
                        running: state.running,
                        cpu_units_held: state.cpu_units_held.to_string(),
                        memory_units_held: state.memory_units_held.to_string(),
                    },
                )
            })
            .collect(),
        capabilities: snapshot
            .capabilities
            .iter()
            .map(|(pool, usage)| (pool.clone(), usage.in_use))
            .collect(),
        conservation_ok: snapshot.conservation_violation().is_none(),
    }
}

/// Run a finite, prevalidated public-host scenario. Results are diagnostic
/// until the external receipt checker and quiet-host calibration qualify them.
pub async fn run_host_scenario(scenario: &HostScenario) -> Result<RawHostRun, String> {
    run_host_scenario_with_fault(scenario, None).await
}

/// Fault injection is confined to the unpublished bench crate. It tests that
/// harness failures leave a bounded raw artifact rather than a silent gap.
pub async fn run_host_scenario_with_fault(
    scenario: &HostScenario,
    fault: Option<HostHarnessFault>,
) -> Result<RawHostRun, String> {
    run_host_scenario_with_topology(scenario, fault)
        .await
        .map(|(run, _)| run)
}

/// Return the installed topology of the same runtime that ran the workload.
pub async fn run_host_scenario_with_topology(
    scenario: &HostScenario,
    fault: Option<HostHarnessFault>,
) -> Result<(RawHostRun, ResolvedHostTopology), String> {
    run_host_scenario_inner(scenario, fault, None).await
}

/// Exercise row attribution with a controlled blocking worker. The gate is
/// bench-test code and cannot be supplied by a serialized workload.
pub async fn run_host_scenario_with_test_gate(
    scenario: &HostScenario,
    gate: Arc<HostHarnessTestGate>,
) -> Result<RawHostRun, String> {
    let offer = scenario
        .offers
        .get(gate.offer_id)
        .ok_or("test gate offer id is out of range")?;
    if offer.path != HostPath::Blocking || offer.drop_after_ms.is_none() {
        return Err("test gate requires a blocking offer with caller drop".into());
    }
    run_host_scenario_inner(scenario, None, Some(gate))
        .await
        .map(|(run, _)| run)
}

pub(crate) async fn warmup_runtime(
    scenario: &HostScenario,
    runtime: &TokioRuntime,
) -> Result<(), String> {
    if scenario.load.warmup_ms == 0 {
        return Ok(());
    }
    // Control and host warm the same class/path set before their measured cut.
    let mut warmed = Vec::new();
    for offer in &scenario.offers {
        if warmed.contains(&(offer.class.as_str(), offer.path, offer.stack_size_bytes)) {
            continue;
        }
        warmed.push((offer.class.as_str(), offer.path, offer.stack_size_bytes));
        let class = TaskClass::new(offer.class.clone());
        let result = match offer.path {
            HostPath::Io => {
                runtime
                    .run_io(TaskSpec::io(class).operation("bench-warmup"), async {
                        Ok::<(), ()>(())
                    })
                    .await
            }
            HostPath::Blocking => {
                runtime
                    .run_blocking(TaskSpec::blocking(class).operation("bench-warmup"), || {
                        Ok::<(), ()>(())
                    })
                    .await
            }
            HostPath::Cpu => {
                runtime
                    .run_cpu(TaskSpec::cpu(class).operation("bench-warmup"), || {
                        Ok::<(), ()>(())
                    })
                    .await
            }
            HostPath::RequestedStackBlocking => {
                runtime
                    .run_blocking(
                        TaskSpec::blocking(class)
                            .operation("bench-warmup")
                            .stack_size_bytes(
                                offer.stack_size_bytes.expect("validated stack size"),
                            ),
                        || Ok::<(), ()>(()),
                    )
                    .await
            }
            HostPath::RequestedStackAsync => {
                runtime
                    .run_async_with_requested_stack(
                        TaskSpec::base(class, SubstrateHint::LargeStackCapability)
                            .operation("bench-warmup")
                            .stack_size_bytes(
                                offer.stack_size_bytes.expect("validated stack size"),
                            ),
                        || async { Ok::<(), ()>(()) },
                    )
                    .await
            }
        };
        result.map_err(|error| format!("warmup failed: {error:?}"))?;
    }
    tokio::time::sleep(Duration::from_millis(scenario.load.warmup_ms)).await;
    Ok(())
}

async fn run_host_scenario_inner(
    scenario: &HostScenario,
    fault: Option<HostHarnessFault>,
    test_gate: Option<Arc<HostHarnessTestGate>>,
) -> Result<(RawHostRun, ResolvedHostTopology), String> {
    scenario.validate()?;
    let runtime = scenario.build_runtime()?;
    let resolved_topology = ResolvedHostTopology::from_runtime(&runtime);
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
        return Err("warmup did not settle to zero owned capacity".into());
    }
    let slots: Vec<Slot> = scenario
        .offers
        .iter()
        .enumerate()
        .map(|(id, offer)| {
            Arc::new(Mutex::new(RawHostRecord {
                id,
                class: offer.class.clone(),
                path: offer.path,
                intended_ns: offer.send_time_ns,
                scheduled_lag_ns: None,
                submitted_ns: None,
                body_started_ns: None,
                caller_response_ns: None,
                caller_drop_ns: None,
                body_finished_ns: None,
                disposition: CallerDisposition::Waiting,
            }))
        })
        .collect();
    let slots_for_producer = slots.clone();
    let offers = scenario.offers.clone();
    let outstanding = Arc::new(AtomicUsize::new(0));
    let outstanding_for_producer = Arc::clone(&outstanding);
    let handle = tokio::runtime::Handle::current();
    let max_outstanding = scenario.load.max_outstanding;
    let injection = Duration::from_millis(scenario.load.injection_ms);
    let producer_runtime = runtime.clone();
    let (producer_tx, producer_rx) = tokio::sync::oneshot::channel();
    let preallocated_jobs = Vec::with_capacity(offers.len());
    let origin = Instant::now();
    let snapshot_cadence_ns = scenario.load.snapshot_ms * 1_000_000;
    let sampler = if snapshot_cadence_ns == 0 {
        None
    } else {
        let sample_runtime = runtime.clone();
        let sample_count = scenario.load.injection_ms / scenario.load.snapshot_ms;
        Some(tokio::spawn(async move {
            if fault == Some(HostHarnessFault::SamplerPanic) {
                panic!("injected snapshot sampler failure");
            }
            let mut samples = Vec::with_capacity(sample_count as usize);
            for index in 0..sample_count {
                let intended_ns = index * snapshot_cadence_ns;
                tokio::time::sleep_until(tokio::time::Instant::from_std(
                    origin + Duration::from_nanos(intended_ns),
                ))
                .await;
                samples.push(snapshot_sample(&sample_runtime, origin, intended_ns));
            }
            samples
        }))
    };
    let producer = std::thread::spawn(move || {
        let mut jobs = preallocated_jobs;
        for (id, offer) in offers.into_iter().enumerate() {
            if fault == Some(HostHarnessFault::ProducerBeforeOffer(id)) {
                panic!("injected producer failure before offer {id}");
            }
            let slot = Arc::clone(&slots_for_producer[id]);
            let (_, decision) = pace_offer(
                origin,
                offer.send_time_ns,
                u64::try_from(injection.as_nanos()).unwrap_or(u64::MAX),
                &outstanding_for_producer,
                max_outstanding,
            );
            match decision {
                ProducerDecision::NotSubmitted { lag_ns } => {
                    mutate(&slot, |row| {
                        row.scheduled_lag_ns = Some(lag_ns);
                        row.disposition = CallerDisposition::NotSubmitted;
                    });
                    continue;
                }
                ProducerDecision::Submit { lag_ns } => {
                    mutate(&slot, |row| row.scheduled_lag_ns = Some(lag_ns));
                }
            }
            outstanding_for_producer.fetch_add(1, Ordering::AcqRel);
            let submit_ns = since(origin);
            let submit_at = origin + Duration::from_nanos(submit_ns);
            mutate(&slot, |row| row.submitted_ns = Some(submit_ns));
            let guard = OutstandingGuard(Arc::clone(&outstanding_for_producer));
            let runtime = producer_runtime.clone();
            let gate = test_gate.clone();
            jobs.push(handle.spawn(async move {
                let _guard = guard;
                let drop_after = offer.drop_after_ms;
                let gated_drop = gate
                    .as_ref()
                    .is_some_and(|gate| gate.offer_id == slot.lock().expect("record lock").id);
                let run = execute(
                    runtime,
                    offer,
                    Some(Arc::clone(&slot)),
                    id,
                    origin,
                    submit_at,
                    gate.clone(),
                );
                if let Some(ms) = drop_after {
                    let drop_gate = gate.clone();
                    tokio::select! {
                        result = run => {
                            mutate(&slot, |row| {
                                row.caller_response_ns = Some(since(origin));
                                row.disposition = CallerDisposition::Responded { outcome: classify_response(result) };
                            });
                        }
                        () = async move {
                            if gated_drop {
                                if let Some(gate) = drop_gate {
                                    gate.drop_now.cancelled().await;
                                }
                            } else {
                                tokio::time::sleep_until(
                                    tokio::time::Instant::from_std(submit_at + Duration::from_millis(ms))
                                ).await;
                            }
                        } => {
                            mutate(&slot, |row| {
                                row.caller_drop_ns = Some(since(origin));
                                row.disposition = CallerDisposition::CallerDropped;
                            });
                            if gated_drop {
                                if let Some(gate) = &gate {
                                    gate.caller_dropped.cancel();
                                }
                            }
                        }
                    }
                } else {
                    let result = run.await;
                    mutate(&slot, |row| {
                        row.caller_response_ns = Some(since(origin));
                        row.disposition = CallerDisposition::Responded { outcome: classify_response(result) };
                    });
                }
            }));
        }
        let cut = origin + injection;
        if let Some(delay) = cut.checked_duration_since(Instant::now()) {
            std::thread::sleep(delay);
        }
        let _ = producer_tx.send(jobs);
    });
    let mut issues = Vec::new();
    let mut jobs = match producer_rx.await {
        Ok(jobs) => jobs,
        Err(_) => {
            issues.push("producer failed before reporting jobs");
            Vec::new()
        }
    };
    if producer.join().is_err() {
        issues.push("producer thread panicked");
    }
    let snapshots = match sampler {
        Some(sampler) => match sampler.await {
            Ok(samples) => samples,
            Err(_) => {
                issues.push("snapshot sampler failed");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    let until = Instant::now() + Duration::from_millis(scenario.load.settlement_ms);
    while jobs.iter().any(|job| !job.is_finished()) && Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    for job in &mut jobs {
        if !job.is_finished() {
            job.abort();
        }
    }
    for job in jobs {
        if job.await.is_err() {
            issues.push("caller task failed or was aborted");
        }
    }
    for slot in &slots {
        mutate(slot, |row| {
            if row.submitted_ns.is_some() && row.disposition == CallerDisposition::Waiting {
                row.disposition = CallerDisposition::UnansweredAtSettlement;
            }
        });
    }
    // Keep malformed rows in the artifact. The public validator will reject
    // these fallback counts after the CLI has persisted the raw records.
    let settlement = CallerCounts::from_records(&snapshot_rows(&slots)).unwrap_or_else(|_| {
        issues.push("settlement rows failed accounting");
        CallerCounts::default()
    });
    let cut = CallerCounts::at_cut(
        &snapshot_rows(&slots),
        u64::try_from(injection.as_nanos()).unwrap_or(u64::MAX),
    )
    .unwrap_or_else(|_| {
        issues.push("window-cut rows failed accounting");
        CallerCounts::default()
    });
    let drain_ok = runtime
        .drain(Duration::from_millis(scenario.load.settlement_ms))
        .await
        .is_ok();
    let final_snapshot = runtime.snapshot();
    if !drain_ok {
        issues.push("host drain did not settle");
    }
    if final_snapshot.conservation_violation().is_some() {
        issues.push("final governor conservation failed");
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
    let run = RawHostRun {
        schema_version: HOST_RAW_VERSION,
        status: if issues.is_empty() {
            HostRunStatus::Complete
        } else {
            HostRunStatus::Invalid {
                reason: issues.join("; "),
            }
        },
        scenario_id: scenario.id.clone(),
        injection_window_ns: u64::try_from(injection.as_nanos()).unwrap_or(u64::MAX),
        interval_ns: scenario.load.interval_ms * 1_000_000,
        snapshot_cadence_ns,
        snapshots,
        cut,
        settlement,
        records: snapshot_rows(&slots),
        class_counters,
        final_capabilities: final_snapshot
            .capabilities
            .iter()
            .map(|(pool, usage)| (pool.clone(), usage.in_use))
            .collect(),
        drain_ok,
        conservation_ok: final_snapshot.conservation_violation().is_none(),
    };
    // Return the raw artifact even if its ledger is inconsistent. The CLI
    // persists it before reporting the validation error, so failed runs are
    // inspectable instead of disappearing behind an Err.
    Ok((run, resolved_topology))
}
