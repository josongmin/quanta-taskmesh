"""Receipt decisions use synthetic rows; they do not qualify wall-clock noise."""

from __future__ import annotations

import copy
import json
import subprocess
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_perf  # noqa: E402


def fixture() -> tuple[dict, dict, dict, dict]:
    scenario = {
        "schema_version": 1,
        "id": "synthetic",
        "load": {"warmup_ms": 0, "injection_ms": 10, "interval_ms": 1,
                 "snapshot_ms": 0, "settlement_ms": 10,
                 "max_outstanding": 2, "max_records": 2},
        "topology": {"cpu_workers": 1, "blocking_threads": 1,
                     "shared_blocking_limit": 1, "cpu_units": 2, "memory_units": 2},
        "classes": [{"name": "c", "slo_ms": 5, "max_inflight": 2,
                     "max_queue_depth": 2, "cpu_units": 1, "memory_units": 1,
                     "overflow": "queue_within_depth"}],
        "offers": [
            {"send_time_ns": 0, "class": "c", "path": "io", "body": {"kind": "noop"}},
            {"send_time_ns": 1_000_000, "class": "c", "path": "io",
             "body": {"kind": "noop"}},
        ],
    }

    def row(index: int) -> dict:
        intended = index * 1_000_000
        return {
            "id": index,
            "class": "c",
            "path": "io",
            "intended_ns": intended,
            "scheduled_lag_ns": 0,
            "submitted_ns": intended,
            "body_started_ns": intended + 1,
            "body_finished_ns": intended + 2,
            "caller_response_ns": intended + 3,
            "caller_drop_ns": None,
            "disposition": {"kind": "responded", "outcome": {"kind": "success"}},
        }

    counts = {
        "intended": 2,
        "submitted": 2,
        "not_submitted": 0,
        "responded": 2,
        "caller_dropped": 0,
        "waiting": 0,
        "unanswered_at_settlement": 0,
    }
    raw = {
        "schema_version": 1,
        "scenario_id": "synthetic",
        "injection_window_ns": 10_000_000,
        "interval_ns": 1_000_000,
        "snapshot_cadence_ns": 0,
        "snapshots": [],
        "cut": counts.copy(),
        "settlement": counts.copy(),
        "records": [row(0), row(1)],
        "class_counters": {
            "c": {"admitted": 2, "started": 2, "terminated": 2, "inflight": 0, "queued": 0}
        },
        "final_capabilities": {"cpu": 0},
        "drain_ok": True,
        "conservation_ok": True,
    }
    identity = {
        "source_head": "a" * 40,
        "source_tree": "b" * 40,
        "source_dirty": False,
        "lock_sha256": "c" * 64,
        "rustc": "rustc synthetic",
        "features": [],
        "topology_fingerprint": "d" * 64,
        "host_fingerprint": "e" * 64,
    }
    calibration = {
        "generator_headroom_ok": True,
        "observer_distorted": False,
        "producer_lag_limit_ns": 10,
    }
    return raw, scenario, identity, calibration


def encoded(value: dict) -> bytes:
    return json.dumps(value, sort_keys=True).encode()


def complete() -> tuple[bytes, bytes, bytes, dict, dict]:
    raw, scenario, identity, calibration = fixture()
    raw_bytes, scenario_bytes = encoded(raw), encoded(scenario)
    summary = host_perf.make_summary(raw_bytes, scenario_bytes, identity, calibration, 2)
    return raw_bytes, scenario_bytes, encoded(summary), identity, calibration


def test_complete_raw_summary_pair_is_structurally_admissible() -> None:
    raw, scenario, summary, identity, _ = complete()
    accepted = host_perf.verify_receipt(raw, scenario, summary, identity)
    assert accepted["metrics"]["success_latency_count"] == 2
    assert accepted["metrics"]["counts"]["intended"] == 2
    assert accepted["metrics"]["success_latency_p99_ns"] == 3
    assert accepted["metrics"]["cohort_goodput"]["c/io"]["slo_success"] == 2
    assert accepted["metrics"]["cohort_goodput"]["c/io"]["slo_fraction"] == 1
    assert accepted["metrics"]["intervals"][0]["responded"] == 1
    assert accepted["metrics"]["intervals"][1]["responded"] == 1
    assert accepted["metrics"]["intervals"][0]["success"] == 1
    assert sum(point["responded"] for point in accepted["metrics"]["intervals"]) == 2


def test_self_asserted_calibration_never_qualifies_performance() -> None:
    raw, scenario, summary, identity, _ = complete()
    assert host_perf.verify_receipt(raw, scenario, summary, identity)
    with pytest.raises(host_perf.ReceiptError, match="measured calibration artifact"):
        host_perf.verify_receipt(raw, scenario, summary, identity, require_performance=True)


def test_cli_create_and_verify_stay_structural(tmp_path: Path) -> None:
    raw, scenario, _, calibration = fixture()
    raw_path = tmp_path / "raw.json"
    scenario_path = tmp_path / "scenario.json"
    calibration_path = tmp_path / "calibration.json"
    summary_path = tmp_path / "summary.json"
    for path, value in (
        (raw_path, raw), (scenario_path, scenario), (calibration_path, calibration)
    ):
        path.write_bytes(encoded(value))
    command = [
        sys.executable, str(Path(host_perf.__file__)), "create", str(raw_path),
        str(scenario_path), str(summary_path), "--calibration", str(calibration_path),
        "--p99-min-samples", "2",
    ]
    created = subprocess.run(command, capture_output=True, text=True, check=False)
    assert created.returncode == 0, created.stderr
    assert "performance=UNQUALIFIED" in created.stdout
    verified = subprocess.run(
        [sys.executable, str(Path(host_perf.__file__)), "verify", str(raw_path),
         str(scenario_path), str(summary_path)],
        capture_output=True, text=True, check=False,
    )
    assert verified.returncode == 0, verified.stderr
    assert "performance=UNQUALIFIED" in verified.stdout
    claimed = subprocess.run(
        [sys.executable, str(Path(host_perf.__file__)), "verify", str(raw_path),
         str(scenario_path), str(summary_path), "--require-performance"],
        capture_output=True, text=True, check=False,
    )
    assert claimed.returncode != 0
    assert "STRUCTURALLY_VALID" not in claimed.stdout


@pytest.mark.parametrize("damage", ["body", "timer", "unknown", "topology"])
def test_rust_scenario_contract_rejects_invalid_receipt_fixture(damage: str) -> None:
    raw, scenario, identity, calibration = fixture()
    if damage == "body":
        scenario["offers"][0]["body"] = {"kind": "cpu_spin", "iterations": 1}
    elif damage == "timer":
        scenario["offers"][0].update(cancel_after_ms=1, drop_after_ms=1)
    elif damage == "unknown":
        scenario["load"]["surprise"] = 1
    else:
        scenario["topology"]["shared_blocking_limit"] = 0
    with pytest.raises(host_perf.ReceiptError, match="Rust scenario preflight"):
        host_perf.make_summary(
            encoded(raw), encoded(scenario), identity, calibration, 2
        )


def test_forged_rejection_variant_is_not_a_typed_verdict() -> None:
    raw, scenario, identity, calibration = fixture()
    row = raw["records"][1]
    row["body_started_ns"] = None
    row["body_finished_ns"] = None
    row["disposition"]["outcome"] = {
        "kind": "rejected", "verdict": "NotAnAdmissionVerdict"
    }
    for field in ("admitted", "started", "terminated"):
        raw["class_counters"]["c"][field] = 1
    with pytest.raises(host_perf.ReceiptError, match="Rust scenario preflight"):
        host_perf.make_summary(
            encoded(raw), encoded(scenario), identity, calibration, 1
        )


def test_post_window_success_stays_in_cohort_but_misses_slo() -> None:
    raw, scenario, identity, calibration = fixture()
    second = raw["records"][1]
    second["body_finished_ns"] = 10_000_001
    second["caller_response_ns"] = 10_100_001
    raw["cut"]["responded"] = 1
    raw["cut"]["waiting"] = 1
    raw_bytes, scenario_bytes = encoded(raw), encoded(scenario)
    summary = host_perf.make_summary(raw_bytes, scenario_bytes, identity, calibration, 2)
    accepted = host_perf.verify_receipt(raw_bytes, scenario_bytes, encoded(summary), identity)
    cohort = accepted["metrics"]["cohort_goodput"]["c/io"]
    assert (cohort["success"], cohort["slo_success"], cohort["intended"]) == (2, 1, 2)
    assert sum(point["responded"] for point in accepted["metrics"]["intervals"]) == 1


@pytest.mark.parametrize("damage", ["missing", "duplicate", "wrong_class", "wrong_time"])
def test_broken_raw_row_population_rejects(damage: str) -> None:
    raw, scenario, summary, identity, _ = complete()
    changed = json.loads(raw)
    if damage == "missing":
        changed["records"].pop()
    elif damage == "duplicate":
        changed["records"][1]["id"] = 0
    elif damage == "wrong_class":
        changed["records"][1]["class"] = "other"
    else:
        changed["records"][1]["body_started_ns"] = 0
    with pytest.raises(host_perf.ReceiptError):
        host_perf.verify_receipt(encoded(changed), scenario, summary, identity)


def test_drained_run_with_residual_pool_slot_rejects() -> None:
    raw, scenario, identity, calibration = fixture()
    raw["final_capabilities"]["cpu"] = 1
    with pytest.raises(host_perf.ReceiptError, match="owns engine capacity"):
        host_perf.make_summary(encoded(raw), encoded(scenario), identity, calibration, 2)


def test_wrong_raw_digest_or_missing_summary_population_rejects() -> None:
    raw, scenario, summary, identity, _ = complete()
    changed = json.loads(summary)
    changed["raw_sha256"] = "0" * 64
    with pytest.raises(host_perf.ReceiptError):
        host_perf.verify_receipt(raw, scenario, encoded(changed), identity)
    changed = json.loads(summary)
    del changed["metrics"]["by_class_path"]
    with pytest.raises(host_perf.ReceiptError):
        host_perf.verify_receipt(raw, scenario, encoded(changed), identity)
    changed = json.loads(summary)
    del changed["metrics"]["by_class_path"]["c/io"]["rejected"]
    with pytest.raises(host_perf.ReceiptError):
        host_perf.verify_receipt(raw, scenario, encoded(changed), identity)


def test_scenario_digest_and_duplicate_json_keys_reject() -> None:
    raw, scenario, summary, identity, _ = complete()
    changed = json.loads(scenario)
    changed["topology"]["cpu_workers"] = 2
    with pytest.raises(host_perf.ReceiptError):
        host_perf.verify_receipt(raw, encoded(changed), summary, identity)
    with pytest.raises(host_perf.ReceiptError, match="duplicate JSON key"):
        host_perf.verify_receipt(raw, scenario, summary[:-1] + b',"metrics":{}}', identity)


@pytest.mark.parametrize(
    "field", ["source_head", "features", "topology_fingerprint", "host_fingerprint"]
)
def test_incompatible_identity_rejects(field: str) -> None:
    raw, scenario, summary, identity, _ = complete()
    other = copy.deepcopy(identity)
    other[field] = ["rayon"] if field == "features" else "different"
    with pytest.raises(host_perf.ReceiptError, match="fingerprint mismatch"):
        host_perf.verify_receipt(raw, scenario, summary, other)


def test_generator_and_observer_invalid_flags_reject() -> None:
    raw, scenario, identity, calibration = fixture()
    raw["records"][1]["scheduled_lag_ns"] = 20
    raw["records"][1]["submitted_ns"] = 1_000_021
    raw["records"][1]["body_started_ns"] = 1_000_022
    raw["records"][1]["body_finished_ns"] = 1_000_023
    raw["records"][1]["caller_response_ns"] = 1_000_024
    raw_bytes, scenario_bytes = encoded(raw), encoded(scenario)
    summary = host_perf.make_summary(raw_bytes, scenario_bytes, identity, calibration, 2)
    with pytest.raises(host_perf.ReceiptError, match="producer lag"):
        host_perf.verify_receipt(
            raw_bytes, scenario_bytes, encoded(summary), identity, require_performance=True
        )
    calibration["observer_distorted"] = True
    summary = host_perf.make_summary(raw_bytes, scenario_bytes, identity, calibration, 2)
    with pytest.raises(host_perf.ReceiptError, match="calibration failed"):
        host_perf.verify_receipt(
            raw_bytes, scenario_bytes, encoded(summary), identity, require_performance=True
        )


def test_producer_cap_not_submitted_rejects_performance() -> None:
    raw, scenario, identity, calibration = fixture()
    second = raw["records"][1]
    for key in ("submitted_ns", "body_started_ns", "body_finished_ns", "caller_response_ns"):
        second[key] = None
    second["disposition"] = {"kind": "not_submitted"}
    for counts in (raw["cut"], raw["settlement"]):
        counts["submitted"] = 1
        counts["not_submitted"] = 1
        counts["responded"] = 1
    for key in ("admitted", "started", "terminated"):
        raw["class_counters"]["c"][key] = 1
    raw_bytes, scenario_bytes = encoded(raw), encoded(scenario)
    summary = host_perf.make_summary(raw_bytes, scenario_bytes, identity, calibration, 1)
    with pytest.raises(host_perf.ReceiptError, match="generator-limited"):
        host_perf.verify_receipt(
            raw_bytes, scenario_bytes, encoded(summary), identity, require_performance=True
        )


def test_precision_floor_and_dirty_source_reject() -> None:
    raw, scenario, identity, calibration = fixture()
    raw_bytes, scenario_bytes = encoded(raw), encoded(scenario)
    thin = host_perf.make_summary(raw_bytes, scenario_bytes, identity, calibration, 3)
    assert thin["metrics"]["success_latency_p99_ns"] is None
    assert thin["metrics"]["success_latency_p99_status"] == "insufficient_population"
    assert thin["metrics"]["success_latency_by_class_path"]["c/io"]["p99_ns"] is None
    with pytest.raises(host_perf.ReceiptError, match="p99 population"):
        host_perf.verify_receipt(
            raw_bytes, scenario_bytes, encoded(thin), identity, require_performance=True
        )
    identity["source_dirty"] = True
    dirty = host_perf.make_summary(raw_bytes, scenario_bytes, identity, calibration, 2)
    with pytest.raises(host_perf.ReceiptError, match="dirty source"):
        host_perf.verify_receipt(
            raw_bytes, scenario_bytes, encoded(dirty), identity, require_performance=True
        )


def test_global_tail_count_cannot_hide_thin_class_population() -> None:
    raw, scenario, identity, calibration = fixture()
    scenario["classes"].append({**scenario["classes"][0], "name": "d"})
    scenario["offers"][1]["class"] = "d"
    raw["records"][1]["class"] = "d"
    raw["class_counters"] = {
        name: {"admitted": 1, "started": 1, "terminated": 1, "inflight": 0, "queued": 0}
        for name in ("c", "d")
    }
    raw_bytes, scenario_bytes = encoded(raw), encoded(scenario)
    summary = host_perf.make_summary(raw_bytes, scenario_bytes, identity, calibration, 2)
    assert summary["metrics"]["success_latency_count"] == 2
    assert summary["metrics"]["success_latency_by_class_path"]["c/io"]["p99_ns"] is None
    with pytest.raises(host_perf.ReceiptError, match="per-class/path p99"):
        host_perf.verify_receipt(
            raw_bytes, scenario_bytes, encoded(summary), identity, require_performance=True
        )


def test_sparse_snapshot_peaks_are_labeled_lower_bounds() -> None:
    raw, scenario, identity, calibration = fixture()
    scenario["load"]["snapshot_ms"] = 5
    raw["snapshot_cadence_ns"] = 5_000_000

    def sample(intended: int, queued: int) -> dict:
        return {
            "intended_ns": intended,
            "observed_ns": intended,
            "classes": {
                "c": {
                    "inflight": 1,
                    "queued": queued,
                    "accepted": 1,
                    "running": 0,
                    "cpu_units_held": "1",
                    "memory_units_held": "1",
                }
            },
            "capabilities": {"cpu": 1},
            "conservation_ok": True,
        }

    raw["snapshots"] = [sample(0, 2), sample(5_000_000, 0)]
    raw_bytes, scenario_bytes = encoded(raw), encoded(scenario)
    summary = host_perf.make_summary(raw_bytes, scenario_bytes, identity, calibration, 2)
    metrics = host_perf.verify_receipt(raw_bytes, scenario_bytes, encoded(summary), identity)[
        "metrics"
    ]
    assert metrics["sampled_peaks_lower_bound"]["c"]["queued"] == 2
    assert metrics["sampled_capability_peaks_lower_bound"]["cpu"] == 1
    assert metrics["snapshot_sample_count"] == 2
    raw["snapshots"][0]["classes"]["c"]["cpu_units_held"] = "01"
    with pytest.raises(host_perf.ReceiptError, match="canonical decimal"):
        host_perf.make_summary(encoded(raw), scenario_bytes, identity, calibration, 2)
