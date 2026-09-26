//! Same-host recovery diagnostics. Intermediate checkpoints do not close admission.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use taskmesh::{Runtime, Snapshot, TaskClass, TaskSpec, TokioRuntime};

use crate::host_load::{
    classify_response, run_host_window, warmup_runtime, RawHostRun, ResolvedHostTopology,
    ResponseOutcome,
};
use crate::host_scenarios::{HostPath, HostScenario};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StabilityManifest {
    pub schema_version: u32,
    pub scenario: HostScenario,
    pub cycles: u32,
    pub min_duration_ms: u64,
    pub max_total_records: u64,
}

impl StabilityManifest {
    pub fn validate(&self) -> Result<(), String> {
        self.scenario.validate()?;
        let paths = canary_paths(&self.scenario)?;
        let records = u64::from(self.cycles)
            .checked_mul((self.scenario.offers.len() + paths.len()) as u64)
            .ok_or("stability record count overflow")?;
        let samples = self
            .scenario
            .load
            .injection_ms
            .checked_div(self.scenario.load.snapshot_ms)
            .unwrap_or(0);
        if self.schema_version != 1
            || !(1..=10_000).contains(&self.cycles)
            || self.min_duration_ms > 1_200_000
            || self.max_total_records == 0
            || self.max_total_records > 1_000_000
            || records > self.max_total_records
            || u64::from(self.cycles) * (samples + 3) + 2 > 1_000_000
        {
            return Err("invalid stability version, duration, cycle or record budget".into());
        }
        Ok(())
    }
}

fn canary_paths(scenario: &HostScenario) -> Result<Vec<(String, HostPath)>, String> {
    let mut paths = Vec::new();
    let mut seen = BTreeSet::new();
    for offer in &scenario.offers {
        if !matches!(
            offer.path,
            HostPath::Io | HostPath::Blocking | HostPath::Cpu
        ) {
            return Err("stability diagnostic supports IO/blocking/CPU only".into());
        }
        if seen.insert((offer.class.as_str(), offer.path)) {
            paths.push((offer.class.clone(), offer.path));
        }
    }
    Ok(paths)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Canary {
    pub class: String,
    pub path: HostPath,
    pub body_ran: bool,
    pub outcome: ResponseOutcome,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StabilityCycle {
    pub schema_version: u32,
    pub index: u32,
    pub before: Snapshot,
    pub window: RawHostRun,
    pub after_window: Snapshot,
    pub canaries: Vec<Canary>,
    pub after_canaries: Snapshot,
    pub admission_open: bool,
}

fn zero_owned(snapshot: &Snapshot, scenario: &HostScenario) -> Result<(), String> {
    if snapshot.schema_version != taskmesh_contract::SNAPSHOT_SCHEMA_VERSION
        || snapshot.conservation_violation().is_some()
        || snapshot.classes.len() != scenario.classes.len()
        || scenario.classes.iter().any(|class| {
            !snapshot
                .classes
                .contains_key(&TaskClass::new(class.name.clone()))
        })
        || snapshot.classes.values().any(|class| {
            class.inflight != 0
                || class.queued != 0
                || class.cpu_units_held != 0
                || class.memory_units_held != 0
        })
        || snapshot.capabilities.is_empty()
        || snapshot
            .capabilities
            .values()
            .any(|usage| usage.in_use != 0)
    {
        return Err("checkpoint has invalid catalog, conservation or live owned work".into());
    }
    Ok(())
}

impl StabilityCycle {
    pub fn validate(
        &self,
        manifest: &StabilityManifest,
        previous: &Snapshot,
        index: u32,
    ) -> Result<(), String> {
        let topology = manifest.scenario.resolved_topology()?;
        self.validate_with_topology(manifest, previous, index, &topology)
    }

    /// Replay callers resolve this oracle once from the manifest. Online callers
    /// use the installed topology from their single measured runtime.
    pub fn validate_with_topology(
        &self,
        manifest: &StabilityManifest,
        previous: &Snapshot,
        index: u32,
        topology: &ResolvedHostTopology,
    ) -> Result<(), String> {
        manifest.validate()?;
        if self.schema_version != 1
            || self.index != index
            || self.before != *previous
            || !self.admission_open
        {
            return Err("cycle sequence, host continuity or admission differs".into());
        }
        self.window
            .validate_window_with_topology(&manifest.scenario, topology)?;
        if self.window.settlement.unanswered_at_settlement != 0 {
            return Err("cycle has unanswered callers".into());
        }
        for state in [&self.before, &self.after_window, &self.after_canaries] {
            zero_owned(state, &manifest.scenario)?;
            if state.capabilities != previous.capabilities
                || state.substrates != previous.substrates
            {
                return Err("checkpoint inventory changed".into());
            }
        }
        let paths = canary_paths(&manifest.scenario)?;
        if self.canaries.len() != paths.len()
            || self
                .canaries
                .iter()
                .zip(&paths)
                .any(|(row, (class, path))| {
                    row.class != *class
                        || row.path != *path
                        || !row.body_ran
                        || row.outcome != ResponseOutcome::Success
                })
        {
            return Err("canary population or execution failed".into());
        }
        for class in &manifest.scenario.classes {
            let key = TaskClass::new(class.name.clone());
            let before = &self.before.classes[&key];
            let window = &self.after_window.classes[&key];
            let after = &self.after_canaries.classes[&key];
            let delta = &self.window.class_counters[&class.name];
            let count = self
                .canaries
                .iter()
                .filter(|row| row.class == class.name)
                .count() as u128;
            if before.admitted_total.checked_add(delta.admitted) != Some(window.admitted_total)
                || before.started_total.checked_add(delta.started) != Some(window.started_total)
                || before.terminated_total.checked_add(delta.terminated)
                    != Some(window.terminated_total)
                || window.admitted_total.checked_add(count) != Some(after.admitted_total)
                || window.started_total.checked_add(count) != Some(after.started_total)
                || window.terminated_total.checked_add(count) != Some(after.terminated_total)
            {
                return Err("cumulative host counters reset or differ from window/canaries".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StabilitySummary {
    pub schema_version: u32,
    pub pid: u32,
    pub host_builds: u32,
    pub expected_cycles: u32,
    pub completed_cycles: u32,
    pub baseline: Snapshot,
    pub final_snapshot: Snapshot,
    pub final_draining: bool,
    pub drain_ok: bool,
    pub elapsed_ns: u64,
    pub topology: ResolvedHostTopology,
    pub reason: Option<String>,
}

impl StabilitySummary {
    pub fn validate(&self, manifest: &StabilityManifest, last: &Snapshot) -> Result<(), String> {
        let topology = manifest.scenario.resolved_topology()?;
        self.validate_with_topology(manifest, last, &topology)
    }

    pub fn validate_with_topology(
        &self,
        manifest: &StabilityManifest,
        last: &Snapshot,
        expected: &ResolvedHostTopology,
    ) -> Result<(), String> {
        manifest.validate()?;
        if self.schema_version != 1
            || self.pid == 0
            || self.host_builds != 1
            || self.expected_cycles != manifest.cycles
            || self.completed_cycles != manifest.cycles
            || !self.final_draining
            || !self.drain_ok
            || self.reason.is_some()
            || self.final_snapshot != *last
            || self.elapsed_ns < manifest.min_duration_ms * 1_000_000
        {
            return Err("incomplete or failed stability execution".into());
        }
        zero_owned(&self.baseline, &manifest.scenario)?;
        zero_owned(last, &manifest.scenario)?;
        if self.topology != *expected {
            return Err("stability topology differs".into());
        }
        for state in [&self.baseline, last] {
            let usage: BTreeMap<_, _> = state
                .capabilities
                .iter()
                .map(|(key, value)| (key.clone(), value.in_use))
                .collect();
            self.topology
                .validate_capability_usage(&usage, "stability checkpoint")?;
            if state
                .capabilities
                .iter()
                .any(|(key, value)| value.limit != self.topology.capability_limits[key])
            {
                return Err("stability capability limit differs".into());
            }
        }
        Ok(())
    }
}

async fn canary(runtime: &TokioRuntime, class: String, path: HostPath, budget: Duration) -> Canary {
    let ran = Arc::new(AtomicBool::new(false));
    let body_ran = ran.clone();
    let task_class = TaskClass::new(class.clone());
    let call = async {
        match path {
            HostPath::Io => {
                runtime
                    .run_io(
                        TaskSpec::io(task_class).operation("stability-canary"),
                        async move {
                            body_ran.store(true, Ordering::SeqCst);
                            Ok::<(), ()>(())
                        },
                    )
                    .await
            }
            HostPath::Blocking => {
                runtime
                    .run_blocking(
                        TaskSpec::blocking(task_class).operation("stability-canary"),
                        move || {
                            body_ran.store(true, Ordering::SeqCst);
                            Ok::<(), ()>(())
                        },
                    )
                    .await
            }
            HostPath::Cpu => {
                runtime
                    .run_cpu(
                        TaskSpec::cpu(task_class).operation("stability-canary"),
                        move || {
                            body_ran.store(true, Ordering::SeqCst);
                            Ok::<(), ()>(())
                        },
                    )
                    .await
            }
            _ => unreachable!("validated stability path"),
        }
    };
    let outcome = match tokio::time::timeout(budget, call).await {
        Ok(result) => classify_response(result),
        Err(_) => ResponseOutcome::Deadline,
    };
    Canary {
        class,
        path,
        body_ran: ran.load(Ordering::SeqCst),
        outcome,
    }
}

/// One runtime, one warmup; each emitted cycle can be flushed and dropped.
pub async fn run_stability(
    manifest: &StabilityManifest,
    mut emit: impl FnMut(&StabilityCycle) -> Result<(), String>,
) -> Result<StabilitySummary, String> {
    manifest.validate()?;
    let runtime = manifest.scenario.build_runtime()?;
    run_stability_on_runtime(manifest, &runtime, &mut emit).await
}

async fn run_stability_on_runtime(
    manifest: &StabilityManifest,
    runtime: &TokioRuntime,
    emit: &mut impl FnMut(&StabilityCycle) -> Result<(), String>,
) -> Result<StabilitySummary, String> {
    let result = run_stability_loop(manifest, runtime, emit).await;
    if let Err(error) = result {
        // Writer, warmup and window errors must still close admission and settle
        // owned work. Preserve the original failure if bounded cleanup succeeds.
        return match runtime
            .drain(Duration::from_millis(manifest.scenario.load.settlement_ms))
            .await
        {
            Ok(_) => Err(error),
            Err(cleanup) => Err(format!(
                "{error}; stability cleanup drain failed: {cleanup:?}"
            )),
        };
    }
    result
}

async fn run_stability_loop(
    manifest: &StabilityManifest,
    runtime: &TokioRuntime,
    emit: &mut impl FnMut(&StabilityCycle) -> Result<(), String>,
) -> Result<StabilitySummary, String> {
    warmup_runtime(&manifest.scenario, runtime).await?;
    let baseline = runtime.snapshot();
    zero_owned(&baseline, &manifest.scenario)?;
    let topology = ResolvedHostTopology::from_runtime(runtime);
    let paths = canary_paths(&manifest.scenario)?;
    let mut previous = baseline.clone();
    let mut reason = None;
    let mut completed = 0;
    let origin = Instant::now();
    for index in 0..manifest.cycles {
        let (window, _) = run_host_window(&manifest.scenario, runtime, None, None, false).await?;
        let after_window = runtime.snapshot();
        let mut canaries = Vec::new();
        if window
            .validate_window_with_topology(&manifest.scenario, &topology)
            .is_ok()
        {
            for (class, path) in &paths {
                canaries.push(
                    canary(
                        runtime,
                        class.clone(),
                        *path,
                        Duration::from_millis(manifest.scenario.load.settlement_ms),
                    )
                    .await,
                );
            }
        }
        let cycle = StabilityCycle {
            schema_version: 1,
            index,
            before: previous.clone(),
            window,
            after_window,
            canaries,
            after_canaries: runtime.snapshot(),
            admission_open: !runtime.is_draining(),
        };
        let verdict = cycle.validate_with_topology(manifest, &previous, index, &topology);
        emit(&cycle)?;
        if let Err(error) = verdict {
            reason = Some(error);
            break;
        }
        previous = cycle.after_canaries;
        completed += 1;
        let target = origin
            + Duration::from_millis(
                manifest.min_duration_ms * u64::from(completed) / u64::from(manifest.cycles),
            );
        tokio::time::sleep_until(tokio::time::Instant::from_std(target)).await;
    }
    let drain_ok = runtime
        .drain(Duration::from_millis(manifest.scenario.load.settlement_ms))
        .await
        .is_ok();
    let expected_topology = topology.clone();
    let mut summary = StabilitySummary {
        schema_version: 1,
        pid: std::process::id(),
        host_builds: 1,
        expected_cycles: manifest.cycles,
        completed_cycles: completed,
        baseline,
        final_snapshot: runtime.snapshot(),
        final_draining: runtime.is_draining(),
        drain_ok,
        elapsed_ns: u64::try_from(origin.elapsed().as_nanos()).unwrap_or(u64::MAX),
        topology,
        reason,
    };
    if summary.reason.is_none() {
        if let Err(error) = summary.validate_with_topology(manifest, &previous, &expected_topology)
        {
            summary.reason = Some(error);
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_load::{CallerDisposition, HostHarnessTestGate};

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn failed_cycle_writer_closes_admission_and_settles_owned_work() {
        let scenario = HostScenario::from_json(include_bytes!(
            "../../../tools/bench/scenarios/h2-burst-recovery-smoke.json"
        ))
        .unwrap();
        let manifest = StabilityManifest {
            schema_version: 1,
            scenario,
            cycles: 3,
            min_duration_ms: 0,
            max_total_records: 100,
        };
        let runtime = manifest.scenario.build_runtime().unwrap();
        let mut writes = 0;
        let error = run_stability_on_runtime(&manifest, &runtime, &mut |_| {
            writes += 1;
            Err("writer failed".into())
        })
        .await
        .unwrap_err();
        assert_eq!(error, "writer failed");
        assert_eq!(writes, 1);
        assert!(runtime.is_draining());
        zero_owned(&runtime.snapshot(), &manifest.scenario).unwrap();
        assert!(
            run_host_window(&manifest.scenario, &runtime, None, None, false)
                .await
                .is_err()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn early_error_reports_bounded_cleanup_failure_with_live_worker() {
        let mut scenario = HostScenario::from_json(include_bytes!(
            "../../../tools/bench/scenarios/h2-burst-recovery-smoke.json"
        ))
        .unwrap();
        scenario.load.warmup_ms = 0;
        let manifest = StabilityManifest {
            schema_version: 1,
            scenario,
            cycles: 3,
            min_duration_ms: 0,
            max_total_records: 100,
        };
        let runtime = manifest.scenario.build_runtime().unwrap();
        let release = Arc::new(AtomicBool::new(false));
        let release_worker = release.clone();
        let owned = runtime.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let held = tokio::spawn(async move {
            owned
                .run_blocking(
                    TaskSpec::blocking(TaskClass::new("burst")).operation("held-cleanup"),
                    move || {
                        let _ = started_tx.send(());
                        while !release_worker.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        Ok::<(), ()>(())
                    },
                )
                .await
        });
        if tokio::time::timeout(Duration::from_secs(2), started_rx)
            .await
            .unwrap()
            .is_err()
        {
            panic!("held submission failed: {:?}", held.await);
        }
        let error = run_stability_on_runtime(&manifest, &runtime, &mut |_| {
            panic!("baseline error must precede the writer")
        })
        .await
        .unwrap_err();
        assert!(error.contains("checkpoint has invalid catalog"));
        assert!(error.contains("stability cleanup drain failed"));
        assert!(runtime.is_draining());
        assert!(runtime
            .snapshot()
            .classes
            .values()
            .any(|class| class.inflight > 0));
        release.store(true, Ordering::SeqCst);
        held.await.unwrap().unwrap();
        runtime.drain(Duration::from_secs(2)).await.unwrap();
        zero_owned(&runtime.snapshot(), &manifest.scenario).unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn intermediate_checkpoint_cannot_release_a_held_worker_after_caller_drop() {
        let mut scenario = HostScenario::from_json(include_bytes!(
            "../../../tools/bench/scenarios/h5-custody-smoke.json"
        ))
        .unwrap();
        scenario.offers.truncate(1);
        let runtime = scenario.build_runtime().unwrap();
        let gate = Arc::new(HostHarnessTestGate::new(0));
        let run_runtime = runtime.clone();
        let run_scenario = scenario.clone();
        let run_gate = gate.clone();
        let run = tokio::spawn(async move {
            run_host_window(&run_scenario, &run_runtime, None, Some(run_gate), false).await
        });
        tokio::time::timeout(Duration::from_secs(5), gate.wait_started())
            .await
            .unwrap();
        gate.trigger_drop();
        tokio::time::timeout(Duration::from_secs(5), gate.wait_caller_dropped())
            .await
            .unwrap();
        assert!(zero_owned(&runtime.snapshot(), &scenario).is_err());
        assert!(!runtime.is_draining());
        assert!(!run.is_finished());
        gate.release_worker();
        let (window, _) = tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        window
            .validate_window_with_topology(&scenario, &ResolvedHostTopology::from_runtime(&runtime))
            .unwrap();
        assert_eq!(
            window.records[0].disposition,
            CallerDisposition::CallerDropped
        );
        assert!(
            window.records[0].body_finished_ns.unwrap() > window.records[0].caller_drop_ns.unwrap()
        );
        zero_owned(&runtime.snapshot(), &scenario).unwrap();
        assert!(
            canary(
                &runtime,
                scenario.classes[0].name.clone(),
                HostPath::Blocking,
                Duration::from_secs(1)
            )
            .await
            .body_ran
        );
        runtime.drain(Duration::from_secs(1)).await.unwrap();
        assert!(run_host_window(&scenario, &runtime, None, None, false)
            .await
            .is_err());
    }
}

#[cfg(test)]
mod construction_tests {
    use super::*;
    use crate::host_scenarios::BUILD_CALLS;

    // Keep factory observation on one thread; other tests cannot affect it.
    #[tokio::test(flavor = "current_thread")]
    async fn measurement_builds_exactly_one_host_including_online_validation() {
        let manifest = StabilityManifest {
            schema_version: 1,
            scenario: HostScenario::from_json(include_bytes!(
                "../../../tools/bench/scenarios/h2-burst-recovery-smoke.json"
            ))
            .unwrap(),
            cycles: 3,
            min_duration_ms: 0,
            max_total_records: 100,
        };
        let before = BUILD_CALLS.with(std::cell::Cell::get);
        let mut emitted = 0;
        let summary = run_stability(&manifest, |_| {
            emitted += 1;
            Ok(())
        })
        .await
        .unwrap();
        let actual = BUILD_CALLS.with(std::cell::Cell::get) - before;
        assert_eq!(emitted, 3);
        assert!(summary.reason.is_none());
        assert_eq!(actual, 1, "online validation must not create another host");
    }
}
