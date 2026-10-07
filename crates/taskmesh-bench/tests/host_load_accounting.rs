use std::collections::BTreeMap;

use serde_json::{json, Value};
use taskmesh::AdmissionVerdict;
use taskmesh_bench::host_load::{
    producer_decision, run_host_scenario_with_fault, CallerCounts, CallerDisposition,
    ClassCounters, HostHarnessFault, HostRunStatus, ProducerDecision, RawHostRecord, RawHostRun,
    ResponseOutcome, HOST_RAW_VERSION,
};
use taskmesh_bench::host_scenarios::{HostPath, HostScenario};

fn valid_scenario() -> Value {
    json!({
        "schema_version": 1,
        "id": "h1-accounting",
        "load": {"warmup_ms": 0, "injection_ms": 10, "interval_ms": 1,
                 "snapshot_ms": 0,
                 "settlement_ms": 100,
                 "max_outstanding": 4, "max_records": 8},
        "topology": {"cpu_workers": 2, "blocking_threads": 2,
                     "shared_blocking_limit": 2, "cpu_units": 10, "memory_units": 10},
        "classes": [{"name": "c", "slo_ms": 50, "max_inflight": 2, "max_queue_depth": 1,
                     "cpu_units": 1, "memory_units": 1, "overflow": "reject"}],
        "offers": [{"send_time_ns": 0, "class": "c", "path": "io",
                    "body": {"kind": "noop"}}]
    })
}

#[test]
fn scenario_schema_fails_before_timing_on_invalid_boundary() {
    let valid = valid_scenario();
    HostScenario::from_json(&serde_json::to_vec(&valid).unwrap()).expect("valid fixture");
    for (path, value) in [
        ("unknown key", {
            let mut v = valid.clone();
            v["load"]["surprise"] = json!(1);
            v
        }),
        ("unknown class", {
            let mut v = valid.clone();
            v["offers"][0]["class"] = json!("ghost");
            v
        }),
        ("zero WFQ weight", {
            let mut v = valid.clone();
            v["classes"][0]["fairness"] = json!({"kind": "weighted_fair", "weight": 0});
            v
        }),
        ("path/body mismatch", {
            let mut v = valid.clone();
            v["offers"][0]["body"] = json!({"kind": "cpu_spin", "iterations": 10});
            v
        }),
        ("unbounded async body", {
            let mut v = valid.clone();
            v["offers"][0]["body"] = json!({"kind": "async_sleep", "millis": 101});
            v
        }),
        ("zero outstanding", {
            let mut v = valid.clone();
            v["load"]["max_outstanding"] = json!(0);
            v
        }),
        ("unbounded record allocation", {
            let mut v = valid.clone();
            v["load"]["max_records"] = json!(1_000_001);
            v
        }),
        ("too many intervals", {
            let mut v = valid.clone();
            v["load"]["injection_ms"] = json!(10_001);
            v
        }),
        ("invalid topology", {
            let mut v = valid.clone();
            v["topology"]["shared_blocking_limit"] = json!(0);
            v
        }),
        ("requested stack without finite slot", {
            let mut v = valid.clone();
            v["offers"][0]["path"] = json!("requested_stack_async");
            v["offers"][0]["stack_size_bytes"] = json!(2_097_152);
            v
        }),
        ("requested stack without size", {
            let mut v = valid.clone();
            v["topology"]["large_stack_slots"] = json!(1);
            v["offers"][0]["path"] = json!("requested_stack_async");
            v
        }),
        ("stack size on IO path", {
            let mut v = valid.clone();
            v["offers"][0]["stack_size_bytes"] = json!(2_097_152);
            v
        }),
        ("outside injection", {
            let mut v = valid.clone();
            v["offers"][0]["send_time_ns"] = json!(10_000_000);
            v
        }),
        ("out-of-order integer schedule", {
            let mut v = valid.clone();
            v["offers"] = json!([
                {"send_time_ns": 2, "class": "c", "path": "io", "body": {"kind": "noop"}},
                {"send_time_ns": 1, "class": "c", "path": "io", "body": {"kind": "noop"}}
            ]);
            v
        }),
    ] {
        let expected = match path {
            "unknown key" => "unknown field",
            "unknown class" => "unknown class",
            "zero WFQ weight" => "WFQ weight must be positive",
            "path/body mismatch" => "path/body mismatch",
            "unbounded async body" => "body exceeds settlement window",
            "zero outstanding" => "max_outstanding and max_records must be nonzero",
            "unbounded record allocation" => "record/outstanding bounds are invalid",
            "too many intervals" => "scenario may emit at most 10000 report intervals",
            "invalid topology" => "fixed physical topology capacities must be nonzero",
            "requested stack without finite slot" => "finite large_stack_slots limit",
            "requested stack without size" => "invalid requested stack size",
            "stack size on IO path" => "stack size on non-stack path",
            "outside injection" => "outside injection window",
            "out-of-order integer schedule" => "intended times are not ordered",
            _ => unreachable!("all malformed fixtures have a named contract"),
        };
        assert!(
            HostScenario::from_json(&serde_json::to_vec(&value).unwrap())
                .is_err_and(|error| error.contains(expected)),
            "{path}"
        );
    }
    let mut simultaneous = valid;
    simultaneous["offers"].as_array_mut().unwrap().push(json!({
        "send_time_ns": 0, "class": "c", "path": "io", "body": {"kind": "noop"}
    }));
    HostScenario::from_json(&serde_json::to_vec(&simultaneous).unwrap())
        .expect("simultaneous arrivals remain distinct intended IDs");
}

fn row(id: usize, disposition: CallerDisposition) -> RawHostRecord {
    let not_submitted = disposition == CallerDisposition::NotSubmitted;
    RawHostRecord {
        id,
        class: "c".into(),
        path: HostPath::Io,
        intended_ns: id as u64,
        scheduled_lag_ns: Some(0),
        submitted_ns: (!not_submitted).then_some(id as u64),
        body_started_ns: None,
        caller_response_ns: matches!(disposition, CallerDisposition::Responded { .. })
            .then_some(id as u64 + 1),
        caller_drop_ns: (disposition == CallerDisposition::CallerDropped).then_some(id as u64 + 1),
        body_finished_ns: None,
        disposition,
    }
}

#[test]
fn raw_rows_count_every_intended_offer_without_inventing_latency() {
    let mut success = row(
        0,
        CallerDisposition::Responded {
            outcome: ResponseOutcome::Success,
        },
    );
    success.body_started_ns = Some(0);
    success.body_finished_ns = Some(5);
    success.caller_response_ns = Some(6);
    let mut records = vec![
        success,
        row(
            1,
            CallerDisposition::Responded {
                outcome: ResponseOutcome::Rejected {
                    verdict: AdmissionVerdict::CpuSaturated {
                        retry_after_ms: None,
                    },
                },
            },
        ),
        row(
            2,
            CallerDisposition::Responded {
                outcome: ResponseOutcome::Deadline,
            },
        ),
        row(
            3,
            CallerDisposition::Responded {
                outcome: ResponseOutcome::Cancelled,
            },
        ),
        row(4, CallerDisposition::CallerDropped),
        row(5, CallerDisposition::NotSubmitted),
        row(6, CallerDisposition::UnansweredAtSettlement),
    ];
    for (index, record) in records.iter_mut().enumerate().take(4).skip(1) {
        record.caller_response_ns = Some(10 + index as u64);
    }
    records[2].body_started_ns = Some(2);
    records[2].body_finished_ns = Some(20); // worker can finish after deadline response
    let settlement = CallerCounts::from_records(&records).unwrap();
    assert_eq!(
        (
            settlement.intended,
            settlement.submitted,
            settlement.not_submitted,
            settlement.responded,
            settlement.caller_dropped,
            settlement.unanswered_at_settlement
        ),
        (7, 6, 1, 4, 1, 1)
    );
    let run = RawHostRun {
        schema_version: HOST_RAW_VERSION,
        status: HostRunStatus::Complete,
        scenario_id: "synthetic".into(),
        injection_window_ns: 10,
        interval_ns: 1,
        snapshot_cadence_ns: 0,
        snapshots: Vec::new(),
        cut: CallerCounts {
            intended: 7,
            submitted: 6,
            not_submitted: 1,
            responded: 1,
            caller_dropped: 1,
            waiting: 4,
            unanswered_at_settlement: 0,
        },
        settlement,
        records,
        class_counters: BTreeMap::from([("c".into(), ClassCounters::default())]),
        final_capabilities: BTreeMap::from([("cpu".into(), 0)]),
        drain_ok: true,
        conservation_ok: true,
    };
    run.validate().unwrap();
    assert_eq!(CallerCounts::at_cut(&run.records, 10).unwrap(), run.cut);
    let roundtrip: RawHostRun = serde_json::from_slice(&serde_json::to_vec(&run).unwrap()).unwrap();
    roundtrip.validate().unwrap();
}

#[test]
fn malformed_event_order_and_population_fail_closed() {
    let mut good = row(
        0,
        CallerDisposition::Responded {
            outcome: ResponseOutcome::Success,
        },
    );
    good.body_started_ns = Some(0);
    good.body_finished_ns = Some(1);
    CallerCounts::from_records(&[good.clone()]).expect("baseline row is valid");
    let mut duplicate = good.clone();
    duplicate.id = 0;
    assert!(CallerCounts::from_records(&[good.clone(), duplicate])
        .is_err_and(|error| error.contains("missing, duplicated or reordered id")));

    let mut missing = good.clone();
    missing.id = 1;
    assert!(CallerCounts::from_records(&[missing])
        .is_err_and(|error| error.contains("missing, duplicated or reordered id")));

    let mut reversed_time = good.clone();
    reversed_time.submitted_ns = Some(10);
    reversed_time.body_started_ns = Some(10);
    reversed_time.body_finished_ns = Some(10);
    assert!(CallerCounts::from_records(&[reversed_time])
        .is_err_and(|error| error.contains("response precedes submit")));

    let mut dropped_with_response = good;
    dropped_with_response.disposition = CallerDisposition::CallerDropped;
    dropped_with_response.caller_drop_ns = Some(2);
    assert!(CallerCounts::from_records(&[dropped_with_response])
        .is_err_and(|error| error.contains("invalid drop timestamps")));

    let mut body_before_submit = row(0, CallerDisposition::CallerDropped);
    body_before_submit.body_started_ns = Some(0);
    body_before_submit.submitted_ns = Some(1);
    assert!(CallerCounts::from_records(&[body_before_submit])
        .is_err_and(|error| error.contains("body start precedes submit")));
}

#[test]
fn pure_producer_cap_and_lag_have_exact_boundary() {
    assert_eq!(
        producer_decision(10, 8, 0, 1),
        ProducerDecision::Submit { lag_ns: 0 }
    );
    assert_eq!(
        producer_decision(10, 12, 0, 1),
        ProducerDecision::Submit { lag_ns: 2 }
    );
    assert_eq!(
        producer_decision(10, 12, 1, 1),
        ProducerDecision::NotSubmitted { lag_ns: 2 }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn complete_raw_binds_scenario_identity_and_path() {
    let scenario =
        HostScenario::from_json(&serde_json::to_vec(&valid_scenario()).unwrap()).unwrap();
    let good = run_host_scenario_with_fault(&scenario, None)
        .await
        .expect("diagnostic run retains raw evidence");
    good.validate_against(&scenario).expect("matching raw run");
    assert_eq!(good.records.len(), 1);

    let mut wrong_scenario = good.clone();
    wrong_scenario.scenario_id = "different-scenario".into();
    wrong_scenario
        .validate()
        .expect("internally coherent raw run");
    assert_eq!(
        wrong_scenario.validate_against(&scenario).unwrap_err(),
        "raw scenario identity or offer population differs"
    );

    let mut wrong_path = good;
    wrong_path.records[0].path = HostPath::Blocking;
    wrong_path.validate().expect("internally coherent raw run");
    assert_eq!(
        wrong_path.validate_against(&scenario).unwrap_err(),
        "row 0: offer identity differs from scenario"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn complete_raw_binds_offer_population() {
    let scenario =
        HostScenario::from_json(&serde_json::to_vec(&valid_scenario()).unwrap()).unwrap();
    let good = run_host_scenario_with_fault(&scenario, None)
        .await
        .expect("diagnostic run retains raw evidence");
    good.validate_against(&scenario).expect("matching raw run");
    assert_eq!(good.records.len(), 1);

    let mut missing_offer = good;
    missing_offer.records.clear();
    missing_offer.settlement = CallerCounts::from_records(&missing_offer.records).unwrap();
    missing_offer.cut =
        CallerCounts::at_cut(&missing_offer.records, missing_offer.injection_window_ns).unwrap();
    *missing_offer.class_counters.get_mut("c").unwrap() = ClassCounters::default();
    missing_offer
        .validate()
        .expect("internally coherent raw run");
    assert_eq!(
        missing_offer.validate_against(&scenario).unwrap_err(),
        "raw scenario identity or offer population differs"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn complete_raw_rejects_failed_settlement_and_retained_ownership() {
    let mut scenario =
        HostScenario::from_json(&serde_json::to_vec(&valid_scenario()).unwrap()).unwrap();
    scenario.load.snapshot_ms = 5;
    let good = run_host_scenario_with_fault(&scenario, None)
        .await
        .expect("diagnostic run retains raw evidence");
    good.validate_against(&scenario).expect("settled baseline");
    assert_eq!(good.snapshots.len(), 2);

    let mut wrong_schema = good.clone();
    wrong_schema.schema_version = HOST_RAW_VERSION + 1;
    let expected = "invalid raw host schema/window";
    assert_eq!(wrong_schema.validate().unwrap_err(), expected);
    assert_eq!(
        wrong_schema.validate_against(&scenario).unwrap_err(),
        expected
    );

    let mut missing_snapshot = good.clone();
    missing_snapshot.snapshots.pop();
    let expected = "snapshot cadence/sample population differs";
    assert_eq!(missing_snapshot.validate().unwrap_err(), expected);
    assert_eq!(
        missing_snapshot.validate_against(&scenario).unwrap_err(),
        expected
    );

    let mut extra_snapshot_class = good.clone();
    let sample = &mut extra_snapshot_class.snapshots[0];
    sample
        .classes
        .insert("ghost".into(), sample.classes["c"].clone());
    extra_snapshot_class
        .validate()
        .expect("snapshot class catalog is scenario-bound");
    assert_eq!(
        extra_snapshot_class
            .validate_against(&scenario)
            .unwrap_err(),
        "snapshot 0: class catalog differs"
    );

    let mut invalid_snapshot_gauge = good.clone();
    let class = invalid_snapshot_gauge.snapshots[0]
        .classes
        .get_mut("c")
        .unwrap();
    class.accepted = class.inflight + 1;
    invalid_snapshot_gauge
        .validate()
        .expect("snapshot class gauges are scenario-bound");
    assert_eq!(
        invalid_snapshot_gauge
            .validate_against(&scenario)
            .unwrap_err(),
        "snapshot 0: invalid class gauges"
    );

    let mut extra_counter_class = good.clone();
    extra_counter_class
        .class_counters
        .insert("ghost".into(), ClassCounters::default());
    extra_counter_class
        .validate()
        .expect("class counter catalog is scenario-bound");
    assert_eq!(
        extra_counter_class.validate_against(&scenario).unwrap_err(),
        "class counter population differs from scenario"
    );

    for case in [
        "drain",
        "conservation",
        "inflight",
        "queued",
        "capability",
        "inventory",
    ] {
        let mut raw = good.clone();
        match case {
            "drain" => raw.drain_ok = false,
            "conservation" => raw.conservation_ok = false,
            "inflight" => raw.class_counters.get_mut("c").unwrap().inflight = 1,
            "queued" => raw.class_counters.get_mut("c").unwrap().queued = 1,
            "capability" => *raw.final_capabilities.values_mut().next().unwrap() = 1,
            "inventory" => raw.final_capabilities.clear(),
            _ => unreachable!(),
        }
        assert_eq!(raw.status, HostRunStatus::Complete);
        let expected = "complete host run did not settle owned engine capacity";
        assert_eq!(raw.validate().unwrap_err(), expected, "{case}");
        assert_eq!(
            raw.validate_against(&scenario).unwrap_err(),
            expected,
            "{case}"
        );
        // Failed evidence stays serializable for diagnosis, including callers
        // that retain it before invoking the validation boundary.
        let bytes = serde_json::to_vec(&raw).expect("failed raw is retainable");
        let restored: RawHostRun = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            restored.validate_against(&scenario).unwrap_err(),
            expected,
            "{case}"
        );
    }
    let mut forged = good.clone();
    forged.final_capabilities = BTreeMap::from([("unregistered".into(), 0)]);
    assert!(forged
        .validate_against(&scenario)
        .unwrap_err()
        .contains("capability catalog"));
    let mut forged = good.clone();
    forged.snapshots[0].capabilities = BTreeMap::from([("unregistered".into(), 0)]);
    assert!(forged
        .validate_against(&scenario)
        .unwrap_err()
        .contains("capability catalog"));
    let mut forged = good;
    *forged.snapshots[0]
        .capabilities
        .values_mut()
        .next()
        .unwrap() = u32::MAX;
    assert!(forged
        .validate_against(&scenario)
        .unwrap_err()
        .contains("resolved limit"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn class_admissions_cannot_exceed_submitted_rows() {
    let mut declaration = valid_scenario();
    let mut unused = declaration["classes"][0].clone();
    unused["name"] = json!("unused");
    declaration["classes"].as_array_mut().unwrap().push(unused);
    let scenario = HostScenario::from_json(&serde_json::to_vec(&declaration).unwrap()).unwrap();
    let good = run_host_scenario_with_fault(&scenario, None)
        .await
        .expect("diagnostic run retains raw evidence");
    good.validate_against(&scenario).expect("settled baseline");
    assert_eq!(good.settlement.submitted, 1);

    let mut excess = good.clone();
    let active = excess.class_counters.get_mut("c").unwrap();
    active.admitted = 2;
    active.terminated = 2;
    assert!(excess
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("counters differ from raw rows")));

    let mut unused_admission = good.clone();
    let idle = unused_admission.class_counters.get_mut("unused").unwrap();
    idle.admitted = 1;
    idle.terminated = 1;
    assert!(unused_admission
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("counters differ from raw rows")));

    let mut not_submitted = good.clone();
    let row = &mut not_submitted.records[0];
    row.submitted_ns = None;
    row.body_started_ns = None;
    row.body_finished_ns = None;
    row.caller_response_ns = None;
    row.disposition = CallerDisposition::NotSubmitted;
    not_submitted.settlement = CallerCounts::from_records(&not_submitted.records).unwrap();
    not_submitted.cut =
        CallerCounts::at_cut(&not_submitted.records, not_submitted.injection_window_ns).unwrap();
    *not_submitted.class_counters.get_mut("c").unwrap() = ClassCounters::default();
    not_submitted
        .validate_against(&scenario)
        .expect("a missed offer contributes no engine admission");
    let active = not_submitted.class_counters.get_mut("c").unwrap();
    active.admitted = 1;
    active.terminated = 1;
    assert!(not_submitted
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("counters differ from raw rows")));

    let mut cancelled_before_start = good;
    let row = &mut cancelled_before_start.records[0];
    row.body_started_ns = None;
    row.body_finished_ns = None;
    row.disposition = CallerDisposition::Responded {
        outcome: ResponseOutcome::Cancelled,
    };
    let active = cancelled_before_start.class_counters.get_mut("c").unwrap();
    active.started = 0;
    let mut cancel_scenario = scenario.clone();
    cancel_scenario.offers[0].cancel_after_ms = Some(0);
    cancel_scenario
        .validate()
        .expect("declared immediate cancellation");
    cancelled_before_start
        .validate_against(&cancel_scenario)
        .expect("an admitted job may terminate before its body starts");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn body_start_requires_a_corresponding_class_admission() {
    let scenario =
        HostScenario::from_json(&serde_json::to_vec(&valid_scenario()).unwrap()).unwrap();
    let mut raw = run_host_scenario_with_fault(&scenario, None)
        .await
        .expect("diagnostic run retains raw evidence");
    raw.validate_against(&scenario).expect("settled baseline");
    assert_eq!(raw.class_counters["c"].started, 1);
    assert!(raw.records[0].body_started_ns.is_some());

    let counters = raw.class_counters.get_mut("c").unwrap();
    counters.admitted = 0;
    counters.terminated = 0;
    // Started still matches the body-start row, admission is within the
    // submitted bound, and the terminal ledger remains settled.
    assert!(raw
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("counters differ from raw rows")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn producer_and_sampler_failures_return_bounded_invalid_raw() {
    let scenario =
        HostScenario::from_json(&serde_json::to_vec(&valid_scenario()).unwrap()).unwrap();
    for (fault, expected) in [
        (HostHarnessFault::ProducerBeforeOffer(0), "producer"),
        (HostHarnessFault::SamplerPanic, "sampler"),
    ] {
        let mut scenario = scenario.clone();
        if fault == HostHarnessFault::SamplerPanic {
            scenario.load.snapshot_ms = 5;
        }
        let raw = run_host_scenario_with_fault(&scenario, Some(fault))
            .await
            .expect("post-start failures retain raw rows");
        assert_eq!(raw.records.len(), scenario.offers.len());
        assert!(matches!(raw.status, HostRunStatus::Invalid { .. }));
        assert!(raw
            .validate_against(&scenario)
            .unwrap_err()
            .contains(expected));
        let encoded = serde_json::to_vec(&raw).unwrap();
        let decoded: RawHostRun = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.records.len(), scenario.offers.len());
        assert!(matches!(decoded.status, HostRunStatus::Invalid { .. }));
    }
}

#[test]
fn raw_record_dispositions_reject_conflicting_activity() {
    let reject = |case: &str, record: RawHostRecord, expected: &str| {
        assert_eq!(
            CallerCounts::from_records(&[record]).unwrap_err(),
            expected,
            "{case}"
        );
    };
    let quiet = row(0, CallerDisposition::NotSubmitted);
    CallerCounts::from_records(std::slice::from_ref(&quiet)).unwrap();
    type RecordChange = fn(&mut RawHostRecord);
    let changes: [(&str, RecordChange); 3] = [
        ("start", |r: &mut RawHostRecord| r.body_started_ns = Some(0)),
        ("response", |r: &mut RawHostRecord| {
            r.caller_response_ns = Some(1)
        }),
        ("drop", |r: &mut RawHostRecord| r.caller_drop_ns = Some(1)),
    ];
    for (case, change) in changes {
        let mut invalid = quiet.clone();
        change(&mut invalid);
        reject(case, invalid, "row 0: not-submitted offer has activity");
    }
    let responded = row(
        0,
        CallerDisposition::Responded {
            outcome: ResponseOutcome::Rejected {
                verdict: AdmissionVerdict::CpuSaturated {
                    retry_after_ms: None,
                },
            },
        },
    );
    CallerCounts::from_records(std::slice::from_ref(&responded)).unwrap();
    let mut missing_response = responded.clone();
    missing_response.caller_response_ns = None;
    reject(
        "missing response",
        missing_response,
        "row 0: invalid responded timestamps",
    );
    let mut response_with_drop = responded;
    response_with_drop.caller_drop_ns = Some(1);
    reject(
        "responded and dropped",
        response_with_drop,
        "row 0: invalid responded timestamps",
    );
    let dropped = row(0, CallerDisposition::CallerDropped);
    CallerCounts::from_records(std::slice::from_ref(&dropped)).unwrap();
    let mut missing_drop = dropped;
    missing_drop.caller_drop_ns = None;
    reject(
        "missing drop",
        missing_drop,
        "row 0: invalid drop timestamps",
    );
    let waiting = row(0, CallerDisposition::Waiting);
    CallerCounts::from_records(std::slice::from_ref(&waiting)).unwrap();
    let mut waiting_with_response = waiting.clone();
    waiting_with_response.caller_response_ns = Some(1);
    reject(
        "waiting with response",
        waiting_with_response,
        "row 0: invalid outstanding timestamps",
    );
    let mut waiting_with_drop = waiting;
    waiting_with_drop.caller_drop_ns = Some(1);
    reject(
        "waiting with drop",
        waiting_with_drop,
        "row 0: invalid outstanding timestamps",
    );
}

#[test]
fn raw_host_snapshots_and_admission_reject_inconsistent_fields() {
    use taskmesh_bench::host_load::{SampledClass, SnapshotSample};
    let mut declaration = valid_scenario();
    declaration["load"]["snapshot_ms"] = json!(5);
    let scenario = HostScenario::from_json(&serde_json::to_vec(&declaration).unwrap()).unwrap();
    let capabilities: BTreeMap<_, _> = scenario
        .resolved_topology()
        .unwrap()
        .capability_limits
        .keys()
        .cloned()
        .map(|key| (key, 0))
        .collect();
    let records = vec![row(0, CallerDisposition::NotSubmitted)];
    let settlement = CallerCounts::from_records(&records).unwrap();
    let cut = CallerCounts::at_cut(&records, 10_000_000).unwrap();
    let sample = |intended_ns| SnapshotSample {
        intended_ns,
        observed_ns: intended_ns,
        classes: BTreeMap::from([(
            "c".into(),
            SampledClass {
                inflight: 0,
                queued: 0,
                accepted: 0,
                running: 0,
                cpu_units_held: "0".into(),
                memory_units_held: "0".into(),
            },
        )]),
        capabilities: capabilities.clone(),
        conservation_ok: true,
    };
    let good = RawHostRun {
        schema_version: HOST_RAW_VERSION,
        status: HostRunStatus::Complete,
        scenario_id: scenario.id.clone(),
        injection_window_ns: 10_000_000,
        interval_ns: 1_000_000,
        snapshot_cadence_ns: 5_000_000,
        snapshots: vec![sample(0), sample(5_000_000)],
        cut,
        settlement,
        records,
        class_counters: BTreeMap::from([("c".into(), ClassCounters::default())]),
        final_capabilities: capabilities,
        drain_ok: true,
        conservation_ok: true,
    };
    good.validate_against(&scenario).unwrap();
    let mut wrong_intended = good.clone();
    wrong_intended.snapshots[1].intended_ns += 1;
    assert_eq!(
        wrong_intended.validate().unwrap_err(),
        "snapshot sample 1 is invalid"
    );
    let mut early_observation = good.clone();
    early_observation.snapshots[1].observed_ns = 0;
    assert_eq!(
        early_observation.validate().unwrap_err(),
        "snapshot sample 1 is invalid"
    );
    let mut false_sample_conservation = good.clone();
    false_sample_conservation.snapshots[0].conservation_ok = false;
    assert_eq!(
        false_sample_conservation.validate().unwrap_err(),
        "snapshot sample 0 is invalid"
    );
    for field in ["cpu", "memory"] {
        let mut wrong_gauge = good.clone();
        let gauge = wrong_gauge.snapshots[0].classes.get_mut("c").unwrap();
        match field {
            "cpu" => gauge.cpu_units_held = "invalid".into(),
            "memory" => gauge.memory_units_held = "invalid".into(),
            _ => unreachable!(),
        }
        assert_eq!(
            wrong_gauge.validate_against(&scenario).unwrap_err(),
            "snapshot 0: invalid class gauges",
            "{field}"
        );
    }
    let mut missing_started_counter = good;
    // A started body may be retained in the row even when the ledger lies.
    let record = &mut missing_started_counter.records[0];
    record.disposition = CallerDisposition::Responded {
        outcome: ResponseOutcome::Success,
    };
    record.submitted_ns = Some(0);
    record.body_started_ns = Some(0);
    record.body_finished_ns = Some(1);
    record.caller_response_ns = Some(1);
    missing_started_counter.settlement =
        CallerCounts::from_records(&missing_started_counter.records).unwrap();
    missing_started_counter.cut =
        CallerCounts::at_cut(&missing_started_counter.records, 10_000_000).unwrap();
    let counters = missing_started_counter.class_counters.get_mut("c").unwrap();
    counters.admitted = 1;
    counters.terminated = 1;
    assert_eq!(
        missing_started_counter
            .validate_against(&scenario)
            .unwrap_err(),
        "class c: counters differ from raw rows"
    );
}

#[test]
fn raw_host_window_rejects_zero_and_nondivisible_dimensions() {
    let scenario =
        HostScenario::from_json(&serde_json::to_vec(&valid_scenario()).unwrap()).unwrap();
    let final_capabilities = scenario
        .resolved_topology()
        .unwrap()
        .capability_limits
        .keys()
        .cloned()
        .map(|pool| (pool, 0))
        .collect();
    let good = RawHostRun {
        schema_version: HOST_RAW_VERSION,
        status: HostRunStatus::Complete,
        scenario_id: scenario.id,
        injection_window_ns: 10_000_000,
        interval_ns: 1_000_000,
        snapshot_cadence_ns: 0,
        snapshots: Vec::new(),
        cut: CallerCounts::default(),
        settlement: CallerCounts::default(),
        records: Vec::new(),
        class_counters: BTreeMap::from([("c".into(), ClassCounters::default())]),
        final_capabilities,
        drain_ok: true,
        conservation_ok: true,
    };
    good.validate()
        .expect("empty raw ledger is internally coherent");

    let mut zero_window = good.clone();
    zero_window.injection_window_ns = 0;
    assert_eq!(
        zero_window.validate().unwrap_err(),
        "invalid raw host schema/window"
    );

    let mut nondivisible_window = good;
    nondivisible_window.injection_window_ns = 11_000_000;
    nondivisible_window.interval_ns = 2_000_000;
    assert_eq!(
        nondivisible_window.validate().unwrap_err(),
        "invalid raw host schema/window"
    );
}
