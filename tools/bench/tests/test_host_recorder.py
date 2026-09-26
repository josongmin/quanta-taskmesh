"""Full/minimal recorder pairs stay diagnostic and reject forged latency."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_perf  # noqa: E402
import host_recorder  # noqa: E402

SCENARIO = host_perf.REPO / "tools/bench/scenarios/h1-default-send-smoke.json"


def test_recorder_pair_uses_one_binary_and_rejects_forged_mode(tmp_path: Path) -> None:
    calibration = tmp_path / "diagnostic-calibration.json"
    calibration.write_text(
        json.dumps(
            {
                "generator_headroom_ok": True,
                "observer_distorted": False,
                "producer_lag_limit_ns": 1_000_000_000,
            }
        )
    )
    directory = tmp_path / "recorder"
    bundle = host_recorder.acquire(SCENARIO, calibration, directory, 1)
    assert bundle["status"] == "diagnostic_complete"
    assert [run["mode"] for run in bundle["runs"]] == ["full", "minimal"]
    assert bundle["runs"][0]["binary_sha256"] == bundle["runs"][1]["binary_sha256"]
    assert bundle["runs"][1]["metrics"]["success_latency_by_class_path"] is None
    bundle_bytes = (directory / "recorder-bundle.json").read_bytes()
    host_recorder.verify_bundle(bundle_bytes, directory, SCENARIO.read_bytes())

    swapped = json.loads(bundle_bytes)
    swapped["runs"][1]["mode"] = "full"
    with pytest.raises(host_perf.ReceiptError, match="pair order or mode"):
        host_recorder.verify_bundle(json.dumps(swapped).encode(), directory, SCENARIO.read_bytes())
    fabricated = json.loads(bundle_bytes)
    fabricated["runs"][1]["metrics"]["success_latency_by_class_path"] = {"c/io": 0}
    with pytest.raises(host_perf.ReceiptError, match="artifact or metric differs"):
        host_recorder.verify_bundle(
            json.dumps(fabricated).encode(), directory, SCENARIO.read_bytes()
        )


def test_recorder_pair_order_and_snapshot_preflight(tmp_path: Path) -> None:
    assert [host_recorder.mode_for(index) for index in range(4)] == [
        "full",
        "minimal",
        "minimal",
        "full",
    ]
    scenario = json.loads(SCENARIO.read_bytes())
    scenario["load"]["snapshot_ms"] = 10
    scenario_path = tmp_path / "snapshot-on.json"
    scenario_path.write_text(json.dumps(scenario))
    with pytest.raises(host_perf.ReceiptError, match="Snapshot sampling off"):
        host_recorder.acquire(scenario_path, tmp_path / "unused.json", tmp_path / "out", 1)
