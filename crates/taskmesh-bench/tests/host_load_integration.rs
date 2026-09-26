//! The bench adapter, rather than the engine's already-covered verdict matrix.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use taskmesh::{
    Builder, CancellationPolicy, ClassPolicy, OverflowPolicy, RunError, Runtime, SubmitOptions,
    TaskClass, TaskSpec,
};
use taskmesh_bench::host_load::{
    classify_response, run_host_scenario, run_host_scenario_with_test_gate, CallerCounts,
    CallerDisposition, HostHarnessTestGate, ResponseOutcome,
};
use taskmesh_bench::host_scenarios::HostScenario;
use tokio_util::sync::CancellationToken;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn requested_stack_blocking_and_async_paths_reconcile_public_host_rows() {
    let scenario = HostScenario::from_json(include_bytes!(
        "../../../tools/bench/scenarios/h4-requested-stack-smoke.json"
    ))
    .expect("requested-stack fixture");
    let raw = run_host_scenario(&scenario)
        .await
        .expect("requested-stack host run");
    raw.validate_against(&scenario).expect("typed raw parity");
    assert_eq!(raw.records.len(), 2);
    assert_eq!(raw.settlement.responded, 2);
    assert_eq!(raw.settlement.not_submitted, 0);
    for row in &raw.records {
        assert!(matches!(
            row.disposition,
            CallerDisposition::Responded {
                outcome: ResponseOutcome::Success
            }
        ));
        assert!(row.body_started_ns.is_some());
        assert!(row.body_finished_ns.is_some());
    }
    assert_eq!(raw.final_capabilities["large_stack"], 0);
    assert!(raw.drain_ok && raw.conservation_ok);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn finite_two_class_mixed_path_run_keeps_every_intended_offer() {
    let fixture = json!({
        "schema_version": 1,
        "id": "two-class-adapter",
        "load": {"warmup_ms": 1, "injection_ms": 40, "interval_ms": 10,
                 "snapshot_ms": 10,
                 "settlement_ms": 500,
                 "max_outstanding": 16, "max_records": 16},
        "topology": {"cpu_workers": 2, "blocking_threads": 2,
                     "shared_blocking_limit": 2, "cpu_units": 8, "memory_units": 8},
        "classes": [
            {"name": "interactive", "slo_ms": 100, "max_inflight": 2, "max_queue_depth": 2,
             "cpu_units": 1, "memory_units": 1, "overflow": "queue_within_depth"},
            {"name": "batch", "slo_ms": 100, "max_inflight": 2, "max_queue_depth": 2,
             "cpu_units": 1, "memory_units": 1, "overflow": "queue_within_depth"}
        ],
        "offers": [
            {"send_time_ns": 0, "class": "interactive", "path": "blocking",
             "body": {"kind": "blocking_sleep", "millis": 30}, "deadline_ms": 1},
            {"send_time_ns": 1_000_000, "class": "batch", "path": "io",
             "body": {"kind": "async_sleep", "millis": 2}},
            {"send_time_ns": 2_000_000, "class": "batch", "path": "cpu",
             "body": {"kind": "cpu_spin", "iterations": 1000}}
        ]
    });
    let scenario = HostScenario::from_json(&serde_json::to_vec(&fixture).unwrap()).unwrap();
    let raw = run_host_scenario(&scenario)
        .await
        .expect("bounded run settles");
    raw.validate().unwrap();
    assert_eq!(raw.records.len(), scenario.offers.len());
    assert!(raw.drain_ok);
    assert!(raw.conservation_ok);
    assert!(raw.final_capabilities.values().all(|in_use| *in_use == 0));
    assert_eq!(raw.snapshots.len(), 4);
    assert!(raw.snapshots.iter().all(|sample| sample.conservation_ok));
    assert_eq!(raw.settlement.not_submitted, 0);
    assert_eq!(raw.settlement.unanswered_at_settlement, 0);
    assert_eq!(raw.settlement.responded, 3);
    assert_eq!(
        CallerCounts::at_cut(&raw.records, raw.injection_window_ns).unwrap(),
        raw.cut
    );
    let by_class = raw.records.iter().fold(BTreeMap::new(), |mut counts, row| {
        *counts
            .entry((row.class.as_str(), row.path))
            .or_insert(0_usize) += 1;
        counts
    });
    assert_eq!(
        by_class.get(&("interactive", scenario.offers[0].path)),
        Some(&1)
    );
    assert_eq!(by_class.get(&("batch", scenario.offers[1].path)), Some(&1));
    assert_eq!(by_class.get(&("batch", scenario.offers[2].path)), Some(&1));
    for (id, row) in raw.records.iter().enumerate() {
        assert_eq!(row.id, id);
        assert_eq!(row.class, scenario.offers[id].class);
        assert_eq!(row.path, scenario.offers[id].path);
        assert!(matches!(
            row.disposition,
            CallerDisposition::Responded { .. }
        ));
    }
    assert_eq!(raw.class_counters["batch"].admitted, 2);
    assert_eq!(raw.class_counters["batch"].terminated, 2);
    let encoded = serde_json::to_vec(&raw).unwrap();
    let decoded: taskmesh_bench::host_load::RawHostRun = serde_json::from_slice(&encoded).unwrap();
    decoded.validate_against(&scenario).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mixed_caller_events_keep_blocking_worker_custody_in_raw_rows() {
    let fixture = json!({
        "schema_version": 1,
        "id": "mixed-caller-attribution",
        "load": {"warmup_ms": 0, "injection_ms": 100, "interval_ms": 10,
                 "snapshot_ms": 0, "settlement_ms": 1000,
                 "max_outstanding": 8, "max_records": 8},
        "topology": {"cpu_workers": 2, "blocking_threads": 2,
                     "shared_blocking_limit": 2, "cpu_units": 8, "memory_units": 8},
        "classes": [
            {"name": "held", "slo_ms": 900, "max_inflight": 2,
             "max_queue_depth": 1, "cpu_units": 1, "memory_units": 1,
             "overflow": "queue_within_depth"},
            {"name": "other", "slo_ms": 900, "max_inflight": 2,
             "max_queue_depth": 1, "cpu_units": 1, "memory_units": 1,
             "overflow": "queue_within_depth"}
        ],
        "offers": [
            {"send_time_ns": 0, "class": "held", "path": "blocking",
             "body": {"kind": "blocking_sleep", "millis": 1}, "drop_after_ms": 900},
            {"send_time_ns": 1_000_000, "class": "other", "path": "io",
             "body": {"kind": "async_sleep", "millis": 1}, "deadline_ms": 0},
            {"send_time_ns": 2_000_000, "class": "other", "path": "io",
             "body": {"kind": "async_sleep", "millis": 1}, "cancel_after_ms": 0}
        ]
    });
    let scenario = HostScenario::from_json(&serde_json::to_vec(&fixture).unwrap()).unwrap();
    let gate = Arc::new(HostHarnessTestGate::new(0));
    let run_gate = Arc::clone(&gate);
    let run = tokio::spawn(async move {
        run_host_scenario_with_test_gate(&scenario, run_gate)
            .await
            .map(|raw| (raw, scenario))
    });
    tokio::time::timeout(Duration::from_secs(5), gate.wait_started())
        .await
        .expect("blocking body starts");
    gate.trigger_drop();
    tokio::time::timeout(Duration::from_secs(5), gate.wait_caller_dropped())
        .await
        .expect("caller drop is recorded");
    gate.release_worker();
    let (raw, scenario) = tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .expect("bounded scenario settles")
        .expect("harness task")
        .expect("harness run");
    raw.validate_against(&scenario).unwrap();
    assert_eq!(raw.records.len(), 3);
    assert_eq!(raw.settlement.intended, 3);
    assert_eq!(raw.settlement.submitted, 3);
    assert_eq!(raw.settlement.caller_dropped, 1);
    assert_eq!(raw.settlement.responded, 2);
    assert_eq!(raw.settlement.unanswered_at_settlement, 0);
    let held = &raw.records[0];
    assert_eq!(held.disposition, CallerDisposition::CallerDropped);
    assert!(held.caller_response_ns.is_none());
    assert!(held.body_started_ns.unwrap() <= held.caller_drop_ns.unwrap());
    assert!(held.body_finished_ns.unwrap() > held.caller_drop_ns.unwrap());
    for row in &raw.records[1..] {
        assert!(matches!(
            row.disposition,
            CallerDisposition::Responded { .. }
        ));
        assert!(row.caller_response_ns.is_some());
    }
    assert!(raw.final_capabilities.values().all(|in_use| *in_use == 0));
    assert!(raw.drain_ok && raw.conservation_ok);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn held_worker_rejects_follower_and_remains_owned_after_caller_response() {
    let class = TaskClass::new("c");
    let runtime = Builder::new()
        .class_policy(
            class.clone(),
            ClassPolicy::new()
                .max_inflight(1)
                .overflow_policy(OverflowPolicy::Reject)
                .cancellation_policy(CancellationPolicy::CooperativeWithDeadline),
        )
        .build()
        .unwrap();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let cancel = CancellationToken::new();
    let holder_cancel = cancel.clone();
    let holder_rt = runtime.clone();
    let holder = tokio::spawn(async move {
        holder_rt
            .run_blocking_with(
                TaskSpec::blocking(TaskClass::new("c")).operation("held"),
                SubmitOptions::unbounded().with_cancel(holder_cancel),
                move || {
                    started_tx.send(()).ok();
                    release_rx.recv().unwrap_or(());
                    Ok::<(), ()>(())
                },
            )
            .await
    });
    if tokio::time::timeout(Duration::from_secs(5), started_rx)
        .await
        .expect("holder starts")
        .is_err()
    {
        panic!("holder did not start: {:?}", holder.await);
    }
    let rejected: Result<(), RunError<()>> = runtime
        .run_io(
            TaskSpec::io(class.clone()).operation("rejected-follower"),
            async { Ok::<(), ()>(()) },
        )
        .await;
    assert!(matches!(
        classify_response(rejected),
        ResponseOutcome::Rejected { .. }
    ));
    cancel.cancel();
    assert!(matches!(
        classify_response(holder.await.unwrap()),
        ResponseOutcome::Cancelled
    ));
    assert_eq!(runtime.snapshot().classes[&class].inflight, 1);
    release_tx.send(()).unwrap();
    runtime.drain(Duration::from_secs(5)).await.unwrap();
    let after = runtime.snapshot();
    assert_eq!(after.classes[&class].inflight, 0);
    assert_eq!(after.conservation_violation(), None);
}
