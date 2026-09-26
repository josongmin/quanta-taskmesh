use taskmesh_bench::closed_loop::{run_closed_loop_with_topology, ClosedLoopScenario};
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

#[test]
fn closed_loop_preflight_rejects_zero_or_unbounded_callers() {
    let mut scenario = ClosedLoopScenario::from_json(FIXTURE).expect("fixture");
    scenario.concurrency = 0;
    assert!(scenario
        .validate()
        .is_err_and(|error| error.contains("out of bounds")));
    scenario.concurrency = 2;
    scenario.iterations_per_slot = 1_000_000;
    assert!(scenario
        .validate()
        .is_err_and(|error| error.contains("out of bounds")));
    scenario.iterations_per_slot = 3;
    scenario.max_run_ms = 0;
    assert!(scenario
        .validate()
        .is_err_and(|error| error.contains("out of bounds")));
}
