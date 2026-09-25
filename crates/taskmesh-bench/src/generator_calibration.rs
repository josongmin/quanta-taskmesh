//! Generator-only headroom control for the finite host arrival schedule.
//!
//! This measures the pacing/submission path without Taskmesh work. It cannot
//! alone qualify host latency: observer cost and A/A remain separate controls.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use taskmesh::Runtime;

use crate::host_load::{
    pace_offer, since, warmup_runtime, HostHarnessFault, HostRunStatus, ProducerDecision,
    ResolvedHostTopology,
};
use crate::host_scenarios::HostScenario;

pub const GENERATOR_RAW_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneratorDisposition {
    Pending,
    Submitted,
    NotSubmitted,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratorRecord {
    pub id: usize,
    pub intended_ns: u64,
    pub observed_ns: Option<u64>,
    pub scheduled_lag_ns: Option<u64>,
    pub submitted_ns: Option<u64>,
    pub disposition: GeneratorDisposition,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratorRun {
    pub schema_version: u32,
    pub status: HostRunStatus,
    pub scenario_id: String,
    pub injection_window_ns: u64,
    pub max_outstanding: usize,
    pub submitted: usize,
    pub not_submitted: usize,
    pub completed: usize,
    pub outstanding_final: usize,
    pub drain_ok: bool,
    pub conservation_ok: bool,
    pub records: Vec<GeneratorRecord>,
}

impl GeneratorRun {
    pub fn validate_against(&self, scenario: &HostScenario) -> Result<(), String> {
        scenario.validate()?;
        if self.status != HostRunStatus::Complete {
            return Err("generator run is invalid".into());
        }
        if self.schema_version != GENERATOR_RAW_VERSION
            || self.scenario_id != scenario.id
            || self.injection_window_ns != scenario.load.injection_ms * 1_000_000
            || self.max_outstanding != scenario.load.max_outstanding
            || self.records.len() != scenario.offers.len()
        {
            return Err("generator scenario or record population differs".into());
        }
        let mut submitted = 0;
        let mut not_submitted = 0;
        for (id, (row, offer)) in self.records.iter().zip(&scenario.offers).enumerate() {
            let observed_ns = row
                .observed_ns
                .ok_or(format!("generator row {id} was not paced"))?;
            if row.id != id
                || row.intended_ns != offer.send_time_ns
                || observed_ns < row.intended_ns
                || row.scheduled_lag_ns != Some(observed_ns - row.intended_ns)
            {
                return Err(format!("generator row {id} differs from intended schedule"));
            }
            match row.disposition {
                GeneratorDisposition::Submitted => {
                    if observed_ns >= self.injection_window_ns
                        || !row.submitted_ns.is_some_and(|submit| {
                            submit >= observed_ns && submit < self.injection_window_ns
                        })
                    {
                        return Err(format!("generator row {id} submitted after cut"));
                    }
                    submitted += 1;
                }
                GeneratorDisposition::NotSubmitted => {
                    if row.submitted_ns.is_some() {
                        return Err(format!("generator row {id} has impossible submit"));
                    }
                    not_submitted += 1;
                }
                GeneratorDisposition::Pending => {
                    return Err(format!("generator row {id} is still pending"));
                }
            }
        }
        if self.submitted != submitted
            || self.not_submitted != not_submitted
            || self.submitted + self.not_submitted != self.records.len()
            || self.completed != self.submitted
            || self.outstanding_final != 0
            || !self.drain_ok
            || !self.conservation_ok
        {
            return Err("generator conservation or settlement failed".into());
        }
        Ok(())
    }
}

struct OutstandingGuard(Arc<AtomicUsize>);

impl Drop for OutstandingGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

pub async fn run_generator_control(scenario: &HostScenario) -> Result<GeneratorRun, String> {
    run_generator_control_with_topology(scenario, None)
        .await
        .map(|(run, _)| run)
}

/// Run the same finite pacer with a null Taskmesh sink. A fast control is a
/// necessary producer check, not sufficient host-performance calibration.
pub async fn run_generator_control_with_topology(
    scenario: &HostScenario,
    fault: Option<HostHarnessFault>,
) -> Result<(GeneratorRun, ResolvedHostTopology), String> {
    scenario.validate()?;
    if fault == Some(HostHarnessFault::SamplerPanic) {
        return Err("generator control has no Snapshot sampler".into());
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
        return Err("generator warmup did not settle".into());
    }
    let offers = scenario.offers.clone();
    let injection = Duration::from_millis(scenario.load.injection_ms);
    let cap = scenario.load.max_outstanding;
    let slots: Vec<_> = scenario
        .offers
        .iter()
        .enumerate()
        .map(|(id, offer)| {
            Arc::new(Mutex::new(GeneratorRecord {
                id,
                intended_ns: offer.send_time_ns,
                observed_ns: None,
                scheduled_lag_ns: None,
                submitted_ns: None,
                disposition: GeneratorDisposition::Pending,
            }))
        })
        .collect();
    let producer_slots = slots.clone();
    let outstanding = Arc::new(AtomicUsize::new(0));
    let producer_outstanding = Arc::clone(&outstanding);
    let handle = tokio::runtime::Handle::current();
    let producer_runtime = runtime.clone();
    let origin = Instant::now();
    let (producer_tx, producer_rx) = tokio::sync::oneshot::channel();
    let producer = std::thread::spawn(move || {
        let mut jobs = Vec::with_capacity(offers.len());
        for (id, offer) in offers.into_iter().enumerate() {
            if fault == Some(HostHarnessFault::ProducerBeforeOffer(id)) {
                panic!("injected generator producer failure before offer {id}");
            }
            let slot = Arc::clone(&producer_slots[id]);
            let (observed_ns, decision) = pace_offer(
                origin,
                offer.send_time_ns,
                u64::try_from(injection.as_nanos()).unwrap_or(u64::MAX),
                &producer_outstanding,
                cap,
            );
            match decision {
                ProducerDecision::Submit { lag_ns } => {
                    {
                        let mut row = slot.lock().expect("generator record lock");
                        row.observed_ns = Some(observed_ns);
                        row.scheduled_lag_ns = Some(lag_ns);
                    }
                    producer_outstanding.fetch_add(1, Ordering::AcqRel);
                    let submit_ns = since(origin);
                    {
                        let mut row = slot.lock().expect("generator record lock");
                        row.submitted_ns = Some(submit_ns);
                        row.disposition = GeneratorDisposition::Submitted;
                    }
                    let guard = OutstandingGuard(Arc::clone(&producer_outstanding));
                    let task_runtime = producer_runtime.clone();
                    jobs.push(handle.spawn(async move {
                        let _guard = guard;
                        std::hint::black_box((id, offer, task_runtime));
                    }));
                }
                ProducerDecision::NotSubmitted { lag_ns } => {
                    let mut row = slot.lock().expect("generator record lock");
                    row.observed_ns = Some(observed_ns);
                    row.scheduled_lag_ns = Some(lag_ns);
                    row.disposition = GeneratorDisposition::NotSubmitted;
                }
            }
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
            issues.push("generator producer failed before reporting jobs");
            Vec::new()
        }
    };
    if producer.join().is_err() {
        issues.push("generator producer panicked");
    }
    let until = Instant::now() + Duration::from_millis(scenario.load.settlement_ms);
    let mut completed = 0;
    while jobs.iter().any(|job| !job.is_finished()) && Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    for job in &mut jobs {
        if !job.is_finished() {
            job.abort();
        }
    }
    for job in jobs {
        match job.await {
            Ok(()) => completed += 1,
            Err(_) => issues.push("generator task failed or was aborted"),
        }
    }
    while outstanding.load(Ordering::Acquire) != 0 && Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let drain_ok = runtime
        .drain(Duration::from_millis(scenario.load.settlement_ms))
        .await
        .is_ok();
    let final_snapshot = runtime.snapshot();
    let conservation_ok = final_snapshot.conservation_violation().is_none()
        && final_snapshot
            .classes
            .values()
            .all(|class| class.inflight == 0 && class.queued == 0)
        && final_snapshot
            .capabilities
            .values()
            .all(|usage| usage.in_use == 0);
    if !drain_ok {
        issues.push("generator runtime did not drain");
    }
    if !conservation_ok {
        issues.push("generator runtime retained capacity");
    }
    let records: Vec<_> = slots
        .iter()
        .map(|slot| slot.lock().expect("generator record lock").clone())
        .collect();
    let submitted = records
        .iter()
        .filter(|row| row.disposition == GeneratorDisposition::Submitted)
        .count();
    let run = GeneratorRun {
        schema_version: GENERATOR_RAW_VERSION,
        status: if issues.is_empty() {
            HostRunStatus::Complete
        } else {
            HostRunStatus::Invalid {
                reason: issues.join("; "),
            }
        },
        scenario_id: scenario.id.clone(),
        injection_window_ns: scenario.load.injection_ms * 1_000_000,
        max_outstanding: cap,
        submitted,
        not_submitted: records.len() - submitted,
        completed,
        outstanding_final: outstanding.load(Ordering::Acquire),
        drain_ok,
        conservation_ok,
        records,
    };
    Ok((run, topology))
}
