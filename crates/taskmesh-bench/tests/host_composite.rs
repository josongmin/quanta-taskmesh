use taskmesh_bench::composite_host::{run_composite_with_topology, CompositeScenario};
use taskmesh_bench::host_load::ResponseOutcome;

const FIXTURE: &[u8] = include_bytes!("../../../tools/bench/scenarios/h6-composite-smoke.json");

#[tokio::test]
async fn composite_child_failure_keeps_keyed_reduce_and_governance() {
    let scenario = CompositeScenario::from_json(FIXTURE).expect("composite fixture");
    let (mut raw, topology) = run_composite_with_topology(&scenario)
        .await
        .expect("composite diagnostic run");
    raw.validate_against(&scenario)
        .expect("composite raw parity");
    assert_eq!(raw.reduced_keys, [1, 3]);
    assert_eq!(raw.checksum, 4);
    assert!(matches!(
        raw.children[1].outcome,
        ResponseOutcome::TaskError
    ));
    assert_eq!(raw.class_counters["parent"].started, 1);
    assert_eq!(raw.class_counters["child"].started, 3);
    assert_eq!(topology.schema_version, 1);
    let mut wrong_capabilities = raw.clone();
    wrong_capabilities.final_capabilities =
        std::collections::BTreeMap::from([("unregistered".into(), 0)]);
    assert!(wrong_capabilities
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("capability catalog")));

    raw.checksum += 1;
    assert!(raw
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("keyed reduction")));
}

#[tokio::test]
async fn composite_all_success_is_key_ordered() {
    let mut scenario = CompositeScenario::from_json(FIXTURE).expect("composite fixture");
    scenario.fail_child_key = None;
    let (raw, _) = run_composite_with_topology(&scenario)
        .await
        .expect("composite success run");
    raw.validate_against(&scenario)
        .expect("composite raw parity");
    assert_eq!(raw.reduced_keys, [1, 2, 3]);
    assert_eq!(raw.checksum, 6);
}

#[tokio::test]
async fn composite_timeout_retains_typed_invalid_raw_and_settlement_observation() {
    let mut scenario = CompositeScenario::from_json(FIXTURE).expect("composite fixture");
    scenario.id = "h6-timeout-regression".into();
    // Keep delayed bodies beyond the parent deadline, but within the later
    // drain deadline. Equal delays make the timeout/success result a race.
    scenario.settlement_ms = 1_000;
    scenario.parent_timeout_ms = Some(500);
    scenario.io_delay_ms = 750;
    scenario.blocking_delay_ms = 750;
    let failure = run_composite_with_topology(&scenario)
        .await
        .expect_err("parent must time out before delayed child bodies complete");
    assert!(
        failure.reason.contains("exceeded settlement bound"),
        "{}",
        failure.reason
    );
    let raw = failure.raw.expect("failure raw must be retained");
    raw.validate_against(&scenario)
        .expect("partial failure raw must remain typed");
    assert_eq!(raw.status, "invalid");
    assert_eq!(raw.children.len(), 3);
    assert!(raw.drain_ok);
    assert!(raw.conservation_ok);
    let mut unsettled = raw.clone();
    unsettled.drain_ok = false;
    unsettled.conservation_ok = false;
    *unsettled.final_capabilities.values_mut().next().unwrap() = 1;
    unsettled
        .validate_against(&scenario)
        .expect("typed INVALID diagnostic may retain registered owned capacity");
    let mut wrong_capabilities = raw.clone();
    wrong_capabilities.final_capabilities =
        std::collections::BTreeMap::from([("unregistered".into(), 0)]);
    assert!(wrong_capabilities
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("capability catalog")));
    let retained =
        serde_json::to_vec(&wrong_capabilities).expect("invalid diagnostic is retainable");
    let restored: taskmesh_bench::composite_host::CompositeFailureRaw =
        serde_json::from_slice(&retained).unwrap();
    assert!(restored
        .validate_against(&scenario)
        .unwrap_err()
        .contains("capability catalog differs from resolved runtime"));
    assert_eq!(
        *failure.topology.expect("failure topology must be retained"),
        scenario.resolved_topology().expect("scenario topology")
    );
}

#[test]
fn composite_preflight_rejects_unsupported_failure_key_and_body_bound() {
    let mut scenario = CompositeScenario::from_json(FIXTURE).expect("composite fixture");
    scenario.fail_child_key = Some(4);
    assert!(scenario
        .validate()
        .is_err_and(|error| error.contains("failure key")));
    scenario.fail_child_key = Some(2);
    scenario.cpu_iterations = 100_000_001;
    assert!(scenario
        .validate()
        .is_err_and(|error| error.contains("cpu work exceeds fixture bound")));
    scenario.cpu_iterations = 1_000;
    scenario.parent_timeout_ms = Some(scenario.settlement_ms + 1);
    assert!(scenario
        .validate()
        .is_err_and(|error| error.contains("parent timeout must fit settlement window")));
}
