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

    let mut coherent_wrong_counter = raw.clone();
    let counters = coherent_wrong_counter
        .class_counters
        .get_mut("local")
        .unwrap();
    counters.started = 3;
    counters.admitted = 3;
    counters.terminated = 3;
    assert_eq!(
        coherent_wrong_counter.validate_against(&scenario),
        Err("local class local: counters differ from rows".into())
    );

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

fn fixed_local_raw(scenario: &LocalHostScenario) -> taskmesh_bench::local_host::LocalHostRun {
    use std::collections::BTreeMap;
    use taskmesh_bench::host_load::{ClassCounters, SampledClass, SnapshotSample};
    use taskmesh_bench::local_host::{LocalHostRun, LocalRecord, LOCAL_HOST_VERSION};
    let caps: BTreeMap<_, _> = scenario
        .resolved_topology()
        .unwrap()
        .capability_limits
        .keys()
        .cloned()
        .map(|key| (key, 0))
        .collect();
    let cadence = scenario.load.snapshot_ms * 1_000_000;
    let sample_count = if cadence == 0 {
        0
    } else {
        scenario.load.injection_ms / scenario.load.snapshot_ms
    };
    let snapshots = (0..sample_count)
        .map(|index| {
            let intended_ns = index * cadence;
            SnapshotSample {
                intended_ns,
                observed_ns: intended_ns,
                classes: BTreeMap::from([(
                    "local".into(),
                    SampledClass {
                        inflight: 0,
                        queued: 0,
                        accepted: 0,
                        running: 0,
                        cpu_units_held: "0".into(),
                        memory_units_held: "0".into(),
                    },
                )]),
                capabilities: caps.clone(),
                conservation_ok: true,
            }
        })
        .collect();
    LocalHostRun {
        schema_version: LOCAL_HOST_VERSION,
        mode: "caller_affine_local_open_loop".into(),
        scenario_id: scenario.id.clone(),
        injection_window_ns: scenario.load.injection_ms * 1_000_000,
        snapshot_cadence_ns: cadence,
        snapshots,
        records: scenario
            .offers
            .iter()
            .enumerate()
            .map(|(id, offer)| LocalRecord {
                id,
                class: offer.class.clone(),
                intended_ns: offer.send_time_ns,
                pacer_observed_ns: offer.send_time_ns,
                scheduled_lag_ns: 0,
                submitted_ns: Some(offer.send_time_ns),
                body_started_ns: Some(offer.send_time_ns),
                body_finished_ns: Some(offer.send_time_ns),
                response_ns: Some(offer.send_time_ns),
                caller_drop_ns: None,
                outcome: Some(ResponseOutcome::Success),
            })
            .collect(),
        class_counters: BTreeMap::from([(
            "local".into(),
            ClassCounters {
                admitted: 2,
                started: 2,
                terminated: 2,
                inflight: 0,
                queued: 0,
            },
        )]),
        final_capabilities: caps,
        drain_ok: true,
        conservation_ok: true,
        invalid_reason: None,
    }
}

#[test]
fn local_raw_identity_and_ledger_reject_inconsistent_fields() {
    let scenario = LocalHostScenario::from_json(FIXTURE).unwrap();
    let good = fixed_local_raw(&scenario);
    good.validate_against(&scenario).unwrap();
    let expect = |case: &str, raw: taskmesh_bench::local_host::LocalHostRun, error: &str| {
        assert_eq!(
            raw.validate_against(&scenario).unwrap_err(),
            error,
            "{case}"
        );
    };
    let header = "local raw identity or population differs";
    let mut schema = good.clone();
    schema.schema_version += 1;
    expect("raw schema", schema, header);
    let mut mode = good.clone();
    mode.mode = "other".into();
    expect("mode", mode, header);
    let mut window = good.clone();
    window.injection_window_ns += 1;
    expect("window", window, header);
    let mut cadence = good.clone();
    cadence.snapshot_cadence_ns = cadence.injection_window_ns + 1;
    expect("cadence", cadence, header);
    let mut undrained = good.clone();
    undrained.drain_ok = false;
    expect(
        "drain",
        undrained,
        "local host did not settle or class catalog differs",
    );
    let mut unconserved = good.clone();
    unconserved.conservation_ok = false;
    expect(
        "conservation",
        unconserved,
        "local host did not settle or class catalog differs",
    );
    let mut extra_class = good.clone();
    extra_class
        .class_counters
        .insert("ghost".into(), Default::default());
    expect(
        "class catalog",
        extra_class,
        "local host did not settle or class catalog differs",
    );
    let mut held_capacity = good.clone();
    *held_capacity
        .final_capabilities
        .values_mut()
        .next()
        .expect("registered capability") = 1;
    expect(
        "held final capacity",
        held_capacity,
        "local host did not settle or class catalog differs",
    );
    let ledger = "local class local: counters differ from rows";
    let mut excess_admission = good.clone();
    let c = excess_admission.class_counters.get_mut("local").unwrap();
    c.admitted = 3;
    c.terminated = 3;
    expect("admitted differs from started", excess_admission, ledger);
    let mut excess_termination = good.clone();
    excess_termination
        .class_counters
        .get_mut("local")
        .unwrap()
        .terminated = 3;
    expect(
        "terminated differs from admitted",
        excess_termination,
        ledger,
    );
    let mut inflight = good.clone();
    inflight.class_counters.get_mut("local").unwrap().inflight = 1;
    expect("inflight", inflight, ledger);
    let mut queued = good;
    queued.class_counters.get_mut("local").unwrap().queued = 1;
    expect("queued", queued, ledger);
}

#[test]
fn local_snapshot_observation_catalog_and_gauges_are_independent() {
    let mut scenario = LocalHostScenario::from_json(FIXTURE).unwrap();
    scenario.load.snapshot_ms = 10;
    let good = fixed_local_raw(&scenario);
    good.validate_against(&scenario).unwrap();
    let expect = |case: &str, raw: taskmesh_bench::local_host::LocalHostRun, error: &str| {
        assert_eq!(
            raw.validate_against(&scenario).unwrap_err(),
            error,
            "{case}"
        );
    };
    let observation = "local Snapshot 1: invalid identity or observation";
    let mut early = good.clone();
    early.snapshots[1].observed_ns = 0;
    expect("observed before intended", early, observation);
    let mut reversed = good.clone();
    reversed.snapshots[0].observed_ns = 20_000_000;
    expect("observed before last", reversed, observation);
    let mut unconserved = good.clone();
    unconserved.snapshots[0].conservation_ok = false;
    expect(
        "sample conservation",
        unconserved,
        "local Snapshot 0: invalid identity or observation",
    );
    let mut extra_class = good.clone();
    let gauge = extra_class.snapshots[0].classes["local"].clone();
    extra_class.snapshots[0]
        .classes
        .insert("ghost".into(), gauge);
    expect(
        "extra sample class",
        extra_class,
        "local Snapshot 0: invalid identity or observation",
    );
    let mut replaced_class = good.clone();
    let gauge = replaced_class.snapshots[0].classes.remove("local").unwrap();
    replaced_class.snapshots[0]
        .classes
        .insert("ghost".into(), gauge);
    expect(
        "same-size wrong sample class",
        replaced_class,
        "local Snapshot 0: invalid identity or observation",
    );
    for field in ["cpu", "memory"] {
        let mut invalid = good.clone();
        let gauge = invalid.snapshots[0].classes.get_mut("local").unwrap();
        match field {
            "cpu" => gauge.cpu_units_held = "invalid".into(),
            "memory" => gauge.memory_units_held = "invalid".into(),
            _ => unreachable!(),
        }
        expect(field, invalid, "local Snapshot 0: impossible class gauges");
    }
}

#[test]
fn local_dispositions_reject_conflicting_activity() {
    use taskmesh::AdmissionVerdict;
    let scenario = LocalHostScenario::from_json(FIXTURE).unwrap();
    let success = fixed_local_raw(&scenario);
    success.validate_against(&scenario).unwrap();
    let mut quiet = success.clone();
    let row = &mut quiet.records[1];
    row.submitted_ns = None;
    row.body_started_ns = None;
    row.body_finished_ns = None;
    row.response_ns = None;
    row.outcome = None;
    let c = quiet.class_counters.get_mut("local").unwrap();
    c.started = 1;
    c.admitted = 1;
    c.terminated = 1;
    quiet.validate_against(&scenario).unwrap();
    let mut wrong_unsubmitted_class = quiet.clone();
    wrong_unsubmitted_class.records[1].class = "ghost".into();
    assert_eq!(
        wrong_unsubmitted_class.validate_against(&scenario),
        Err("local row 1: offer identity differs".into()),
    );
    for (case, change) in [
        (
            "body start",
            (|r: &mut taskmesh_bench::local_host::LocalRecord| {
                r.body_started_ns = Some(r.pacer_observed_ns)
            }) as fn(&mut taskmesh_bench::local_host::LocalRecord),
        ),
        (
            "body finish",
            |r: &mut taskmesh_bench::local_host::LocalRecord| {
                r.body_finished_ns = Some(r.pacer_observed_ns)
            },
        ),
        (
            "response",
            |r: &mut taskmesh_bench::local_host::LocalRecord| {
                r.response_ns = Some(r.pacer_observed_ns)
            },
        ),
        (
            "outcome",
            |r: &mut taskmesh_bench::local_host::LocalRecord| {
                r.outcome = Some(ResponseOutcome::Cancelled)
            },
        ),
        (
            "caller drop",
            |r: &mut taskmesh_bench::local_host::LocalRecord| {
                r.caller_drop_ns = Some(r.pacer_observed_ns)
            },
        ),
    ] {
        let mut invalid = quiet.clone();
        change(&mut invalid.records[1]);
        assert_eq!(
            invalid.validate_against(&scenario).unwrap_err(),
            "local row 1: unsubmitted offer has activity",
            "{case}"
        );
    }
    let mut drop_scenario = scenario.clone();
    drop_scenario.offers[1].drop_after_ms = Some(1);
    let mut dropped = quiet.clone();
    dropped.records[1].submitted_ns = Some(10_000_000);
    dropped.records[1].caller_drop_ns = Some(10_000_000);
    dropped.validate_against(&drop_scenario).unwrap();
    let drop_error = "local row 1: invalid caller drop";
    assert_eq!(dropped.validate_against(&scenario).unwrap_err(), drop_error);
    let mut drop_before_submit = dropped.clone();
    drop_before_submit.records[1].caller_drop_ns = Some(9_999_999);
    assert_eq!(
        drop_before_submit
            .validate_against(&drop_scenario)
            .unwrap_err(),
        drop_error
    );
    let mut drop_with_response = dropped.clone();
    drop_with_response.records[1].response_ns = Some(10_000_000);
    assert_eq!(
        drop_with_response
            .validate_against(&drop_scenario)
            .unwrap_err(),
        drop_error
    );
    let mut drop_with_outcome = dropped.clone();
    drop_with_outcome.records[1].outcome = Some(ResponseOutcome::Cancelled);
    assert_eq!(
        drop_with_outcome
            .validate_against(&drop_scenario)
            .unwrap_err(),
        drop_error
    );
    let mut drop_with_finish = dropped.clone();
    drop_with_finish.records[1].body_finished_ns = Some(10_000_000);
    assert_eq!(
        drop_with_finish
            .validate_against(&drop_scenario)
            .unwrap_err(),
        drop_error
    );
    let mut start_before_submit = dropped.clone();
    start_before_submit.records[1].body_started_ns = Some(9_999_999);
    let c = start_before_submit.class_counters.get_mut("local").unwrap();
    c.started = 2;
    c.admitted = 2;
    c.terminated = 2;
    assert_eq!(
        start_before_submit
            .validate_against(&drop_scenario)
            .unwrap_err(),
        "local row 1: invalid dropped body start"
    );
    let mut late_finish = dropped.clone();
    late_finish.records[1].body_started_ns = Some(10_000_000);
    late_finish.records[1].body_finished_ns = Some(10_000_001);
    let c = late_finish.class_counters.get_mut("local").unwrap();
    c.started = 2;
    c.admitted = 2;
    c.terminated = 2;
    assert_eq!(
        late_finish.validate_against(&drop_scenario).unwrap_err(),
        drop_error
    );
    let mut rejected = quiet;
    rejected.records[1].submitted_ns = Some(10_000_000);
    rejected.records[1].response_ns = Some(10_000_000);
    rejected.records[1].outcome = Some(ResponseOutcome::Rejected {
        verdict: AdmissionVerdict::CpuSaturated {
            retry_after_ms: None,
        },
    });
    rejected.validate_against(&scenario).unwrap();
    rejected.records[1].body_started_ns = Some(10_000_000);
    let c = rejected.class_counters.get_mut("local").unwrap();
    c.started = 2;
    c.admitted = 2;
    c.terminated = 2;
    assert_eq!(
        rejected.validate_against(&scenario).unwrap_err(),
        "local row 1: rejected body started"
    );
}

#[test]
fn local_submission_and_terminal_events_respect_time_order() {
    let scenario = LocalHostScenario::from_json(FIXTURE).unwrap();
    let good = fixed_local_raw(&scenario);
    good.validate_against(&scenario).unwrap();
    let mut wrong_submit = good.clone();
    wrong_submit.records[1].submitted_ns = Some(9_999_999);
    assert_eq!(
        wrong_submit.validate_against(&scenario).unwrap_err(),
        "local row 1: invalid submit or lag"
    );
    let mut cancelled_scenario = scenario;
    cancelled_scenario.offers[1].cancel_after_ms = Some(1);
    let mut cancelled = good;
    cancelled.records[1].outcome = Some(ResponseOutcome::Cancelled);
    cancelled.validate_against(&cancelled_scenario).unwrap();
    let expect = |case: &str, raw: taskmesh_bench::local_host::LocalHostRun| {
        assert_eq!(
            raw.validate_against(&cancelled_scenario).unwrap_err(),
            "local row 1: invalid terminal body order",
            "{case}"
        );
    };
    let mut start_before_submit = cancelled.clone();
    start_before_submit.records[1].body_started_ns = Some(9_999_999);
    expect("start before submit", start_before_submit);
    let mut start_after_response = cancelled.clone();
    start_after_response.records[1].body_started_ns = Some(10_000_001);
    start_after_response.records[1].body_finished_ns = None;
    expect("start after response", start_after_response);
    let mut finish_before_start = cancelled.clone();
    finish_before_start.records[1].body_finished_ns = Some(9_999_999);
    expect("finish before start", finish_before_start);
    let mut finish_after_response = cancelled;
    finish_after_response.records[1].body_finished_ns = Some(10_000_001);
    expect("finish after response", finish_after_response);
}

#[test]
fn local_success_body_order_is_independent_from_terminal_ledger() {
    let scenario = LocalHostScenario::from_json(FIXTURE).expect("local fixture");
    let good = fixed_local_raw(&scenario);
    good.validate_against(&scenario)
        .expect("valid success baseline");
    let mut start_before_submit = good.clone();
    start_before_submit.records[1].body_started_ns = Some(9_999_999);
    assert_eq!(
        start_before_submit.validate_against(&scenario),
        Err("local row 1: impossible body order".into()),
    );
    let mut finish_before_start = good;
    finish_before_start.records[1].body_finished_ns = Some(9_999_999);
    assert_eq!(
        finish_before_start.validate_against(&scenario),
        Err("local row 1: impossible body order".into()),
    );
}
