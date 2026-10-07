use taskmesh_bench::host_load::ResponseOutcome;
use taskmesh_bench::local_host::{run_local_host_with_topology, LocalHostScenario};

const FIXTURE: &[u8] = include_bytes!("../../../tools/bench/scenarios/h4-local-smoke.json");

#[tokio::test(flavor = "current_thread")]
async fn caller_affine_non_send_fixture_keeps_bounded_raw_rows() {
    let scenario = LocalHostScenario::from_json(FIXTURE).expect("local fixture");
    assert_eq!(scenario.id, "h4-caller-affine-local-smoke-v3");
    assert_eq!(scenario.offers.len(), 2);
    let (mut raw, topology) = run_local_host_with_topology(&scenario)
        .await
        .expect("local diagnostic run");
    raw.validate_against(&scenario).expect("local raw parity");
    assert_eq!(raw.scenario_id, "h4-caller-affine-local-smoke-v3");
    assert_eq!(raw.records.len(), 2);
    assert!(raw
        .records
        .iter()
        .all(|row| matches!(row.outcome.as_ref(), Some(ResponseOutcome::Success))));
    assert_eq!(raw.class_counters["local"].started, 2);
    assert_eq!(topology.schema_version, 1);
    let mut wrong_capabilities = raw.clone();
    wrong_capabilities.final_capabilities =
        std::collections::BTreeMap::from([("unregistered".into(), 0)]);
    assert!(wrong_capabilities
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("capability catalog")));
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
async fn local_raw_rejects_independently_wrong_identity_and_coherent_empty_population() {
    let scenario = LocalHostScenario::from_json(FIXTURE).expect("local fixture");
    assert_eq!(scenario.id, "h4-caller-affine-local-smoke-v3");
    assert_eq!(scenario.offers.len(), 2);
    let (raw, _) = run_local_host_with_topology(&scenario)
        .await
        .expect("local diagnostic run");
    raw.validate_against(&scenario).expect("local raw parity");

    let mut wrong_scenario = raw.clone();
    wrong_scenario.scenario_id = "different-scenario".into();

    let mut missing_population = raw.clone();
    missing_population.records.clear();
    for counters in missing_population.class_counters.values_mut() {
        *counters = Default::default();
    }
    assert_eq!(
        (
            wrong_scenario.validate_against(&scenario),
            missing_population.validate_against(&scenario),
        ),
        (
            Err("local raw identity or population differs".into()),
            Err("local raw identity or population differs".into()),
        ),
    );

    let mut wrong_row = raw.clone();
    wrong_row.records[0].id = 1;
    assert!(wrong_row
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("local row 0: offer identity differs")));

    let mut wrong_counter = raw;
    wrong_counter
        .class_counters
        .get_mut("local")
        .unwrap()
        .started += 1;
    assert!(wrong_counter
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("local class local: counters differ from rows")));
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
fn local_preflight_validates_snapshot_cadence_and_rejects_old_schema() {
    let mut scenario = LocalHostScenario::from_json(FIXTURE).expect("local fixture");
    scenario.load.snapshot_ms = 10;
    scenario.validate().expect("local Snapshot is supported");
    scenario.load.snapshot_ms = 17;
    assert!(scenario
        .validate()
        .is_err_and(|error| error.contains("snapshot_ms must divide injection_ms")));
    scenario.load.snapshot_ms = 0;
    scenario.schema_version = 2;
    assert!(scenario
        .validate()
        .is_err_and(|error| error.contains("unsupported local host scenario version")));
}

#[tokio::test(flavor = "current_thread")]
async fn local_controls_and_snapshot_keep_caller_disposition_and_settlement_typed() {
    let mut scenario = LocalHostScenario::from_json(FIXTURE).expect("local fixture");
    scenario.load.snapshot_ms = 10;
    scenario.offers[0].body = taskmesh_bench::host_scenarios::HostBody::AsyncSleep { millis: 40 };
    scenario.offers[0].cancel_after_ms = Some(2);
    scenario.offers[1].body = taskmesh_bench::host_scenarios::HostBody::AsyncSleep { millis: 40 };
    scenario.offers[1].drop_after_ms = Some(2);
    let (mut raw, _) = run_local_host_with_topology(&scenario)
        .await
        .expect("local control run");
    raw.validate_against(&scenario)
        .expect("local control raw parity");
    assert_eq!(raw.snapshots.len(), 10);
    let mut wrong_capabilities = raw.clone();
    wrong_capabilities.snapshots[0].capabilities =
        std::collections::BTreeMap::from([("unregistered".into(), 0)]);
    assert!(wrong_capabilities
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("capability catalog")));
    wrong_capabilities.final_capabilities = wrong_capabilities.snapshots[0].capabilities.clone();
    assert!(wrong_capabilities
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("capability catalog")));
    assert!(raw.drain_ok && raw.conservation_ok);
    assert_eq!(raw.class_counters["local"].inflight, 0);
    for row in &raw.records {
        assert!(row.response_ns.is_some() ^ row.caller_drop_ns.is_some());
    }
    raw.snapshots[0].intended_ns += 1;
    assert!(raw
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("Snapshot")));
}

#[tokio::test(flavor = "current_thread")]
async fn local_deadline_observation_has_a_valid_caller_terminal() {
    let mut scenario = LocalHostScenario::from_json(FIXTURE).expect("local fixture");
    scenario.offers[0].body = taskmesh_bench::host_scenarios::HostBody::AsyncSleep { millis: 40 };
    scenario.offers[0].deadline_ms = Some(2);
    let (raw, _) = run_local_host_with_topology(&scenario)
        .await
        .expect("local deadline run");
    raw.validate_against(&scenario)
        .expect("local deadline parity");
    assert!(matches!(
        raw.records[0].outcome,
        Some(ResponseOutcome::Deadline | ResponseOutcome::Success)
    ));
    assert!(raw.drain_ok);
}
