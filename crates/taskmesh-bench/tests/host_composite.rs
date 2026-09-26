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
}
