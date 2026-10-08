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
    pace_offer, remaining_injection_window, since, warmup_runtime, HostHarnessFault, HostRunStatus,
    ProducerDecision, ResolvedHostTopology,
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

fn settlement_deadline(start: Instant, settlement: Duration) -> Result<Instant, String> {
    start
        .checked_add(settlement)
        .ok_or_else(|| "generator settlement deadline overflowed".into())
}

struct AbortOnDropJobs(Vec<tokio::task::JoinHandle<()>>);

impl Drop for AbortOnDropJobs {
    fn drop(&mut self) {
        for job in &self.0 {
            job.abort();
        }
    }
}

async fn settle_null_jobs(mut jobs: AbortOnDropJobs, until: Instant) -> (usize, usize) {
    let mut completed = 0;
    let mut failed = 0;
    let deadline = tokio::time::Instant::from_std(until);
    // Keep abort authority for both the currently awaited job and every later
    // job if the caller drops this settlement future.
    while let Some(mut job) = jobs.0.pop() {
        let abort = job.abort_handle();
        struct AbortCurrent(tokio::task::AbortHandle);
        impl Drop for AbortCurrent {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _abort_on_drop = AbortCurrent(abort);
        let result = match tokio::time::timeout_at(deadline, &mut job).await {
            Ok(result) => result,
            Err(_) => {
                job.abort();
                // Await the abort so the outstanding guard has been dropped.
                job.await
            }
        };
        match result {
            Ok(()) => completed += 1,
            Err(_) => failed += 1,
        }
    }
    (completed, failed)
}

async fn wait_for_orphaned_guards(outstanding: &AtomicUsize, until: Instant) -> bool {
    let deadline = tokio::time::Instant::from_std(until);
    tokio::time::timeout_at(deadline, async {
        while outstanding.load(Ordering::Acquire) != 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .is_ok()
}

struct NullTaskGate {
    polled: tokio::sync::oneshot::Sender<()>,
    release: tokio::sync::oneshot::Receiver<()>,
    producer_resume: Option<std::sync::mpsc::Receiver<()>>,
    dropped: Option<Arc<tokio::sync::Notify>>,
    #[cfg(test)]
    suppress_transfer: bool,
}

struct DropWitness(Arc<tokio::sync::Notify>);

impl Drop for DropWitness {
    fn drop(&mut self) {
        self.0.notify_one();
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
    run_generator_control_inner(scenario, fault, None).await
}

async fn run_generator_control_inner(
    scenario: &HostScenario,
    fault: Option<HostHarnessFault>,
    gate: Option<NullTaskGate>,
) -> Result<(GeneratorRun, ResolvedHostTopology), String> {
    scenario.validate()?;
    if fault == Some(HostHarnessFault::SamplerPanic) {
        return Err("generator control has no Snapshot sampler".into());
    }
    let runtime = scenario.build_runtime()?;
    let topology = ResolvedHostTopology::from_runtime(&runtime);
    warmup_runtime(scenario, &runtime).await?;
    let baseline = runtime.snapshot();
    if !crate::host_load::warmup_snapshot_capacity_settled(&baseline) {
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
        let mut gate = gate;
        #[cfg(test)]
        let suppress_transfer = gate.as_ref().is_some_and(|gate| gate.suppress_transfer);
        let producer_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
                        let guard = OutstandingGuard(Arc::clone(&producer_outstanding));
                        let submit_ns = since(origin);
                        {
                            let mut row = slot.lock().expect("generator record lock");
                            row.submitted_ns = Some(submit_ns);
                            row.disposition = GeneratorDisposition::Submitted;
                        }
                        let task_runtime = producer_runtime.clone();
                        let mut task_gate = if id == 0 { gate.take() } else { None };
                        let producer_resume = task_gate
                            .as_mut()
                            .and_then(|gate| gate.producer_resume.take());
                        jobs.push(handle.spawn(async move {
                            let _drop_witness = task_gate
                                .as_ref()
                                .and_then(|gate| gate.dropped.as_ref())
                                .map(|signal| DropWitness(Arc::clone(signal)));
                            let _guard = guard;
                            if let Some(task_gate) = task_gate {
                                task_gate.polled.send(()).expect("gate observer is alive");
                                task_gate.release.await.expect("gate release remains live");
                            }
                            std::hint::black_box((id, offer, task_runtime));
                        }));
                        if let Some(resume) = producer_resume {
                            resume
                                .recv_timeout(Duration::from_secs(5))
                                .expect("producer test gate must resume");
                        }
                    }
                    ProducerDecision::NotSubmitted { lag_ns } => {
                        let mut row = slot.lock().expect("generator record lock");
                        row.observed_ns = Some(observed_ns);
                        row.scheduled_lag_ns = Some(lag_ns);
                        row.disposition = GeneratorDisposition::NotSubmitted;
                    }
                }
            }
            if let Some(delay) = remaining_injection_window(injection, origin.elapsed()) {
                std::thread::sleep(delay);
            }
        }));
        // Transfer every spawned handle even when a later offer panics. Dropping
        // the local Vec here would detach those jobs and lose abort authority.
        // The channel owns abort authority even if its receiver is canceled
        // after a successful send but before the payload is received.
        #[cfg(test)]
        if suppress_transfer {
            // Test-only simulation of an unreported producer task. The held
            // task remains live after its JoinHandle is dropped.
            return;
        }
        drop(producer_tx.send(AbortOnDropJobs(jobs)));
        if let Err(panic) = producer_result {
            std::panic::resume_unwind(panic);
        }
    });
    let mut issues = Vec::new();
    let (jobs, orphaned_jobs_possible) = match producer_rx.await {
        Ok(jobs) => (jobs, false),
        Err(_) => {
            issues.push("generator producer failed before reporting jobs");
            (AbortOnDropJobs(Vec::new()), true)
        }
    };
    if producer.join().is_err() {
        issues.push("generator producer panicked");
    }
    let until = settlement_deadline(
        Instant::now(),
        Duration::from_millis(scenario.load.settlement_ms),
    )?;
    let (completed, failed_jobs) = settle_null_jobs(jobs, until).await;
    issues.resize(
        issues.len() + failed_jobs,
        "generator task failed or was aborted",
    );
    if orphaned_jobs_possible {
        // Only a failure before handle transfer reaches this fallback. An
        // unresolved guard remains an explicit invalid custody diagnostic.
        if !wait_for_orphaned_guards(&outstanding, until).await {
            issues.push("generator orphaned task retained capacity");
        }
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

#[cfg(test)]
mod settlement_tests {
    use super::{
        run_generator_control_inner, settle_null_jobs, settlement_deadline,
        wait_for_orphaned_guards, AbortOnDropJobs, DropWitness, NullTaskGate, OutstandingGuard,
    };
    use crate::host_load::{HostHarnessFault, HostRunStatus};
    use crate::host_scenarios::HostScenario;
    use std::future::{poll_fn, Future};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::Poll;
    use std::time::{Duration, Instant};

    const HANG: Duration = Duration::from_secs(5);

    #[test]
    fn settlement_window_is_one_forward_absolute_deadline() {
        let start = Instant::now();
        let window = Duration::from_millis(500);
        let deadline = settlement_deadline(start, window).expect("bounded window fits Instant");
        assert_eq!(deadline.duration_since(start), window);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pending_null_job_is_aborted_awaited_and_releases_its_guard() {
        let outstanding = Arc::new(AtomicUsize::new(1));
        let owned = Arc::clone(&outstanding);
        let (polled_tx, polled_rx) = tokio::sync::oneshot::channel();
        let (_release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let job = tokio::spawn(async move {
            let _guard = OutstandingGuard(owned);
            polled_tx.send(()).expect("first-poll observer is alive");
            release_rx.await.expect("held job release remains live");
        });
        tokio::time::timeout(HANG, polled_rx)
            .await
            .expect("null job must start polling")
            .expect("null job first-poll signal must arrive");
        let (completed, failed) = tokio::time::timeout(
            HANG,
            settle_null_jobs(AbortOnDropJobs(vec![job]), Instant::now()),
        )
        .await
        .expect("aborted null job must settle");
        assert_eq!((completed, failed), (0, 1));
        assert_eq!(outstanding.load(Ordering::Acquire), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn completed_null_job_remains_successful() {
        let job = tokio::spawn(async {});
        let until = settlement_deadline(Instant::now(), HANG).expect("bounded window fits Instant");
        let result =
            tokio::time::timeout(HANG, settle_null_jobs(AbortOnDropJobs(vec![job]), until))
                .await
                .expect("completed null job must settle");
        assert_eq!(result, (1, 0));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn already_aborted_null_job_is_counted_as_failed() {
        let (polled, first_poll) = tokio::sync::oneshot::channel();
        let (release, held) = tokio::sync::oneshot::channel::<()>();
        let job = tokio::spawn(async move {
            polled.send(()).expect("first-poll observer remains live");
            held.await.expect("held release remains live");
        });
        tokio::time::timeout(HANG, first_poll)
            .await
            .expect("null job must start polling")
            .expect("first-poll observer remains live");
        job.abort();
        let until = settlement_deadline(Instant::now(), HANG).expect("bounded deadline fits");
        let result =
            tokio::time::timeout(HANG, settle_null_jobs(AbortOnDropJobs(vec![job]), until))
                .await
                .expect("aborted job must settle");
        assert_eq!(result, (0, 1));
        assert!(release.send(()).is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn detached_producer_job_keeps_orphan_guard_until_it_finishes() {
        let outstanding = Arc::new(AtomicUsize::new(1));
        let owned = Arc::clone(&outstanding);
        let (polled_tx, polled_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let job = tokio::spawn(async move {
            let _guard = OutstandingGuard(owned);
            polled_tx.send(()).expect("first-poll observer is alive");
            release_rx.await.expect("orphan release remains live");
        });
        tokio::time::timeout(HANG, polled_rx)
            .await
            .expect("orphan task must start polling")
            .expect("orphan first-poll signal must arrive");
        drop(job); // Producer unwind drops its JoinHandle, not the running task.
        assert_eq!(outstanding.load(Ordering::Acquire), 1);
        let release = tokio::spawn(async move {
            tokio::task::yield_now().await;
            release_tx.send(()).expect("orphan task is still waiting");
        });
        let until = settlement_deadline(Instant::now(), HANG).expect("bounded window fits Instant");
        assert!(
            tokio::time::timeout(HANG, wait_for_orphaned_guards(&outstanding, until))
                .await
                .expect("orphan wait must settle")
        );
        release.await.expect("orphan release task must finish");
        assert_eq!(outstanding.load(Ordering::Acquire), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unresolved_orphan_guard_reports_deadline_expiry() {
        let outstanding = Arc::new(AtomicUsize::new(1));
        let guard = OutstandingGuard(Arc::clone(&outstanding));
        assert!(!wait_for_orphaned_guards(&outstanding, Instant::now()).await);
        drop(guard);
        assert_eq!(outstanding.load(Ordering::Acquire), 0);
    }

    fn producer_failure_scenario() -> HostScenario {
        let fixture = serde_json::json!({
            "schema_version": 1,
            "id": "generator-panic-custody",
            "load": {"warmup_ms": 5, "injection_ms": 100, "interval_ms": 10,
                     "snapshot_ms": 0, "settlement_ms": 100,
                     "max_outstanding": 2, "max_records": 2},
            "topology": {"cpu_workers": 2, "blocking_threads": 2,
                         "shared_blocking_limit": 2, "cpu_units": 2, "memory_units": 2},
            "classes": [{"name": "interactive", "slo_ms": 100, "max_inflight": 2,
                         "max_queue_depth": 2, "cpu_units": 1, "memory_units": 1,
                         "overflow": "queue_within_depth"}],
            "offers": [
                {"send_time_ns": 0, "class": "interactive", "path": "io",
                 "body": {"kind": "noop"}},
                {"send_time_ns": 10_000_000, "class": "interactive", "path": "io",
                 "body": {"kind": "noop"}}
            ]
        });
        HostScenario::from_json(&serde_json::to_vec(&fixture).expect("fixture encodes"))
            .expect("producer failure fixture is valid")
    }

    async fn producer_panic_keeps_first_job_owned(release_before_deadline: bool) {
        let scenario = producer_failure_scenario();
        let (polled, first_poll) = tokio::sync::oneshot::channel();
        let (release, held) = tokio::sync::oneshot::channel();
        let (resume, producer_resume) = std::sync::mpsc::channel();
        let dropped = Arc::new(tokio::sync::Notify::new());
        let task_dropped = Arc::clone(&dropped);
        let mut release = Some(release);
        let run = tokio::spawn(async move {
            run_generator_control_inner(
                &scenario,
                Some(HostHarnessFault::ProducerBeforeOffer(1)),
                Some(NullTaskGate {
                    polled,
                    release: held,
                    producer_resume: Some(producer_resume),
                    dropped: Some(task_dropped),
                    suppress_transfer: false,
                }),
            )
            .await
        });
        tokio::time::timeout(HANG, first_poll)
            .await
            .expect("first null task must start polling")
            .expect("first-poll observer must receive signal");
        if release_before_deadline {
            release
                .take()
                .expect("release sender exists")
                .send(())
                .expect("first null task remains held");
            tokio::time::timeout(HANG, dropped.notified())
                .await
                .expect("first null task must complete before producer resumes")
        }
        resume.send(()).expect("producer remains held at gate");
        let (raw, _) = tokio::time::timeout(HANG, run)
            .await
            .expect("producer failure must settle within its bound")
            .expect("generator task must return")
            .expect("generator run must return diagnostic raw");
        assert!(matches!(raw.status, HostRunStatus::Invalid { .. }));
        assert_eq!(raw.submitted, 1);
        assert_eq!(raw.outstanding_final, 0);
        assert_eq!(raw.completed, usize::from(release_before_deadline));
        drop(release);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn producer_panic_joins_first_job_when_released_before_deadline() {
        producer_panic_keeps_first_job_owned(true).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn producer_panic_aborts_first_job_at_deadline() {
        producer_panic_keeps_first_job_owned(false).await;
    }

    async fn unreported_producer_job_has_bounded_diagnostic(settled_before_transfer: bool) {
        let mut scenario = producer_failure_scenario();
        scenario.offers.truncate(1);
        scenario
            .validate()
            .expect("single-offer fixture remains valid");
        let (polled, first_poll) = tokio::sync::oneshot::channel();
        let (release, held) = tokio::sync::oneshot::channel();
        let (resume, producer_resume) = std::sync::mpsc::channel();
        let dropped = Arc::new(tokio::sync::Notify::new());
        let task_dropped = Arc::clone(&dropped);
        let mut release = Some(release);
        let run = tokio::spawn(async move {
            run_generator_control_inner(
                &scenario,
                None,
                Some(NullTaskGate {
                    polled,
                    release: held,
                    producer_resume: Some(producer_resume),
                    dropped: Some(task_dropped),
                    suppress_transfer: true,
                }),
            )
            .await
        });
        tokio::time::timeout(HANG, first_poll)
            .await
            .expect("unreported null task must start polling")
            .expect("first-poll observer remains live");
        if settled_before_transfer {
            release
                .take()
                .expect("release sender exists")
                .send(())
                .expect("null task is still held");
            tokio::time::timeout(HANG, dropped.notified())
                .await
                .expect("null task must settle before producer transfer");
        }
        resume.send(()).expect("producer remains at transfer gate");
        let (raw, _) = tokio::time::timeout(HANG, run)
            .await
            .expect("unreported producer must finish within bound")
            .expect("generator task must return")
            .expect("generator run must return diagnostic raw");
        if !settled_before_transfer {
            release
                .take()
                .expect("release sender exists")
                .send(())
                .expect("orphan task is still held after invalid response");
            tokio::time::timeout(HANG, dropped.notified())
                .await
                .expect("orphan task must settle after release");
        }
        drop(release);
        let reason = if settled_before_transfer {
            "generator producer failed before reporting jobs"
        } else {
            "generator producer failed before reporting jobs; generator orphaned task retained capacity"
        };
        assert_eq!(
            raw.status,
            HostRunStatus::Invalid {
                reason: reason.into()
            }
        );
        assert_eq!(raw.submitted, 1);
        assert_eq!(raw.completed, 0);
        assert_eq!(raw.outstanding_final, usize::from(!settled_before_transfer));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unreported_producer_settled_guard_has_no_orphan_diagnostic() {
        unreported_producer_job_has_bounded_diagnostic(true).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unreported_producer_held_guard_reports_orphan_diagnostic() {
        unreported_producer_job_has_bounded_diagnostic(false).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canceled_settlement_aborts_current_and_remaining_jobs() {
        let outstanding = Arc::new(AtomicUsize::new(2));
        let dropped = Arc::new(tokio::sync::Notify::new());
        let mut jobs = Vec::new();
        let mut polled = Vec::new();
        let mut releases = Vec::new();
        for _ in 0..2 {
            let owned = Arc::clone(&outstanding);
            let drop_signal = Arc::clone(&dropped);
            let (polled_tx, polled_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
            jobs.push(tokio::spawn(async move {
                let _witness = DropWitness(drop_signal);
                let _guard = OutstandingGuard(owned);
                polled_tx
                    .send(())
                    .expect("first-poll observer remains live");
                release_rx.await.expect("held release remains live");
            }));
            polled.push(polled_rx);
            releases.push(release_tx);
        }
        for started in polled {
            tokio::time::timeout(HANG, started)
                .await
                .expect("both null jobs must start polling")
                .expect("first-poll observer remains live");
        }
        let until = settlement_deadline(Instant::now(), HANG).expect("bounded deadline fits");
        let mut settlement = Box::pin(settle_null_jobs(AbortOnDropJobs(jobs), until));
        poll_fn(|cx| {
            assert!(settlement.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(settlement);
        tokio::time::timeout(HANG, async {
            while outstanding.load(Ordering::Acquire) != 0 {
                dropped.notified().await;
            }
        })
        .await
        .expect("both null jobs must terminate after abort requests");
        drop(releases);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canceled_receiver_aborts_producer_owned_job() {
        let scenario = producer_failure_scenario();
        let (polled, first_poll) = tokio::sync::oneshot::channel();
        let (release, held) = tokio::sync::oneshot::channel();
        let (resume, producer_resume) = std::sync::mpsc::channel();
        let dropped = Arc::new(tokio::sync::Notify::new());
        let run = tokio::spawn({
            let dropped = Arc::clone(&dropped);
            async move {
                run_generator_control_inner(
                    &scenario,
                    None,
                    Some(NullTaskGate {
                        polled,
                        release: held,
                        producer_resume: Some(producer_resume),
                        dropped: Some(dropped),
                        suppress_transfer: false,
                    }),
                )
                .await
            }
        });
        tokio::time::timeout(HANG, first_poll)
            .await
            .expect("first null job must start polling")
            .expect("first-poll observer remains live");
        run.abort();
        let failure = tokio::time::timeout(HANG, run)
            .await
            .expect("caller cancellation must resolve")
            .expect_err("caller must be canceled");
        assert!(failure.is_cancelled());
        resume
            .send(())
            .expect("producer remains held until receiver cancellation");
        tokio::time::timeout(HANG, dropped.notified())
            .await
            .expect("closed receiver must abort the producer-owned job");
        assert!(release.send(()).is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn receiver_dropped_after_successful_transfer_aborts_queued_job() {
        let outstanding = Arc::new(AtomicUsize::new(1));
        let dropped = Arc::new(tokio::sync::Notify::new());
        let (polled, first_poll) = tokio::sync::oneshot::channel();
        let (release, held) = tokio::sync::oneshot::channel::<()>();
        let owned = Arc::clone(&outstanding);
        let drop_signal = Arc::clone(&dropped);
        let job = tokio::spawn(async move {
            let _witness = DropWitness(drop_signal);
            let _guard = OutstandingGuard(owned);
            polled.send(()).expect("first-poll observer remains live");
            held.await.expect("held release remains live");
        });
        tokio::time::timeout(HANG, first_poll)
            .await
            .expect("queued job must start polling")
            .expect("first-poll observer remains live");
        let (sender, receiver) = tokio::sync::oneshot::channel();
        sender
            .send(AbortOnDropJobs(vec![job]))
            .map_err(drop)
            .expect("channel receiver remains live at send");
        drop(receiver);
        tokio::time::timeout(HANG, dropped.notified())
            .await
            .expect("dropped channel payload must abort the queued job");
        assert_eq!(outstanding.load(Ordering::Acquire), 0);
        assert!(release.send(()).is_err());
    }
}
