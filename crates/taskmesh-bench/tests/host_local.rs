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
    raw.records[0].body_finished_ns = None;
    assert!(raw
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("success lacks body finish")));
}

#[test]
fn local_preflight_rejects_unimplemented_snapshot_observer() {
    let mut scenario = LocalHostScenario::from_json(FIXTURE).expect("local fixture");
    scenario.load.snapshot_ms = 10;
    assert!(scenario
        .validate()
        .is_err_and(|error| error.contains("Snapshot sampling is unavailable")));
}
