"""Generator artifact checks do not assert a host performance threshold."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_perf  # noqa: E402

SCENARIO = host_perf.REPO / "tools/bench/scenarios/h1-default-send-smoke.json"
RUNNER = host_perf.REPO / "tools/bench/generator_run.py"


def test_real_generator_control_binds_binary_topology_and_raw(tmp_path: Path) -> None:
    raw_path = tmp_path / "generator.json"
    result = subprocess.run(
        [sys.executable, str(RUNNER), str(SCENARIO), str(raw_path)],
        cwd=host_perf.REPO,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    raw_bytes = raw_path.read_bytes()
    scenario_bytes = SCENARIO.read_bytes()
    provenance_bytes = raw_path.with_name(raw_path.name + ".provenance.json").read_bytes()
    binary_bytes = raw_path.with_name(raw_path.name + ".runner").read_bytes()
    topology_bytes = raw_path.with_name(raw_path.name + ".topology.json").read_bytes()
    resource_bytes = raw_path.with_name(raw_path.name + ".resources.json").read_bytes()
    provenance = host_perf.parse_object(provenance_bytes, "generator provenance")
    assert provenance["status"] == "complete"
    assert provenance["runner_mode"] == "generator"
    assert provenance["runner_flags"] == []
    assert host_perf.sha256(raw_bytes) == provenance["raw_sha256"]
    assert host_perf.sha256(binary_bytes) == provenance["binary_sha256"]
    host_perf.validate_with_rust(
        scenario_bytes, raw_bytes, [], topology_bytes, raw_kind="generator"
    )
    host_perf.validate_execution_provenance(
        provenance_bytes,
        raw_bytes,
        scenario_bytes,
        provenance["end_identity"],
        binary_bytes,
        topology_bytes,
        resource_bytes,
        example_name="host_generator_probe",
        runner_mode="generator",
    )
    with pytest.raises(host_perf.ReceiptError, match="runner mode or flags"):
        host_perf.validate_execution_provenance(
            provenance_bytes,
            raw_bytes,
            scenario_bytes,
            provenance["end_identity"],
            binary_bytes,
            topology_bytes,
            resource_bytes,
        )
    forged_mode = dict(provenance)
    forged_mode["runner_mode"] = "full"
    with pytest.raises(host_perf.ReceiptError, match="runner mode"):
        host_perf.validate_execution_provenance(
            json.dumps(forged_mode).encode(),
            raw_bytes,
            scenario_bytes,
            provenance["end_identity"],
            binary_bytes,
            topology_bytes,
            resource_bytes,
            example_name="host_generator_probe",
            runner_mode="generator",
        )
    tampered = json.loads(raw_bytes)
    tampered["records"][0]["scheduled_lag_ns"] += 1
    with pytest.raises(host_perf.ReceiptError, match="Rust scenario preflight failed"):
        host_perf.validate_with_rust(
            scenario_bytes,
            json.dumps(tampered).encode(),
            [],
            topology_bytes,
            raw_kind="generator",
        )


def test_generator_cli_persists_invalid_rows_on_producer_failure(tmp_path: Path) -> None:
    raw_path = tmp_path / "invalid-generator.json"
    result = subprocess.run(
        [
            "cargo",
            "run",
            "--locked",
            "--quiet",
            "-p",
            "taskmesh-bench",
            "--example",
            "host_generator_probe",
            "--",
            str(SCENARIO),
            str(raw_path),
            "--inject-producer-before-first-offer",
        ],
        cwd=host_perf.REPO,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode != 0
    raw = json.loads(raw_path.read_bytes())
    assert raw["status"]["kind"] == "invalid"
    assert len(raw["records"]) == len(json.loads(SCENARIO.read_bytes())["offers"])
    assert raw["records"][0]["disposition"] == "pending"
    with pytest.raises(host_perf.ReceiptError, match="Rust scenario preflight failed"):
        host_perf.validate_with_rust(
            SCENARIO.read_bytes(), raw_path.read_bytes(), [], raw_kind="generator"
        )
