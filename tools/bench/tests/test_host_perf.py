"""Receipt decisions use synthetic rows; they do not qualify wall-clock noise."""

from __future__ import annotations

import copy
import json
import subprocess
import sys
from functools import lru_cache
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_perf  # noqa: E402

RUNNER_BYTES = b"synthetic retained runner bytes"


def fixture() -> tuple[dict, dict, dict, dict]:
    scenario = {
        "schema_version": 1,
        "id": "synthetic",
        "load": {
            "warmup_ms": 0,
            "injection_ms": 10,
            "interval_ms": 1,
            "snapshot_ms": 0,
            "settlement_ms": 10,
            "max_outstanding": 2,
            "max_records": 2,
        },
        "topology": {
            "cpu_workers": 1,
            "blocking_threads": 1,
            "shared_blocking_limit": 1,
            "cpu_units": 2,
            "memory_units": 2,
        },
        "classes": [
            {
                "name": "c",
                "slo_ms": 5,
                "max_inflight": 2,
                "max_queue_depth": 2,
                "cpu_units": 1,
                "memory_units": 1,
                "overflow": "queue_within_depth",
            }
        ],
        "offers": [
            {"send_time_ns": 0, "class": "c", "path": "io", "body": {"kind": "noop"}},
            {"send_time_ns": 1_000_000, "class": "c", "path": "io", "body": {"kind": "noop"}},
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
        "schema_version": 2,
        "scenario_id": "synthetic",
        "status": {"kind": "complete"},
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
        "source_content_sha256": "f" * 64,
        "lock_sha256": "c" * 64,
        "rustc": "rustc synthetic",
        "features": [],
        "topology_fingerprint": "d" * 64,
        "host_fingerprint": "e" * 64,
        "host_environment": {
            "system": "SyntheticOS",
            "release": "1",
            "machine": "synthetic",
            "cpu_model": "synthetic CPU",
            "logical_cpus": 4,
            "power_source": "ac",
            "power_mode": "normal",
        },
        "build_environment": {},
    }
    calibration = {
        "generator_headroom_ok": True,
        "observer_distorted": False,
        "producer_lag_limit_ns": 10,
    }
    return raw, scenario, identity, calibration


def encoded(value: dict) -> bytes:
    return json.dumps(value, sort_keys=True).encode()


def synthetic_resources() -> bytes:
    return encoded(
        {
            "schema_version": 2,
            "status": "complete",
            "reason": None,
            "pid": 1234,
            "process_create_time_ns": 1,
            "boot_time_ns": 1,
            "cadence_ms": 250,
            "sampling_started_monotonic_ns": 10,
            "sampling_ended_monotonic_ns": 20,
            "sampling_started_epoch_ns": 10,
            "sampling_ended_epoch_ns": 20,
            "samples": [
                {
                    "monotonic_ns": 15,
                    "cpu_user_ns": 1,
                    "cpu_system_ns": 1,
                    "rss_bytes": 4096,
                    "threads": 2,
                }
            ],
        }
    )


@lru_cache(maxsize=4)
def resolved_topology(scenario_bytes: bytes) -> bytes:
    result = subprocess.run(
        [
            "cargo",
            "run",
            "--locked",
            "--quiet",
            "-p",
            "taskmesh-bench",
            "--example",
            "host_scenario_validate",
            "--",
            "--emit-topology",
        ],
        cwd=host_perf.REPO,
        input=scenario_bytes,
        capture_output=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr.decode(errors="replace")
    return result.stdout


def complete() -> tuple[bytes, bytes, bytes, dict, dict]:
    raw, scenario, identity, calibration = fixture()
    raw_bytes, scenario_bytes = encoded(raw), encoded(scenario)
    summary = host_perf.make_summary(raw_bytes, scenario_bytes, identity, calibration, 2)
    return raw_bytes, scenario_bytes, encoded(summary), identity, calibration


def provenance(raw: bytes, scenario: bytes, identity: dict) -> dict:
    return {
        "schema_version": host_perf.PROVENANCE_VERSION,
        "status": "complete",
        "reason": None,
        "scenario_sha256": host_perf.sha256(scenario),
        "raw_sha256": host_perf.sha256(raw),
        "binary_sha256": host_perf.sha256(RUNNER_BYTES),
        "binary_artifact": "synthetic-raw.json.runner",
        "topology_sha256": host_perf.sha256(resolved_topology(scenario)),
        "topology_artifact": "synthetic-raw.json.topology.json",
        "resources_sha256": host_perf.sha256(synthetic_resources()),
        "resources_artifact": "synthetic-raw.json.resources.json",
        "runner_pid": 1234,
        "runner_mode": "full",
        "runner_flags": [],
        "build_command": [
            "cargo",
            "build",
            "--locked",
            "-p",
            "taskmesh-bench",
            "--example",
            "host_load_probe",
            "--message-format=json",
        ],
        "build_artifact_features": ["default"],
        "start_identity": identity,
        "end_identity": identity,
        "runner_exit_code": 0,
    }


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


@pytest.mark.parametrize("path", ["requested_stack_blocking", "requested_stack_async"])
def test_requested_stack_public_paths_survive_raw_to_summary_validation(path: str) -> None:
    raw, scenario, identity, calibration = fixture()
    scenario["topology"]["large_stack_slots"] = 1
    for offer, row in zip(scenario["offers"], raw["records"]):
        offer["path"] = path
        offer["stack_size_bytes"] = 2 * 1024 * 1024
        row["path"] = path
    raw_bytes, scenario_bytes = encoded(raw), encoded(scenario)
    summary = host_perf.make_summary(raw_bytes, scenario_bytes, identity, calibration, 2)
    accepted = host_perf.verify_receipt(raw_bytes, scenario_bytes, encoded(summary), identity)
    assert accepted["metrics"]["cohort_goodput"][f"c/{path}"]["slo_success"] == 2


def test_self_asserted_calibration_never_qualifies_performance() -> None:
    raw, scenario, summary, identity, _ = complete()
    assert host_perf.verify_receipt(raw, scenario, summary, identity)
    with pytest.raises(host_perf.ReceiptError, match="execution provenance"):
        host_perf.verify_receipt(raw, scenario, summary, identity, require_performance=True)
    proof = encoded(provenance(raw, scenario, identity))
    raw_obj, scenario_obj, _, calibration = fixture()
    summary_with_proof = host_perf.make_summary(
        encoded(raw_obj),
        encoded(scenario_obj),
        identity,
        calibration,
        2,
        proof,
        RUNNER_BYTES,
        resolved_topology(scenario),
        synthetic_resources(),
    )
    with pytest.raises(host_perf.ReceiptError, match="measured calibration artifact"):
        host_perf.verify_receipt(
            raw,
            scenario,
            encoded(summary_with_proof),
            identity,
            require_performance=True,
            provenance_bytes=proof,
            binary_bytes=RUNNER_BYTES,
            topology_bytes=resolved_topology(scenario),
            resource_bytes=synthetic_resources(),
        )


def test_cli_create_and_verify_stay_structural(tmp_path: Path) -> None:
    raw, scenario, _, calibration = fixture()
    raw_path = tmp_path / "raw.json"
    scenario_path = tmp_path / "scenario.json"
    calibration_path = tmp_path / "calibration.json"
    summary_path = tmp_path / "summary.json"
    for path, value in (
        (raw_path, raw),
        (scenario_path, scenario),
        (calibration_path, calibration),
    ):
        path.write_bytes(encoded(value))
    command = [
        sys.executable,
        str(Path(host_perf.__file__)),
        "create",
        str(raw_path),
        str(scenario_path),
        str(summary_path),
        "--calibration",
        str(calibration_path),
        "--p99-min-samples",
        "2",
    ]
    created = subprocess.run(command, capture_output=True, text=True, check=False)
    assert created.returncode == 0, created.stderr
    assert "performance=UNQUALIFIED" in created.stdout
    verified = subprocess.run(
        [
            sys.executable,
            str(Path(host_perf.__file__)),
            "verify",
            str(raw_path),
            str(scenario_path),
            str(summary_path),
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    assert verified.returncode == 0, verified.stderr
    assert "performance=UNQUALIFIED" in verified.stdout
    claimed = subprocess.run(
        [
            sys.executable,
            str(Path(host_perf.__file__)),
            "verify",
            str(raw_path),
            str(scenario_path),
            str(summary_path),
            "--require-performance",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    assert claimed.returncode != 0
    assert "STRUCTURALLY_VALID" not in claimed.stdout


def test_execution_provenance_binds_run_start_end_and_artifact_digests() -> None:
    raw, scenario, identity, calibration = fixture()
    raw_bytes, scenario_bytes = encoded(raw), encoded(scenario)
    proof = provenance(raw_bytes, scenario_bytes, identity)
    proof_bytes = encoded(proof)
    summary = host_perf.make_summary(
        raw_bytes,
        scenario_bytes,
        identity,
        calibration,
        2,
        proof_bytes,
        RUNNER_BYTES,
        resolved_topology(scenario_bytes),
        synthetic_resources(),
    )
    assert summary["execution_provenance_sha256"] == host_perf.sha256(proof_bytes)
    host_perf.verify_receipt(
        raw_bytes,
        scenario_bytes,
        encoded(summary),
        identity,
        provenance_bytes=proof_bytes,
        binary_bytes=RUNNER_BYTES,
        topology_bytes=resolved_topology(scenario_bytes),
        resource_bytes=synthetic_resources(),
    )
    with pytest.raises(host_perf.ReceiptError):
        host_perf.verify_receipt(raw_bytes, scenario_bytes, encoded(summary), identity)
    proof["end_identity"] = {**identity, "source_head": "changed"}
    with pytest.raises(host_perf.ReceiptError, match="changed during run"):
        host_perf.make_summary(
            raw_bytes,
            scenario_bytes,
            identity,
            calibration,
            2,
            encoded(proof),
            RUNNER_BYTES,
            resolved_topology(scenario_bytes),
            synthetic_resources(),
        )
    proof["end_identity"] = identity
    proof["raw_sha256"] = "0" * 64
    with pytest.raises(host_perf.ReceiptError, match="digest mismatch"):
        host_perf.make_summary(
            raw_bytes,
            scenario_bytes,
            identity,
            calibration,
            2,
            encoded(proof),
            RUNNER_BYTES,
            resolved_topology(scenario_bytes),
            synthetic_resources(),
        )
    proof["raw_sha256"] = host_perf.sha256(raw_bytes)
    proof["build_command"].extend(["--features", "taskmesh/rayon"])
    with pytest.raises(host_perf.ReceiptError, match="build command or features"):
        host_perf.make_summary(
            raw_bytes,
            scenario_bytes,
            identity,
            calibration,
            2,
            encoded(proof),
            RUNNER_BYTES,
            resolved_topology(scenario_bytes),
            synthetic_resources(),
        )
    with pytest.raises(host_perf.ReceiptError, match="binary artifact digest mismatch"):
        host_perf.make_summary(
            raw_bytes,
            scenario_bytes,
            identity,
            calibration,
            2,
            encoded(provenance(raw_bytes, scenario_bytes, identity)),
            b"tampered",
            resolved_topology(scenario_bytes),
            synthetic_resources(),
        )
    changed_topology = json.loads(resolved_topology(scenario_bytes))
    changed_topology["cpu_executor"]["declared_workers"] = 999
    changed_topology_bytes = encoded(changed_topology)
    forged_proof = provenance(raw_bytes, scenario_bytes, identity)
    forged_proof["topology_sha256"] = host_perf.sha256(changed_topology_bytes)
    with pytest.raises(host_perf.ReceiptError, match="topology artifact differs"):
        host_perf.make_summary(
            raw_bytes,
            scenario_bytes,
            identity,
            calibration,
            2,
            encoded(forged_proof),
            RUNNER_BYTES,
            changed_topology_bytes,
            synthetic_resources(),
        )
    with pytest.raises(host_perf.ReceiptError, match="resource artifact digest mismatch"):
        host_perf.make_summary(
            raw_bytes,
            scenario_bytes,
            identity,
            calibration,
            2,
            encoded(provenance(raw_bytes, scenario_bytes, identity)),
            RUNNER_BYTES,
            resolved_topology(scenario_bytes),
            b"tampered",
        )


def test_host_run_wrapper_binds_real_binary_and_raw(tmp_path: Path) -> None:
    _, scenario, _, calibration = fixture()
    scenario_path = tmp_path / "scenario.json"
    raw_path = tmp_path / "raw.json"
    summary_path = tmp_path / "summary.json"
    calibration_path = tmp_path / "calibration.json"
    scenario_path.write_bytes(encoded(scenario))
    calibration_path.write_bytes(encoded(calibration))
    command = [
        sys.executable,
        str(Path(host_perf.__file__).with_name("host_run.py")),
        str(scenario_path),
        str(raw_path),
        str(summary_path),
        str(calibration_path),
    ]
    run = subprocess.run(command, capture_output=True, text=True, check=False)
    assert run.returncode == 0, run.stderr
    assert "performance=UNQUALIFIED" in run.stdout
    proof_path = raw_path.with_name(raw_path.name + ".provenance.json")
    binary_path = raw_path.with_name(raw_path.name + ".runner")
    topology_path = raw_path.with_name(raw_path.name + ".topology.json")
    resource_path = raw_path.with_name(raw_path.name + ".resources.json")
    proof_bytes = proof_path.read_bytes()
    proof = json.loads(proof_bytes)
    assert proof["status"] == "complete"
    assert proof["binary_artifact"] == binary_path.name
    assert proof["binary_sha256"] == host_perf.sha256(binary_path.read_bytes())
    assert proof["topology_artifact"] == topology_path.name
    assert proof["topology_sha256"] == host_perf.sha256(topology_path.read_bytes())
    assert proof["resources_artifact"] == resource_path.name
    assert proof["resources_sha256"] == host_perf.sha256(resource_path.read_bytes())
    assert proof["runner_pid"] == json.loads(resource_path.read_bytes())["pid"]
    summary = json.loads(summary_path.read_bytes())
    assert summary["execution_provenance_sha256"] == host_perf.sha256(proof_bytes)
    host_perf.verify_receipt(
        raw_path.read_bytes(),
        scenario_path.read_bytes(),
        summary_path.read_bytes(),
        proof["end_identity"],
        provenance_bytes=proof_bytes,
        binary_bytes=binary_path.read_bytes(),
        topology_bytes=topology_path.read_bytes(),
        resource_bytes=resource_path.read_bytes(),
    )
    cli = subprocess.run(
        [
            sys.executable,
            str(Path(host_perf.__file__)),
            "verify",
            str(raw_path),
            str(scenario_path),
            str(summary_path),
            "--provenance",
            str(proof_path),
            "--binary",
            str(binary_path),
            "--topology",
            str(topology_path),
            "--resources",
            str(resource_path),
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    assert cli.returncode == 0, cli.stderr
    assert "performance=UNQUALIFIED" in cli.stdout
    tampered_raw = json.loads(raw_path.read_bytes())
    tampered_raw["records"][0]["caller_response_ns"] = None
    with pytest.raises(host_perf.ReceiptError, match="Rust scenario preflight"):
        host_perf.verify_receipt(
            encoded(tampered_raw),
            scenario_path.read_bytes(),
            summary_path.read_bytes(),
            proof["end_identity"],
            provenance_bytes=proof_bytes,
            binary_bytes=binary_path.read_bytes(),
            topology_bytes=topology_path.read_bytes(),
            resource_bytes=resource_path.read_bytes(),
        )
    with pytest.raises(host_perf.ReceiptError, match="retained runner binary is required"):
        host_perf.verify_receipt(
            raw_path.read_bytes(),
            scenario_path.read_bytes(),
            summary_path.read_bytes(),
            proof["end_identity"],
            provenance_bytes=proof_bytes,
            topology_bytes=topology_path.read_bytes(),
            resource_bytes=resource_path.read_bytes(),
        )


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
        host_perf.make_summary(encoded(raw), encoded(scenario), identity, calibration, 2)


def test_forged_rejection_variant_is_not_a_typed_verdict() -> None:
    raw, scenario, identity, calibration = fixture()
    row = raw["records"][1]
    row["body_started_ns"] = None
    row["body_finished_ns"] = None
    row["disposition"]["outcome"] = {"kind": "rejected", "verdict": "NotAnAdmissionVerdict"}
    for field in ("admitted", "started", "terminated"):
        raw["class_counters"]["c"][field] = 1
    with pytest.raises(host_perf.ReceiptError, match="Rust scenario preflight"):
        host_perf.make_summary(encoded(raw), encoded(scenario), identity, calibration, 1)


def test_failed_runner_persists_one_row_per_offer_and_is_rejected(tmp_path: Path) -> None:
    _, scenario, identity, calibration = fixture()
    scenario_path = tmp_path / "scenario.json"
    raw_path = tmp_path / "failed-raw.json"
    scenario_path.write_bytes(encoded(scenario))
    failed = subprocess.run(
        [
            "cargo",
            "run",
            "--locked",
            "--quiet",
            "-p",
            "taskmesh-bench",
            "--example",
            "host_load_probe",
            "--",
            str(scenario_path),
            str(raw_path),
            "--inject-producer-before-first-offer",
        ],
        cwd=host_perf.REPO,
        capture_output=True,
        text=True,
        check=False,
    )
    assert failed.returncode != 0
    raw = json.loads(raw_path.read_bytes())
    assert raw["status"]["kind"] == "invalid"
    assert "producer" in raw["status"]["reason"]
    assert len(raw["records"]) == len(scenario["offers"])
    with pytest.raises(host_perf.ReceiptError, match="invalid host run"):
        host_perf.make_summary(
            raw_path.read_bytes(), scenario_path.read_bytes(), identity, calibration, 1
        )


def test_explicit_invalid_status_cannot_be_summarized() -> None:
    raw, scenario, identity, calibration = fixture()
    raw["status"] = {"kind": "invalid", "reason": "injected"}
    with pytest.raises(host_perf.ReceiptError, match="invalid host run"):
        host_perf.make_summary(encoded(raw), encoded(scenario), identity, calibration, 1)


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


def test_legacy_identity_without_content_custody_rejects() -> None:
    _, _, identity, _ = fixture()
    del identity["source_content_sha256"]
    with pytest.raises(host_perf.ReceiptError, match="identity: missing/unknown fields"):
        host_perf.validate_identity(identity)


@pytest.mark.parametrize("digest", [None, "a", "A" * 64, True])
def test_identity_requires_exact_content_digest(digest: object) -> None:
    _, _, identity, _ = fixture()
    identity["source_content_sha256"] = digest
    with pytest.raises(host_perf.ReceiptError, match="source_content_sha256"):
        host_perf.validate_identity(identity)


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


@pytest.mark.parametrize("name", ["tenant/io", "tenant/team/interactive", "팀/cpu"])
def test_cohort_slo_preserves_slashes_in_class_identity(name: str) -> None:
    raw, scenario, _identity, _calibration = fixture()
    scenario["classes"][0]["name"] = name
    for offer in scenario["offers"]:
        offer["class"] = name
    for row in raw["records"]:
        row["class"] = name
    raw["class_counters"][name] = raw["class_counters"].pop("c")
    metrics = host_perf.analyze_raw(raw, scenario)
    assert metrics["cohort_goodput"][f"{name}/io"]["slo_ns"] == 5_000_000
    assert metrics["cohort_goodput"][f"{name}/io"]["slo_fraction"] == 1.0
