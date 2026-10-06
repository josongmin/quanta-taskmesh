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

    check_success_raw_population(&scenario, &raw);

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
fn composite_preflight_rejects_version_and_class_alias_independently() {
    let fixture = CompositeScenario::from_json(FIXTURE).expect("valid composite fixture");
    fixture.validate().expect("valid direct composite scenario");
    assert_eq!(fixture.fail_child_key, Some(2));

    let rejects = |scenario: &CompositeScenario| {
        let expected = "invalid composite version, class split or failure key";
        assert!(scenario
            .validate()
            .is_err_and(|error| error.contains(expected)));
        let json = serde_json::to_vec(scenario).expect("serialize composite scenario");
        assert!(CompositeScenario::from_json(&json).is_err_and(|error| error.contains(expected)));
    };

    let mut unsupported_version = fixture.clone();
    unsupported_version.schema_version = 2;
    rejects(&unsupported_version); // Class split and failure key remain valid.

    let mut aliased_classes = fixture;
    aliased_classes.child_class = aliased_classes.parent_class.clone();
    rejects(&aliased_classes); // Version and failure key remain valid.
}

#[test]
fn composite_preflight_rejects_unsupported_failure_key_and_body_bound() {
    let mut scenario = CompositeScenario::from_json(FIXTURE).expect("composite fixture");
    scenario.fail_child_key = Some(0);
    assert!(scenario
        .validate()
        .is_err_and(|error| error.contains("failure key")));
    scenario.fail_child_key = Some(4);
    assert!(scenario
        .validate()
        .is_err_and(|error| error.contains("failure key")));
    scenario.fail_child_key = Some(2);
    scenario.validate().expect("registered child key is valid");
    scenario.cpu_iterations = 100_000_001;
    assert!(scenario
        .validate()
        .is_err_and(|error| error.contains("cpu work exceeds fixture bound")));
    scenario.cpu_iterations = 1_000;
    scenario.parent_timeout_ms = Some(0);
    assert!(scenario
        .validate()
        .is_err_and(|error| error.contains("parent timeout must fit settlement window")));
    scenario.parent_timeout_ms = Some(scenario.settlement_ms);
    scenario
        .validate()
        .expect("inclusive settlement timeout is valid");
    scenario.parent_timeout_ms = Some(scenario.settlement_ms + 1);
    assert!(scenario
        .validate()
        .is_err_and(|error| error.contains("parent timeout must fit settlement window")));
}

fn reject_raw_edit<T: serde::Serialize + serde::de::DeserializeOwned>(
    raw: &T,
    pointer: &str,
    value: serde_json::Value,
    expected: &str,
    validate: impl Fn(&T) -> Result<(), String>,
) {
    let mut document = serde_json::to_value(raw).expect("serialize valid raw");
    *document.pointer_mut(pointer).expect("existing raw field") = value;
    let restored: T = serde_json::from_value(document).expect("retain typed invalid raw");
    assert!(
        validate(&restored).is_err_and(|error| error.contains(expected)),
        "{pointer} must reject independently with {expected}"
    );
}

#[test]
fn composite_failure_raw_rejects_identity_population_and_parent_time_independently() {
    use taskmesh_bench::composite_host::{CompositeFailureRaw, CompositePartialChild};
    use taskmesh_bench::host_load::ClassCounters;
    use taskmesh_bench::host_scenarios::HostPath;

    let scenario = CompositeScenario::from_json(FIXTURE).expect("composite fixture");
    let topology = scenario.resolved_topology().expect("resolved fixture");
    // A retained failure can precede child submission. Empty event histories
    // keep the population and parent-time witnesses independent of child time.
    let raw = CompositeFailureRaw {
        schema_version: 1,
        status: "invalid".into(),
        mode: "caller_orchestrated_composite".into(),
        scenario_id: scenario.id.clone(),
        reason: "parent failed before child submission".into(),
        parent_submitted_ns: 10,
        observed_ns: 20,
        children: [HostPath::Io, HostPath::Blocking, HostPath::Cpu]
            .into_iter()
            .enumerate()
            .map(|(index, path)| CompositePartialChild {
                key: (index + 1) as u8,
                path,
                submitted_ns: None,
                body_started_ns: None,
                body_finished_ns: None,
                response_ns: None,
            })
            .collect(),
        root_attribution_cleared: true,
        class_counters: scenario
            .classes
            .iter()
            .map(|class| (class.name.clone(), ClassCounters::default()))
            .collect(),
        final_capabilities: topology
            .capability_limits
            .keys()
            .map(|pool| (pool.clone(), 0))
            .collect(),
        drain_ok: true,
        conservation_ok: true,
    };
    raw.validate_against(&scenario)
        .expect("valid retained failure before child submission");
    let restored: CompositeFailureRaw =
        serde_json::from_slice(&serde_json::to_vec(&raw).unwrap()).unwrap();
    restored
        .validate_against(&scenario)
        .expect("valid failure JSON round trip");

    let identity = "invalid composite failure identity or population";
    let mut extra_child = raw.children.clone();
    extra_child.push(raw.children[0].clone());
    for (pointer, value) in [
        ("/schema_version", serde_json::json!(2)),
        ("/status", serde_json::json!("complete")),
        ("/mode", serde_json::json!("other")),
        ("/scenario_id", serde_json::json!("other")),
        ("/reason", serde_json::json!("")),
        ("/children", serde_json::json!(extra_child)),
        ("/children", serde_json::json!(&raw.children[..2])),
        ("/parent_submitted_ns", serde_json::json!(21)),
    ] {
        reject_raw_edit(&raw, pointer, value, identity, |value| {
            value.validate_against(&scenario)
        });
    }

    let mut equal_parent_time = raw.clone();
    equal_parent_time.observed_ns = equal_parent_time.parent_submitted_ns;
    equal_parent_time
        .validate_against(&scenario)
        .expect("inclusive parent observation boundary");
}

fn check_success_raw_population(
    scenario: &CompositeScenario,
    raw: &taskmesh_bench::composite_host::CompositeRaw,
) {
    let mut extra_child = raw.children.clone();
    extra_child.push(raw.children[0].clone());
    for children in [raw.children[..2].to_vec(), extra_child] {
        reject_raw_edit(
            raw,
            "/children",
            serde_json::json!(children),
            "identity, population or parent time differs",
            |value| value.validate_against(scenario),
        );
    }
}
