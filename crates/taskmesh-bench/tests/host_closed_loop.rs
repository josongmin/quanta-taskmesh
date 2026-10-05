use taskmesh::AdmissionVerdict;
use taskmesh_bench::closed_loop::{
    run_closed_loop_with_topology, ClosedLoopRun, ClosedLoopScenario,
};
use taskmesh_bench::host_load::ResponseOutcome;

const FIXTURE: &[u8] =
    include_bytes!("../../../tools/bench/scenarios/h1-io-closed-loop-smoke.json");

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fixed_caller_slots_reconcile_without_an_intended_arrival_schedule() {
    let scenario = ClosedLoopScenario::from_json(FIXTURE).expect("valid closed-loop fixture");
    let (mut raw, topology) = run_closed_loop_with_topology(&scenario)
        .await
        .expect("closed-loop diagnostic run");
    raw.validate_against(&scenario)
        .expect("fixed-concurrency raw parity");
    assert_eq!(raw.mode, "closed_loop_fixed_concurrency");
    assert_eq!(raw.records.len(), 6);
    assert!(raw.records.iter().all(|record| matches!(
        record.as_ref().map(|record| &record.outcome),
        Some(ResponseOutcome::Success)
    )));
    assert_eq!(raw.class_counters["io"].started, 6);
    assert_eq!(topology.schema_version, 1);
    assert!(raw.final_capabilities.values().all(|held| *held == 0));

    let rejects = |candidate: &ClosedLoopRun, expected: &str| {
        assert!(candidate
            .validate_against(&scenario)
            .is_err_and(|error| error.contains(expected)));
    };

    for version in [0, 2] {
        let mut wrong = raw.clone();
        wrong.schema_version = version;
        rejects(&wrong, "raw identity or population");
    }
    for mode in ["", "open_loop"] {
        let mut wrong = raw.clone();
        wrong.mode = mode.into();
        rejects(&wrong, "raw identity or population");
    }
    for id in ["", "other"] {
        let mut wrong = raw.clone();
        wrong.scenario_id = id.into();
        rejects(&wrong, "raw identity or population");
    }
    for concurrency in [scenario.concurrency - 1, scenario.concurrency + 1] {
        let mut wrong = raw.clone();
        wrong.concurrency = concurrency;
        rejects(&wrong, "raw identity or population");
    }
    for iterations in [
        scenario.iterations_per_slot - 1,
        scenario.iterations_per_slot + 1,
    ] {
        let mut wrong = raw.clone();
        wrong.iterations_per_slot = iterations;
        rejects(&wrong, "raw identity or population");
    }
    let mut wrong = raw.clone();
    wrong.records.pop();
    let io = wrong.class_counters.get_mut("io").unwrap();
    io.admitted = 5;
    io.started = 5;
    io.terminated = 5;
    rejects(&wrong, "raw identity or population");
    let mut wrong = raw.clone();
    wrong.records.push(raw.records[5].clone());
    rejects(&wrong, "raw identity or population");
    let mut wrong = raw.clone();
    wrong.elapsed_ns = 0;
    rejects(&wrong, "raw identity or population");
    let mut at_elapsed_limit = raw.clone();
    at_elapsed_limit.elapsed_ns = scenario.max_run_ms * 1_000_000;
    at_elapsed_limit
        .validate_against(&scenario)
        .expect("elapsed time at the declared limit");
    at_elapsed_limit.elapsed_ns += 1;
    rejects(&at_elapsed_limit, "raw identity or population");
    let mut last_response_at_elapsed = raw.clone();
    let elapsed_ns = last_response_at_elapsed.elapsed_ns;
    last_response_at_elapsed.records[5]
        .as_mut()
        .unwrap()
        .response_ns = elapsed_ns;
    last_response_at_elapsed
        .validate_against(&scenario)
        .expect("last response at elapsed time is legal");

    let mut wrong = raw.clone();
    wrong.records[0] = None;
    rejects(&wrong, "missing caller terminal");
    let mut wrong = raw.clone();
    wrong.records[0].as_mut().unwrap().id = 1;
    rejects(&wrong, "invalid caller sequence");
    let mut wrong = raw.clone();
    wrong.records[0].as_mut().unwrap().caller_slot = 1;
    rejects(&wrong, "invalid caller sequence");
    let mut wrong = raw.clone();
    wrong.records[0].as_mut().unwrap().iteration = 1;
    rejects(&wrong, "invalid caller sequence");
    let mut equal_times = raw.clone();
    let first = equal_times.records[0].as_mut().unwrap();
    first.response_ns = first.submitted_ns;
    equal_times
        .validate_against(&scenario)
        .expect("equal submission and response timestamps");
    let mut slot_handoff_at_response = raw.clone();
    slot_handoff_at_response.records[1]
        .as_mut()
        .unwrap()
        .submitted_ns = raw.records[0].as_ref().unwrap().response_ns;
    slot_handoff_at_response
        .validate_against(&scenario)
        .expect("same-slot next call may submit at prior response");
    let mut wrong = raw.clone();
    let first = wrong.records[0].as_mut().unwrap();
    first.submitted_ns = first.response_ns + 1;
    rejects(&wrong, "invalid caller sequence");
    let mut wrong = raw.clone();
    wrong.records[0].as_mut().unwrap().response_ns = raw.elapsed_ns + 1;
    rejects(&wrong, "invalid caller sequence");

    let mut wrong = raw.clone();
    wrong.drain_ok = false;
    rejects(&wrong, "did not settle owned capacity");
    let mut wrong = raw.clone();
    wrong.conservation_ok = false;
    rejects(&wrong, "did not settle owned capacity");
    let mut wrong = raw.clone();
    let available_pool = topology
        .capability_limits
        .iter()
        .find(|(_, limit)| **limit > 0)
        .map(|(name, _)| name)
        .expect("fixture registers owned capacity");
    *wrong.final_capabilities.get_mut(available_pool).unwrap() = 1;
    rejects(&wrong, "did not settle owned capacity");
    let mut wrong = raw.clone();
    wrong.class_counters.remove("io");
    rejects(&wrong, "class catalog differs");
    let mut wrong = raw.clone();
    let mut foreign = wrong.class_counters["io"].clone();
    foreign.admitted = 0;
    foreign.started = 0;
    foreign.terminated = 0;
    wrong
        .class_counters
        .insert("foreign".into(), foreign.clone());
    rejects(&wrong, "class catalog differs");
    let mut wrong = raw.clone();
    wrong.class_counters.remove("io");
    wrong.class_counters.insert("foreign".into(), foreign);
    rejects(&wrong, "class catalog differs");
    let mut wrong = raw.clone();
    wrong.class_counters.get_mut("io").unwrap().queued = 1;
    rejects(&wrong, "did not settle owned capacity");
    let mut wrong = raw.clone();
    wrong.class_counters.get_mut("io").unwrap().inflight = 1;
    rejects(&wrong, "did not settle owned capacity");
    let mut wrong = raw.clone();
    wrong.class_counters.get_mut("io").unwrap().terminated = 5;
    rejects(&wrong, "did not settle owned capacity");
    let mut wrong = raw.clone();
    let io = wrong.class_counters.get_mut("io").unwrap();
    io.admitted = 5;
    io.terminated = 5;
    rejects(&wrong, "class counters");
    let mut wrong = raw.clone();
    wrong.invalid_reason = Some("caller task failed".into());
    rejects(&wrong, "invalid closed-loop run");

    let mut rejected = raw.clone();
    rejected.records[0].as_mut().unwrap().outcome = ResponseOutcome::Rejected {
        verdict: AdmissionVerdict::QueueFull {
            retry_after_ms: None,
        },
    };
    let io = rejected.class_counters.get_mut("io").unwrap();
    io.admitted = 5;
    io.started = 5;
    io.terminated = 5;
    rejected
        .validate_against(&scenario)
        .expect("typed caller rejection has no started class work");

    let mut wrong_capabilities = raw.clone();
    wrong_capabilities.final_capabilities =
        std::collections::BTreeMap::from([("unregistered".into(), 0)]);
    assert!(wrong_capabilities
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("capability catalog")));

    raw.records[0].as_mut().expect("first record").outcome = ResponseOutcome::Cancelled;
    assert!(raw
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("unexpected caller outcome")));
    raw.records[0].as_mut().expect("first record").outcome = ResponseOutcome::Success;
    raw.records[1].as_mut().expect("second record").submitted_ns = 0;
    assert!(raw
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("invalid caller sequence")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closed_loop_raw_counters_bind_successes_to_the_template_class_without_overflow() {
    let mut scenario = ClosedLoopScenario::from_json(FIXTURE).expect("fixture");
    let mut idle = scenario.classes[0].clone();
    idle.name = "idle".into();
    scenario.classes.push(idle);
    let (raw, _) = run_closed_loop_with_topology(&scenario)
        .await
        .expect("six caller responses across a two-class catalog");
    raw.validate_against(&scenario)
        .expect("valid two-class raw");
    assert_eq!(raw.class_counters["io"].started, 6);
    assert_eq!(raw.class_counters["idle"].started, 0);

    let mut misplaced = raw.clone();
    let io = misplaced.class_counters.get_mut("io").unwrap();
    io.admitted = 5;
    io.started = 5;
    io.terminated = 5;
    let idle = misplaced.class_counters.get_mut("idle").unwrap();
    idle.admitted = 1;
    idle.started = 1;
    idle.terminated = 1;
    assert!(misplaced
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("class counters")));

    let mut wrapped = raw;
    let io = wrapped.class_counters.get_mut("io").unwrap();
    io.admitted = u128::MAX;
    io.started = u128::MAX;
    io.terminated = u128::MAX;
    let idle = wrapped.class_counters.get_mut("idle").unwrap();
    idle.admitted = 7;
    idle.started = 7;
    idle.terminated = 7;
    let json = serde_json::to_vec(&wrapped).expect("serialize public raw");
    let restored: ClosedLoopRun = serde_json::from_slice(&json).expect("decode u128 counters");
    assert_eq!(restored.class_counters["io"].started, u128::MAX);
    let verdict = std::panic::catch_unwind(|| restored.validate_against(&scenario));
    assert!(verdict
        .expect("counter verification must not panic")
        .is_err_and(|error| error.contains("class counters")));
}

#[test]
fn closed_loop_preflight_accepts_inclusive_limits_and_rejects_adjacent_values() {
    let fixture = ClosedLoopScenario::from_json(FIXTURE).expect("fixture");
    let admitted = |scenario: &ClosedLoopScenario| {
        scenario.validate().expect("valid direct scenario");
        let json = serde_json::to_vec(scenario).expect("serialize scenario");
        ClosedLoopScenario::from_json(&json).expect("valid public JSON scenario");
    };
    let rejected = |scenario: &ClosedLoopScenario, expected: &str| {
        assert!(scenario
            .validate()
            .is_err_and(|error| error.contains(expected)));
        let json = serde_json::to_vec(scenario).expect("serialize scenario");
        assert!(ClosedLoopScenario::from_json(&json).is_err_and(|error| error.contains(expected)));
    };

    let mut scenario = fixture.clone();
    for version in [0, 2] {
        scenario.schema_version = version;
        rejected(&scenario, "unsupported closed-loop scenario version");
    }
    scenario = fixture.clone();
    scenario.concurrency = 1;
    scenario.iterations_per_slot = 1;
    scenario.max_run_ms = 1;
    admitted(&scenario);

    scenario.iterations_per_slot = 1_000_000;
    scenario.max_run_ms = 3_600_000;
    admitted(&scenario); // Admission only; this test never runs the million calls.
    scenario.iterations_per_slot += 1;
    rejected(&scenario, "out of bounds");

    scenario = fixture.clone();
    scenario.concurrency = 10_000;
    scenario.iterations_per_slot = 1;
    admitted(&scenario);
    scenario.concurrency += 1;
    rejected(&scenario, "out of bounds");

    scenario = fixture.clone();
    scenario.concurrency = 0;
    rejected(&scenario, "out of bounds");
    scenario.concurrency = 1;
    scenario.iterations_per_slot = 0;
    rejected(&scenario, "out of bounds");

    scenario = fixture.clone();
    scenario.max_run_ms = 0;
    rejected(&scenario, "out of bounds");
    scenario.max_run_ms = 3_600_001;
    rejected(&scenario, "out of bounds");

    scenario = fixture.clone();
    scenario.concurrency = usize::MAX;
    scenario.iterations_per_slot = 2;
    rejected(&scenario, "population overflow");

    scenario = fixture;
    scenario.template.class = "unregistered".into();
    rejected(&scenario, "unknown class");

    let mut invalid_json: serde_json::Value = serde_json::from_slice(FIXTURE).expect("JSON");
    invalid_json["unexpected_field"] = serde_json::json!(true);
    assert!(ClosedLoopScenario::from_json(&serde_json::to_vec(&invalid_json).unwrap()).is_err());
}
