use taskmesh_bench::host_scenarios::{HostPath, HostScenario};
use taskmesh_bench::host_stability::{run_stability, StabilityCycle, StabilityManifest};

fn manifest(bytes: &[u8]) -> StabilityManifest {
    StabilityManifest {
        schema_version: 1,
        scenario: HostScenario::from_json(bytes).unwrap(),
        cycles: 3,
        min_duration_ms: 0,
        max_total_records: 100,
    }
}

async fn acquire(manifest: &StabilityManifest) -> Vec<StabilityCycle> {
    let mut cycles = Vec::new();
    let summary = run_stability(manifest, |cycle| {
        cycles.push(cycle.clone());
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(cycles.len(), 3);
    let mut previous = summary.baseline.clone();
    for (index, cycle) in cycles.iter().enumerate() {
        cycle.validate(manifest, &previous, index as u32).unwrap();
        assert!(!cycle.window.drain_ok);
        assert!(cycle.window.validate_against(&manifest.scenario).is_err());
        previous = cycle.after_canaries.clone();
    }
    summary.validate(manifest, &previous).unwrap();
    assert!(summary.final_draining);
    assert_eq!(summary.host_builds, 1);
    cycles
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h2_three_cycles_share_host_and_recover() {
    acquire(&manifest(include_bytes!(
        "../../../tools/bench/scenarios/h2-burst-recovery-smoke.json"
    )))
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn h5_three_cycles_settle_post_response_custody() {
    acquire(&manifest(include_bytes!(
        "../../../tools/bench/scenarios/h5-custody-smoke.json"
    )))
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_used_path_has_real_canary_and_corruptions_reject() {
    let mut manifest = manifest(include_bytes!(
        "../../../tools/bench/scenarios/h1-default-send-smoke.json"
    ));
    manifest.max_total_records = 100;
    let cycles = acquire(&manifest).await;
    let good = &cycles[1];
    assert!(good.canaries.iter().any(|c| c.path == HostPath::Io));
    assert!(good.canaries.iter().any(|c| c.path == HostPath::Blocking));
    assert!(good.canaries.iter().any(|c| c.path == HostPath::Cpu));
    for mutation in 0..8 {
        let mut bad = good.clone();
        match mutation {
            0 => bad.index = 0,
            1 => bad.admission_open = false,
            2 => {
                bad.canaries.pop();
            }
            3 => bad.canaries[0].body_ran = false,
            4 => bad.before = cycles[0].before.clone(),
            5 => {
                bad.after_window
                    .classes
                    .values_mut()
                    .next()
                    .unwrap()
                    .admitted_total += 1
            }
            6 => {
                bad.after_canaries
                    .capabilities
                    .values_mut()
                    .next()
                    .unwrap()
                    .in_use = 1
            }
            _ => bad.window.drain_ok = true,
        }
        assert!(
            bad.validate(&manifest, &cycles[0].after_canaries, 1)
                .is_err(),
            "mutation {mutation}"
        );
    }
}

#[test]
fn budgets_and_unsupported_paths_reject_before_host_creation() {
    let base = manifest(include_bytes!(
        "../../../tools/bench/scenarios/h2-burst-recovery-smoke.json"
    ));
    for mutation in 0..5 {
        let mut bad = base.clone();
        match mutation {
            0 => bad.cycles = 0,
            1 => bad.max_total_records = 1,
            2 => bad.cycles = 10_001,
            3 => bad.min_duration_ms = 1_200_001,
            _ => bad.scenario.offers[0].path = HostPath::RequestedStackBlocking,
        }
        assert!(bad.validate().is_err());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn writer_failure_does_not_emit_success_summary() {
    let manifest = manifest(include_bytes!(
        "../../../tools/bench/scenarios/h2-burst-recovery-smoke.json"
    ));
    let mut attempts = 0;
    let result = run_stability(&manifest, |_| {
        attempts += 1;
        Err("disk failure".into())
    })
    .await;
    assert!(result.is_err());
    assert_eq!(attempts, 1);
}

#[test]
fn global_snapshot_budget_includes_summary_endpoints() {
    let mut manifest = manifest(include_bytes!(
        "../../../tools/bench/scenarios/h2-burst-recovery-smoke.json"
    ));
    manifest.cycles = 1000;
    manifest.max_total_records = 20_000;
    manifest.scenario.load.injection_ms = 997;
    manifest.scenario.load.interval_ms = 1;
    manifest.scenario.load.snapshot_ms = 1;
    manifest.scenario.validate().unwrap();
    assert!(manifest.validate().is_err());
    manifest.scenario.load.injection_ms = 996;
    manifest.validate().unwrap();
}
