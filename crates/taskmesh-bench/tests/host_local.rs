use taskmesh_bench::host_load::ResponseOutcome;
use taskmesh_bench::local_host::{run_local_host_with_topology, LocalHostScenario};

const FIXTURE: &[u8] = include_bytes!("../../../tools/bench/scenarios/h4-local-smoke.json");

#[tokio::test(flavor = "current_thread")]
async fn caller_affine_non_send_fixture_keeps_bounded_raw_rows() {
    let scenario = LocalHostScenario::from_json(FIXTURE).expect("local fixture");
    let (mut raw, topology) = run_local_host_with_topology(&scenario)
        .await
        .expect("local diagnostic run");
    raw.validate_against(&scenario).expect("local raw parity");
    assert_eq!(raw.records.len(), 2);
    assert!(raw
        .records
        .iter()
        .all(|row| matches!(row.outcome.as_ref(), Some(ResponseOutcome::Success))));
    assert_eq!(raw.class_counters["local"].started, 2);
    assert_eq!(topology.schema_version, 1);
    raw.records[0].pacer_observed_ns += 1;
    assert!(raw
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("pacer observation")));
    raw.records[0].pacer_observed_ns -= 1;
    raw.records[0].body_finished_ns = None;
    assert!(raw
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("success lacks body finish")));
}

#[tokio::test(flavor = "current_thread")]
async fn unsubmitted_local_offer_lag_must_reconcile_with_pacer_observation() {
    let mut scenario = LocalHostScenario::from_json(FIXTURE).expect("local fixture");
    scenario.load.max_outstanding = 1;
    scenario.offers[0].body = taskmesh_bench::host_scenarios::HostBody::AsyncSleep { millis: 40 };
    let (mut raw, _) = run_local_host_with_topology(&scenario)
        .await
        .expect("bounded local run");
    raw.validate_against(&scenario)
        .expect("unsubmitted raw parity");
    assert!(raw.records[1].submitted_ns.is_none());
    raw.records[1].scheduled_lag_ns = u64::MAX;
    assert!(raw
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("pacer observation")));
}

#[test]
fn local_preflight_rejects_unimplemented_snapshot_observer() {
    let mut scenario = LocalHostScenario::from_json(FIXTURE).expect("local fixture");
    scenario.load.snapshot_ms = 10;
    assert!(scenario
        .validate()
        .is_err_and(|error| error.contains("Snapshot sampling is unavailable")));
    scenario.load.snapshot_ms = 0;
    scenario.schema_version = 1;
    assert!(scenario
        .validate()
        .is_err_and(|error| error.contains("unsupported local host scenario version")));
}
