"""Minimal recorder controls cannot be admitted as response-latency results."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_perf  # noqa: E402

SCENARIO = host_perf.REPO / "tools/bench/scenarios/h1-default-send-smoke.json"
RUNNER = host_perf.REPO / "tools/bench/minimal_run.py"


def test_real_minimal_control_binds_same_host_binary_and_rejects_fabricated_latency(
    tmp_path: Path,
) -> None:
    raw_path = tmp_path / "minimal.json"
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
    provenance = host_perf.parse_object(provenance_bytes, "minimal provenance")
    raw = host_perf.parse_object(raw_bytes, "minimal raw")
    assert raw["recorder_mode"] == "minimal"
    assert raw["response_latency_available"] is False
    assert "caller_response_ns" not in raw["records"][0]
    assert provenance["status"] == "complete"
    host_perf.validate_with_rust(
        scenario_bytes, raw_bytes, [], topology_bytes, raw_kind="minimal"
    )
    host_perf.validate_execution_provenance(
        provenance_bytes,
        raw_bytes,
        scenario_bytes,
        provenance["end_identity"],
        binary_bytes,
        topology_bytes,
        resource_bytes,
    )
    raw["response_latency_available"] = True
    with pytest.raises(host_perf.ReceiptError, match="Rust scenario preflight failed"):
        host_perf.validate_with_rust(
            scenario_bytes,
            json.dumps(raw).encode(),
            [],
            topology_bytes,
            raw_kind="minimal",
        )


def test_minimal_cli_persists_invalid_rows_on_producer_failure(tmp_path: Path) -> None:
    raw_path = tmp_path / "invalid-minimal.json"
    result = subprocess.run(
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
            str(SCENARIO),
            str(raw_path),
            "--recorder-minimal",
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
            SCENARIO.read_bytes(), raw_path.read_bytes(), [], raw_kind="minimal"
        )
