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
    let (raw, topology) = run_minimal_host_scenario(&scenario, None).await.unwrap();
    raw.validate_against(&scenario).unwrap();
    assert_eq!(raw.records.len(), scenario.offers.len());
    assert_eq!(
        raw.counts.intended,
        raw.counts.submitted + raw.counts.not_submitted
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
