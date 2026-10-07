//! The recorder-minimal host still executes public Taskmesh work but cannot
//! report per-request latency or body timestamps.

use serde_json::json;
use taskmesh_bench::host_load::{HostHarnessFault, HostRunStatus};
use taskmesh_bench::host_scenarios::HostScenario;
use taskmesh_bench::minimal_host::{run_minimal_host_scenario, MinimalDisposition, MinimalHostRun};

fn scenario() -> HostScenario {
    let fixture = json!({
        "schema_version": 1,
        "id": "minimal-recorder-contract",
        "load": {"warmup_ms": 5, "injection_ms": 100, "interval_ms": 10,
                 "snapshot_ms": 0, "settlement_ms": 500,
                 "max_outstanding": 8, "max_records": 8},
        "topology": {"cpu_workers": 2, "blocking_threads": 2,
                     "shared_blocking_limit": 2, "cpu_units": 8, "memory_units": 8},
        "classes": [{"name": "c", "slo_ms": 100, "max_inflight": 8,
                     "max_queue_depth": 0, "cpu_units": 1, "memory_units": 1,
                     "overflow": "reject"}],
        "offers": [
            {"send_time_ns": 0, "class": "c", "path": "io", "body": {"kind": "noop"}},
            {"send_time_ns": 10_000_000, "class": "c", "path": "blocking",
             "body": {"kind": "noop"}},
            {"send_time_ns": 20_000_000, "class": "c", "path": "cpu",
             "body": {"kind": "noop"}}
        ]
    });
    HostScenario::from_json(&serde_json::to_vec(&fixture).unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn minimal_recorder_conserves_intended_and_terminal_rows_without_latencies() {
    let scenario = scenario();
    assert_eq!(scenario.id, "minimal-recorder-contract");
    assert_eq!(scenario.offers.len(), 3);
    let (raw, topology) = run_minimal_host_scenario(&scenario, None).await.unwrap();
    raw.validate_against(&scenario).unwrap();
    assert_eq!(raw.scenario_id, "minimal-recorder-contract");
    assert_eq!(raw.records.len(), scenario.offers.len());
    assert_eq!(
        raw.counts.intended,
        raw.counts.submitted + raw.counts.not_submitted
    );
    assert_eq!(
        raw.counts.submitted,
        raw.counts.responded + raw.counts.caller_dropped + raw.counts.unanswered_at_settlement
    );
    assert_eq!(
        raw.completed_callers,
        raw.counts.responded + raw.counts.caller_dropped
    );
    assert!(!raw.response_latency_available);
    assert_eq!(raw.recorder_mode, "minimal");
    assert_eq!(raw.outstanding_final, 0);
    assert!(raw.final_capabilities.values().all(|held| *held == 0));
    assert_eq!(topology.schema_version, 1);
    for (id, row) in raw.records.iter().enumerate() {
        assert_eq!(row.id, id);
        assert_ne!(row.disposition, MinimalDisposition::Pending);
    }
    let round_trip: MinimalHostRun =
        serde_json::from_slice(&serde_json::to_vec(&raw).unwrap()).unwrap();
    round_trip.validate_against(&scenario).unwrap();

    let mut fabricated_latency = raw.clone();
    fabricated_latency.response_latency_available = true;
    assert!(fabricated_latency
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("minimal host scenario or mode differs")));
    let mut wrong_count = raw.clone();
    wrong_count.counts.responded += 1;
    assert!(wrong_count
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("minimal host conservation or settlement failed")));
    let mut false_ledger = raw.clone();
    false_ledger.class_counters.get_mut("c").unwrap().admitted = 99;
    assert!(false_ledger
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("minimal host final governor ledger differs")));
    let mut coherent_false_ledger = raw.clone();
    let counters = coherent_false_ledger.class_counters.get_mut("c").unwrap();
    counters.admitted = 99;
    counters.started = 99;
    counters.terminated = 99;
    assert_eq!(
        coherent_false_ledger.validate_against(&scenario),
        Err("minimal host final governor ledger differs".into())
    );
    let mut missing_capabilities = raw.clone();
    missing_capabilities.final_capabilities.clear();
    assert!(missing_capabilities
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("capability catalog")));
    let mut wrong_capabilities = raw.clone();
    wrong_capabilities.final_capabilities =
        std::collections::BTreeMap::from([("unregistered".into(), 0)]);
    assert!(wrong_capabilities
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("capability catalog")));
    let mut wrong_id = raw;
    wrong_id.records[0].id = 7;
    assert!(wrong_id
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("minimal row 0: offer identity differs")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn minimal_recorder_rejects_independently_wrong_identity_and_coherent_empty_population() {
    let scenario = scenario();
    assert_eq!(scenario.id, "minimal-recorder-contract");
    assert_eq!(scenario.offers.len(), 3);
    let (raw, _) = run_minimal_host_scenario(&scenario, None).await.unwrap();
    raw.validate_against(&scenario).unwrap();

    let mut wrong_scenario = raw.clone();
    wrong_scenario.scenario_id = "different-scenario".into();

    let mut missing_population = raw;
    missing_population.records.clear();
    missing_population.counts = Default::default();
    missing_population.completed_callers = 0;
    missing_population.max_producer_lag_ns = None;
    for counters in missing_population.class_counters.values_mut() {
        *counters = Default::default();
    }
    assert_eq!(
        (
            wrong_scenario.validate_against(&scenario),
            missing_population.validate_against(&scenario),
        ),
        (
            Err("minimal host scenario or mode differs".into()),
            Err("minimal host scenario or mode differs".into()),
        ),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn minimal_producer_failure_preserves_bounded_invalid_rows() {
    let scenario = scenario();
    let (raw, _) =
        run_minimal_host_scenario(&scenario, Some(HostHarnessFault::ProducerBeforeOffer(1)))
            .await
            .unwrap();
    assert!(matches!(raw.status, HostRunStatus::Invalid { .. }));
    assert_eq!(raw.records.len(), scenario.offers.len());
    assert_eq!(raw.records[1].disposition, MinimalDisposition::Pending);
    assert!(raw
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("minimal host run is invalid")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn minimal_recorder_rejects_snapshot_sampling_before_timing() {
    let mut scenario = scenario();
    scenario.load.snapshot_ms = 10;
    assert!(run_minimal_host_scenario(&scenario, None).await.is_err_and(
        |error| error.contains("minimal recorder control requires Snapshot sampling off")
    ));
}

fn fixed_minimal_raw(scenario: &HostScenario) -> MinimalHostRun {
    use std::collections::BTreeMap;
    use taskmesh_bench::host_load::{ClassCounters, HostRunStatus};
    use taskmesh_bench::minimal_host::{MinimalCounts, MinimalRecord, MINIMAL_HOST_VERSION};
    let capabilities = scenario
        .resolved_topology()
        .unwrap()
        .capability_limits
        .keys()
        .cloned()
        .map(|key| (key, 0))
        .collect();
    MinimalHostRun {
        schema_version: MINIMAL_HOST_VERSION,
        status: HostRunStatus::Complete,
        scenario_id: scenario.id.clone(),
        injection_window_ns: scenario.load.injection_ms * 1_000_000,
        recorder_mode: "minimal".into(),
        response_latency_available: false,
        counts: MinimalCounts {
            intended: 3,
            submitted: 1,
            not_submitted: 2,
            responded: 1,
            caller_dropped: 0,
            unanswered_at_settlement: 0,
        },
        completed_callers: 1,
        outstanding_final: 0,
        max_producer_lag_ns: Some(0),
        records: scenario
            .offers
            .iter()
            .enumerate()
            .map(|(id, offer)| MinimalRecord {
                id,
                intended_ns: offer.send_time_ns,
                observed_ns: Some(offer.send_time_ns),
                submitted_ns: (id == 0).then_some(offer.send_time_ns),
                disposition: if id == 0 {
                    MinimalDisposition::Success
                } else {
                    MinimalDisposition::NotSubmitted
                },
            })
            .collect(),
        class_counters: BTreeMap::from([(
            "c".into(),
            ClassCounters {
                admitted: 1,
                started: 1,
                terminated: 1,
                inflight: 0,
                queued: 0,
            },
        )]),
        final_capabilities: capabilities,
        drain_ok: true,
        conservation_ok: true,
    }
}

#[test]
fn minimal_validator_independently_rejects_ledger_and_settlement_fields() {
    let scenario = scenario();
    let good = fixed_minimal_raw(&scenario);
    good.validate_against(&scenario).unwrap();
    let expect = |case: &str, raw: MinimalHostRun, error: &str| {
        assert_eq!(
            raw.validate_against(&scenario).unwrap_err(),
            error,
            "{case}"
        );
    };
    let header = "minimal host scenario or mode differs";
    let mut wrong_schema = good.clone();
    wrong_schema.schema_version += 1;
    expect("schema", wrong_schema, header);
    let mut wrong_window = good.clone();
    wrong_window.injection_window_ns += 1;
    expect("window", wrong_window, header);
    let mut wrong_mode = good.clone();
    wrong_mode.recorder_mode = "other".into();
    expect("mode", wrong_mode, header);
    let mut wrong_snapshot_scenario = scenario.clone();
    wrong_snapshot_scenario.load.snapshot_ms = 10;
    assert_eq!(
        good.validate_against(&wrong_snapshot_scenario).unwrap_err(),
        header
    );
    let settlement = "minimal host conservation or settlement failed";
    let mut wrong_completed = good.clone();
    wrong_completed.completed_callers += 1;
    expect("completed", wrong_completed, settlement);
    let mut unanswered = good.clone();
    unanswered.records[0].disposition = MinimalDisposition::UnansweredAtSettlement;
    unanswered.counts.responded = 0;
    unanswered.counts.unanswered_at_settlement = 1;
    unanswered.completed_callers = 0;
    expect("unanswered", unanswered, settlement);
    let mut outstanding = good.clone();
    outstanding.outstanding_final = 1;
    expect("outstanding", outstanding, settlement);
    let mut wrong_lag = good.clone();
    wrong_lag.max_producer_lag_ns = Some(1);
    expect("lag", wrong_lag, settlement);
    let mut undrained = good.clone();
    undrained.drain_ok = false;
    expect("drain", undrained, settlement);
    let mut unconserved = good.clone();
    unconserved.conservation_ok = false;
    expect("conservation", unconserved, settlement);
    let ledger = "minimal host final governor ledger differs";
    let mut started_below_success = good.clone();
    started_below_success
        .class_counters
        .get_mut("c")
        .unwrap()
        .started = 0;
    expect("started lower bound", started_below_success, ledger);
    let mut started_above_admitted = good.clone();
    started_above_admitted
        .class_counters
        .get_mut("c")
        .unwrap()
        .started = 2;
    expect("started upper bound", started_above_admitted, ledger);
    let mut terminated_above_admitted = good.clone();
    terminated_above_admitted
        .class_counters
        .get_mut("c")
        .unwrap()
        .terminated = 2;
    expect("termination", terminated_above_admitted, ledger);
    let mut inflight = good.clone();
    inflight.class_counters.get_mut("c").unwrap().inflight = 1;
    expect("inflight", inflight, ledger);
    let mut queued = good.clone();
    queued.class_counters.get_mut("c").unwrap().queued = 1;
    expect("queued", queued, ledger);
    let mut extra_class = good.clone();
    extra_class
        .class_counters
        .insert("ghost".into(), Default::default());
    expect("extra class", extra_class, ledger);
    let mut replaced_class = good.clone();
    let counters = replaced_class.class_counters.remove("c").unwrap();
    replaced_class
        .class_counters
        .insert("ghost".into(), counters);
    expect("missing class", replaced_class, ledger);
    let mut held_capacity = good;
    *held_capacity
        .final_capabilities
        .values_mut()
        .next()
        .unwrap() = 1;
    expect("held capacity", held_capacity, ledger);
}
