//! Generator-control artifacts are a bench measurement contract, not a second
//! Governor admission oracle.

use serde_json::json;
use taskmesh_bench::generator_calibration::{
    run_generator_control, run_generator_control_with_topology, GeneratorDisposition,
};
use taskmesh_bench::host_load::{HostHarnessFault, HostRunStatus};
use taskmesh_bench::host_scenarios::HostScenario;

fn scenario() -> HostScenario {
    let fixture = json!({
        "schema_version": 1,
        "id": "generator-control-contract",
        "load": {"warmup_ms": 5, "injection_ms": 100, "interval_ms": 10,
                 "snapshot_ms": 0, "settlement_ms": 500,
                 "max_outstanding": 8, "max_records": 8},
        "topology": {"cpu_workers": 2, "blocking_threads": 2,
                     "shared_blocking_limit": 2, "cpu_units": 8, "memory_units": 8},
        "classes": [
            {"name": "interactive", "slo_ms": 100, "max_inflight": 2,
             "max_queue_depth": 2, "cpu_units": 1, "memory_units": 1,
             "overflow": "queue_within_depth"},
            {"name": "batch", "slo_ms": 100, "max_inflight": 2,
             "max_queue_depth": 2, "cpu_units": 1, "memory_units": 1,
             "overflow": "queue_within_depth"}
        ],
        "offers": [
            {"send_time_ns": 0, "class": "interactive", "path": "io",
             "body": {"kind": "noop"}},
            {"send_time_ns": 10_000_000, "class": "batch", "path": "blocking",
             "body": {"kind": "noop"}},
            {"send_time_ns": 20_000_000, "class": "batch", "path": "cpu",
             "body": {"kind": "noop"}}
        ]
    });
    HostScenario::from_json(&serde_json::to_vec(&fixture).unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn control_keeps_every_intended_offer_and_settles_without_taskmesh_work() {
    let scenario = scenario();
    let raw = run_generator_control(&scenario).await.unwrap();
    raw.validate_against(&scenario).unwrap();
    assert_eq!(raw.records.len(), scenario.offers.len());
    assert_eq!(raw.submitted + raw.not_submitted, raw.records.len());
    assert_eq!(raw.completed, raw.submitted);
    assert_eq!(raw.outstanding_final, 0);
    assert!(raw.drain_ok && raw.conservation_ok);
    for (id, row) in raw.records.iter().enumerate() {
        assert_eq!(row.id, id);
        assert_eq!(row.intended_ns, scenario.offers[id].send_time_ns);
        assert_ne!(row.disposition, GeneratorDisposition::Pending);
    }
    let round_trip = serde_json::from_slice(&serde_json::to_vec(&raw).unwrap()).unwrap();
    let decoded: taskmesh_bench::generator_calibration::GeneratorRun = round_trip;
    decoded.validate_against(&scenario).unwrap();

    let mut wrong_id = raw.clone();
    wrong_id.records[0].id = 3;
    assert!(wrong_id.validate_against(&scenario).is_err());
    let mut wrong_lag = raw.clone();
    wrong_lag.records[0].scheduled_lag_ns = Some(u64::MAX);
    assert!(wrong_lag.validate_against(&scenario).is_err());
    let mut wrong_count = raw;
    wrong_count.completed += 1;
    assert!(wrong_count.validate_against(&scenario).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn producer_failure_preserves_bounded_invalid_rows() {
    let scenario = scenario();
    let (raw, topology) = run_generator_control_with_topology(
        &scenario,
        Some(HostHarnessFault::ProducerBeforeOffer(1)),
    )
    .await
    .unwrap();
    assert!(matches!(raw.status, HostRunStatus::Invalid { .. }));
    assert_eq!(raw.records.len(), scenario.offers.len());
    assert_eq!(raw.records[1].disposition, GeneratorDisposition::Pending);
    assert!(raw.validate_against(&scenario).is_err());
    assert_eq!(topology.schema_version, 1);
}
