//! Generator-control artifacts are a bench measurement contract, not a second
//! Governor admission oracle.

use serde_json::json;
use taskmesh_bench::generator_calibration::{
    run_generator_control, run_generator_control_with_topology, GeneratorDisposition,
    GeneratorRecord, GeneratorRun, GENERATOR_RAW_VERSION,
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

fn valid_generator_raw(scenario: &HostScenario) -> GeneratorRun {
    let records = scenario
        .offers
        .iter()
        .enumerate()
        .map(|(id, offer)| GeneratorRecord {
            id,
            intended_ns: offer.send_time_ns,
            observed_ns: Some(offer.send_time_ns),
            scheduled_lag_ns: Some(0),
            submitted_ns: Some(offer.send_time_ns),
            disposition: GeneratorDisposition::Submitted,
        })
        .collect::<Vec<_>>();
    GeneratorRun {
        schema_version: GENERATOR_RAW_VERSION,
        status: HostRunStatus::Complete,
        scenario_id: scenario.id.clone(),
        injection_window_ns: scenario.load.injection_ms * 1_000_000,
        max_outstanding: scenario.load.max_outstanding,
        submitted: records.len(),
        not_submitted: 0,
        completed: records.len(),
        outstanding_final: 0,
        drain_ok: true,
        conservation_ok: true,
        records,
    }
}

#[test]
fn generator_raw_validation_rejects_each_independent_corruption() {
    let scenario = scenario();
    let raw = valid_generator_raw(&scenario);
    raw.validate_against(&scenario).unwrap();
    let decoded: GeneratorRun = serde_json::from_slice(&serde_json::to_vec(&raw).unwrap()).unwrap();
    decoded.validate_against(&scenario).unwrap();
    let mut not_submitted = raw.clone();
    not_submitted.records[1].disposition = GeneratorDisposition::NotSubmitted;
    not_submitted.records[1].submitted_ns = None;
    not_submitted.submitted -= 1;
    not_submitted.not_submitted += 1;
    not_submitted.completed -= 1;
    not_submitted.validate_against(&scenario).unwrap();
    let settlement = "generator conservation or settlement failed";
    let mut wrong_submitted = not_submitted.clone();
    wrong_submitted.submitted += 1;
    assert_eq!(
        wrong_submitted.validate_against(&scenario).unwrap_err(),
        settlement
    );
    let mut wrong_submitted_and_completed = not_submitted.clone();
    wrong_submitted_and_completed.submitted += 1;
    wrong_submitted_and_completed.completed = wrong_submitted_and_completed.submitted;
    assert_eq!(
        wrong_submitted_and_completed
            .validate_against(&scenario)
            .unwrap_err(),
        settlement
    );
    let mut wrong_not_submitted = not_submitted.clone();
    wrong_not_submitted.not_submitted += 1;
    assert_eq!(
        wrong_not_submitted.validate_against(&scenario).unwrap_err(),
        settlement
    );

    let reject = |change: fn(&mut GeneratorRun), expected: &str| {
        let mut invalid = raw.clone();
        change(&mut invalid);
        let error = match invalid.validate_against(&scenario) {
            Ok(()) => panic!("invalid generator raw was accepted: {invalid:?}"),
            Err(error) => error,
        };
        assert_eq!(error, expected);
    };
    let header = "generator scenario or record population differs";
    reject(|run| run.schema_version += 1, header);
    reject(
        |run| run.scenario_id = "another-valid-scenario".into(),
        header,
    );
    reject(|run| run.injection_window_ns += 1, header);
    reject(|run| run.max_outstanding += 1, header);
    reject(
        |run| {
            run.records.push(run.records[0].clone());
            run.submitted += 1;
            run.completed += 1;
        },
        header,
    );

    let schedule = "generator row 1 differs from intended schedule";
    reject(|run| run.records[1].id = 0, schedule);
    reject(
        |run| {
            run.records[1].intended_ns += 1;
            run.records[1].observed_ns = Some(run.records[1].intended_ns);
            run.records[1].submitted_ns = Some(run.records[1].intended_ns);
        },
        schedule,
    );
    reject(
        |run| run.records[1].observed_ns = None,
        "generator row 1 was not paced",
    );
    reject(|run| run.records[1].observed_ns = Some(0), schedule);
    reject(|run| run.records[1].scheduled_lag_ns = Some(1), schedule);
    reject(
        |run| {
            run.records[1].observed_ns = Some(run.injection_window_ns);
            run.records[1].scheduled_lag_ns =
                Some(run.injection_window_ns - run.records[1].intended_ns);
        },
        "generator row 1 submitted after cut",
    );
    reject(
        |run| run.records[1].submitted_ns = None,
        "generator row 1 submitted after cut",
    );
    reject(
        |run| run.records[1].submitted_ns = Some(0),
        "generator row 1 submitted after cut",
    );
    reject(
        |run| run.records[1].submitted_ns = Some(run.injection_window_ns),
        "generator row 1 submitted after cut",
    );
    reject(
        |run| run.records[1].disposition = GeneratorDisposition::NotSubmitted,
        "generator row 1 has impossible submit",
    );
    reject(
        |run| run.records[1].disposition = GeneratorDisposition::Pending,
        "generator row 1 is still pending",
    );

    reject(|run| run.completed -= 1, settlement);
    reject(|run| run.outstanding_final = 1, settlement);
    reject(|run| run.drain_ok = false, settlement);
    reject(|run| run.conservation_ok = false, settlement);
}

#[test]
fn generator_raw_rejects_coherent_shorter_record_population() {
    let scenario = scenario();
    let mut raw = valid_generator_raw(&scenario);
    raw.records.pop();
    raw.submitted = raw.records.len();
    raw.completed = raw.submitted;
    assert_eq!(raw.not_submitted, 0);
    assert_eq!(
        raw.validate_against(&scenario).unwrap_err(),
        "generator scenario or record population differs"
    );
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
    assert!(wrong_id
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("generator row 0 differs from intended schedule")));
    let mut wrong_lag = raw.clone();
    wrong_lag.records[0].scheduled_lag_ns = Some(u64::MAX);
    assert!(wrong_lag
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("generator row 0 differs from intended schedule")));
    let mut wrong_count = raw;
    wrong_count.completed += 1;
    assert!(wrong_count
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("generator conservation or settlement failed")));
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
    assert!(raw
        .validate_against(&scenario)
        .is_err_and(|error| error.contains("generator run is invalid")));
    assert_eq!(topology.schema_version, 1);
}
