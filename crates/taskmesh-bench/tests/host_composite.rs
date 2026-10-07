use taskmesh_bench::composite_host::{run_composite_with_topology, CompositeScenario};
use taskmesh_bench::host_load::ResponseOutcome;

const FIXTURE: &[u8] = include_bytes!("../../../tools/bench/scenarios/h6-composite-smoke.json");

#[tokio::test]
async fn composite_child_failure_keeps_keyed_reduce_and_governance() {
    let mut scenario = CompositeScenario::from_json(FIXTURE).expect("composite fixture");
    let mut unused = scenario.classes[0].clone();
    unused.name = "unused".into();
    scenario.classes.push(unused);
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
    check_success_raw_contract(&scenario, &raw);

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

fn retained_failure_before_submission(
    scenario: &CompositeScenario,
) -> taskmesh_bench::composite_host::CompositeFailureRaw {
    use taskmesh_bench::composite_host::{CompositeFailureRaw, CompositePartialChild};
    use taskmesh_bench::host_load::ClassCounters;
    use taskmesh_bench::host_scenarios::HostPath;

    let topology = scenario.resolved_topology().expect("resolved fixture");
    // A retained failure can precede child submission. Empty event histories
    // keep the population and parent-time witnesses independent of child time.
    CompositeFailureRaw {
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
    }
}

#[test]
fn composite_failure_raw_rejects_identity_population_and_parent_time_independently() {
    use taskmesh_bench::composite_host::CompositeFailureRaw;

    let scenario = CompositeScenario::from_json(FIXTURE).expect("composite fixture");
    let raw = retained_failure_before_submission(&scenario);
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

#[test]
fn composite_failure_raw_rejects_child_key_and_path_independently() {
    use taskmesh_bench::host_scenarios::HostPath;

    let scenario = CompositeScenario::from_json(FIXTURE).expect("composite fixture");
    let raw = retained_failure_before_submission(&scenario);
    raw.validate_against(&scenario)
        .expect("valid child catalog");
    for index in 0..raw.children.len() {
        let expected = format!("composite failure child {index}: key or path differs");
        // Every row keeps its valid path when only its key is corrupted.
        for key in [0, raw.children[(index + 1) % 3].key] {
            reject_raw_edit(
                &raw,
                &format!("/children/{index}/key"),
                serde_json::json!(key),
                &expected,
                |value| value.validate_against(&scenario),
            );
        }
        // A different registered path is still invalid for this keyed child.
        for path in [HostPath::Io, HostPath::Blocking, HostPath::Cpu] {
            if path != raw.children[index].path {
                reject_raw_edit(
                    &raw,
                    &format!("/children/{index}/path"),
                    serde_json::json!(path),
                    &expected,
                    |value| value.validate_against(&scenario),
                );
            }
        }
    }
}

#[test]
fn composite_failure_raw_checks_partial_event_order_and_inclusive_times() {
    let scenario = CompositeScenario::from_json(FIXTURE).expect("composite fixture");
    let raw = retained_failure_before_submission(&scenario);
    // Structural projections exercise timestamp prerequisites independently;
    // the async timeout test above owns actual producer/counter settlement.
    let histories = [
        [None, None, None, None],
        [Some(10), None, None, None],
        [Some(10), None, None, Some(20)], // Rejected before body start.
        [Some(10), Some(12), None, None],
        [Some(10), Some(12), None, Some(20)], // Response does not prove finish.
        [Some(10), Some(12), Some(15), None],
        [Some(10), Some(12), Some(15), Some(20)],
        [Some(10); 4],
        [Some(20); 4],
    ];
    for history in histories {
        let mut partial = raw.clone();
        for row in &mut partial.children {
            [
                row.submitted_ns,
                row.body_started_ns,
                row.body_finished_ns,
                row.response_ns,
            ] = history;
        }
        partial
            .validate_against(&scenario)
            .expect("valid partial history and inclusive observation boundaries");
    }

    for index in 0..raw.children.len() {
        let expected = format!("composite failure child {index}: time differs");
        let mut complete = raw.clone();
        let row = &mut complete.children[index];
        row.submitted_ns = Some(11);
        row.body_started_ns = Some(13);
        row.body_finished_ns = Some(15);
        row.response_ns = Some(17);
        complete
            .validate_against(&scenario)
            .expect("ordered history");
        for (field, before_previous) in [
            ("submitted_ns", 9),
            ("body_started_ns", 10),
            ("body_finished_ns", 12),
        ] {
            for time in [before_previous, 21] {
                reject_raw_edit(
                    &complete,
                    &format!("/children/{index}/{field}"),
                    serde_json::json!(time),
                    &expected,
                    |value| value.validate_against(&scenario),
                );
            }
        }
        // An unfinished worker is valid after caller response; isolate the
        // response-time witness from body-finish ordering.
        complete.children[index].body_finished_ns = None;
        complete
            .validate_against(&scenario)
            .expect("caller responded while body remained unfinished");
        for time in [12, 21] {
            reject_raw_edit(
                &complete,
                &format!("/children/{index}/response_ns"),
                serde_json::json!(time),
                &expected,
                |value| value.validate_against(&scenario),
            );
        }
        let expected = format!("composite failure child {index}: event order differs");
        for history in [
            [None, Some(12), None, None],
            [None, None, Some(15), None],
            [None, None, None, Some(20)],
            [Some(10), None, Some(15), None],
        ] {
            let mut invalid = raw.clone();
            let row = &mut invalid.children[index];
            [
                row.submitted_ns,
                row.body_started_ns,
                row.body_finished_ns,
                row.response_ns,
            ] = history;
            assert!(invalid
                .validate_against(&scenario)
                .is_err_and(|error| error == expected));
        }
    }
}

#[test]
fn composite_failure_raw_checks_class_inventory_and_counter_bounds_independently() {
    let mut scenario = CompositeScenario::from_json(FIXTURE).expect("composite fixture");
    let mut unused = scenario.classes[0].clone();
    unused.name = "unused".into();
    scenario.classes.push(unused);
    let raw = retained_failure_before_submission(&scenario);
    raw.validate_against(&scenario)
        .expect("registered unused class has no admissions");

    let mut settled = raw.clone();
    settled.reason = "parent failed after child execution".into();
    for row in &mut settled.children {
        row.submitted_ns = Some(10);
        row.body_started_ns = Some(12);
        row.body_finished_ns = Some(15);
        row.response_ns = Some(20);
    }
    for (name, maximum) in [("parent", 1), ("child", 3), ("unused", 0)] {
        let counters = settled.class_counters.get_mut(name).unwrap();
        counters.admitted = maximum;
        counters.started = maximum;
        counters.terminated = maximum;
    }
    settled
        .validate_against(&scenario)
        .expect("inclusive class bounds with complete child histories");
    for (name, maximum) in [("parent", 1), ("child", 3), ("unused", 0)] {
        let expected = format!("composite failure class {name} counters differ");
        for field in ["admitted", "started", "terminated"] {
            reject_raw_edit(
                &settled,
                &format!("/class_counters/{name}/{field}"),
                serde_json::json!(maximum + 1),
                &expected,
                |value| value.validate_against(&scenario),
            );
        }
        // Keep catalog size unchanged so the missing-name guard is exercised.
        let mut renamed = raw.clone();
        let counters = renamed.class_counters.remove(name).unwrap();
        renamed
            .class_counters
            .insert("unregistered".into(), counters);
        assert!(renamed
            .validate_against(&scenario)
            .is_err_and(|error| error == format!("composite failure class {name} missing")));
    }
    for extra in [false, true] {
        let mut wrong_population = raw.clone();
        if extra {
            wrong_population
                .class_counters
                .insert("unregistered".into(), Default::default());
        } else {
            wrong_population.class_counters.remove("unused");
        }
        assert!(wrong_population
            .validate_against(&scenario)
            .is_err_and(|error| error == "composite failure governance inventory differs"));
    }
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

fn check_success_raw_contract(
    scenario: &CompositeScenario,
    raw: &taskmesh_bench::composite_host::CompositeRaw,
) {
    use taskmesh_bench::composite_host::CompositeRaw;
    use taskmesh_bench::host_scenarios::HostPath;

    raw.validate_against(scenario).expect("valid success raw");
    let reject = |base: &CompositeRaw, pointer: &str, value: serde_json::Value, expected: &str| {
        reject_raw_edit(base, pointer, value, expected, |candidate| {
            candidate.validate_against(scenario)
        });
    };
    let identity = "composite raw identity, population or parent time differs";
    for (field, value) in [
        ("schema_version", serde_json::json!(2)),
        ("mode", serde_json::json!("other")),
        ("scenario_id", serde_json::json!("other")),
    ] {
        reject(raw, &format!("/{field}"), value, identity);
    }

    // Normalize time so every one-field corruption reaches its own bound,
    // regardless of the timings of the one real diagnostic run above.
    let mut timed = raw.clone();
    timed.parent_submitted_ns = 10;
    timed.parent_response_ns = 40;
    timed.reduce_started_ns = 30;
    timed.reduce_finished_ns = 35;
    for row in &mut timed.children {
        row.submitted_ns = 10;
        row.body_started_ns = Some(15);
        row.body_finished_ns = Some(20);
        row.response_ns = 25;
    }
    timed
        .validate_against(scenario)
        .expect("valid normalized times");
    for (field, value) in [
        ("parent_submitted_ns", serde_json::json!(41)),
        ("reduce_started_ns", serde_json::json!(36)),
        ("reduce_finished_ns", serde_json::json!(41)),
    ] {
        reject(&timed, &format!("/{field}"), value, identity);
    }
    let mut inclusive = timed.clone();
    inclusive.parent_response_ns = 30;
    inclusive.reduce_finished_ns = 30;
    for row in &mut inclusive.children {
        row.body_started_ns = Some(10);
        row.body_finished_ns = Some(10);
        row.response_ns = 30;
    }
    inclusive
        .validate_against(scenario)
        .expect("equal declaration boundaries are valid");
    let mut simultaneous = timed.clone();
    simultaneous.parent_response_ns = 10;
    simultaneous.reduce_started_ns = 10;
    simultaneous.reduce_finished_ns = 10;
    for row in &mut simultaneous.children {
        row.body_started_ns = Some(10);
        row.body_finished_ns = Some(10);
        row.response_ns = 10;
    }
    simultaneous
        .validate_against(scenario)
        .expect("all parent, child and reduce events may share the inclusive boundary");

    for index in 0..3 {
        let expected = format!("composite child {}: invalid attribution or time", index + 1);
        reject(
            &timed,
            &format!("/children/{index}/key"),
            serde_json::json!(0),
            &expected,
        );
        let other_path = if timed.children[index].path == HostPath::Io {
            HostPath::Blocking
        } else {
            HostPath::Io
        };
        reject(
            &timed,
            &format!("/children/{index}/path"),
            serde_json::json!(other_path),
            &expected,
        );
        for (field, value) in [
            ("submitted_ns", serde_json::json!(9)),
            ("body_started_ns", serde_json::json!(9)),
            ("body_finished_ns", serde_json::json!(14)),
            ("response_ns", serde_json::json!(19)),
            ("response_ns", serde_json::json!(31)),
            ("body_started_ns", serde_json::json!(null)),
            ("body_finished_ns", serde_json::json!(null)),
        ] {
            reject(
                &timed,
                &format!("/children/{index}/{field}"),
                value,
                &expected,
            );
        }
    }

    let failure = "composite child 2: failure outcome differs";
    reject(
        raw,
        "/children/1/outcome",
        serde_json::json!(ResponseOutcome::Success),
        failure,
    );
    reject(raw, "/children/1/value", serde_json::json!(2), failure);
    for index in [0, 2] {
        let expected = format!("composite child {}: success outcome differs", index + 1);
        reject(
            raw,
            &format!("/children/{index}/outcome"),
            serde_json::json!(ResponseOutcome::TaskError),
            &expected,
        );
        reject(
            raw,
            &format!("/children/{index}/value"),
            serde_json::json!(null),
            &expected,
        );
        reject(
            raw,
            &format!("/children/{index}/value"),
            serde_json::json!(index + 2),
            &expected,
        );
    }
    reject(
        raw,
        "/reduced_keys",
        serde_json::json!([1, 2]),
        "caller-owned keyed reduction differs",
    );

    let unsettled = "composite governance did not settle";
    for field in ["root_attribution_cleared", "drain_ok", "conservation_ok"] {
        reject(
            raw,
            &format!("/{field}"),
            serde_json::json!(false),
            unsettled,
        );
    }
    let topology = scenario.resolved_topology().expect("resolved scenario");
    let pool = topology
        .capability_limits
        .iter()
        .find(|(_, limit)| **limit > 0)
        .expect("registered pool with capacity")
        .0;
    let mut held = raw.clone();
    *held.final_capabilities.get_mut(pool).unwrap() = 1;
    assert!(held
        .validate_against(scenario)
        .is_err_and(|error| error == unsettled));

    let mut missing = raw.clone();
    missing.class_counters.remove("child");
    assert!(missing
        .validate_against(scenario)
        .is_err_and(|error| error == unsettled));
    let mut renamed = raw.clone();
    let child = renamed.class_counters.remove("child").unwrap();
    renamed.class_counters.insert("unknown".into(), child);
    assert!(renamed
        .validate_against(scenario)
        .is_err_and(|error| error == "composite class child missing"));
    for (name, expected_count) in [("parent", 1), ("child", 3), ("unused", 0)] {
        let expected = format!("composite class {name} counters differ");
        for field in ["admitted", "started", "terminated"] {
            reject(
                raw,
                &format!("/class_counters/{name}/{field}"),
                serde_json::json!(if expected_count == 0 {
                    1
                } else {
                    expected_count - 1
                }),
                &expected,
            );
        }
        for field in ["inflight", "queued"] {
            reject(
                raw,
                &format!("/class_counters/{name}/{field}"),
                serde_json::json!(1),
                &expected,
            );
        }
    }
}
