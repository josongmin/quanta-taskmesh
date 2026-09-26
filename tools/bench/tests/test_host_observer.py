"""Snapshot on/off artifacts bind one executable and one workload definition."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_observer  # noqa: E402
import host_perf  # noqa: E402

SCENARIO = host_perf.REPO / "tools/bench/scenarios/h1-default-send-smoke.json"


def test_snapshot_pair_keeps_work_constant_and_rejects_tamper(tmp_path: Path) -> None:
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
    directory = tmp_path / "snapshot"
    bundle = host_observer.acquire(SCENARIO, calibration, directory, 1, 10)
    assert bundle["status"] == "diagnostic_complete"
    assert [run["mode"] for run in bundle["runs"]] == ["on", "off"]
    assert bundle["runs"][0]["binary_sha256"] == bundle["runs"][1]["binary_sha256"]
    bundle_bytes = (directory / "snapshot-bundle.json").read_bytes()
    host_observer.verify_bundle(bundle_bytes, directory)

    swapped = json.loads(bundle_bytes)
    swapped["runs"][0]["mode"] = "off"
    with pytest.raises(host_perf.ReceiptError, match="pair order or mode"):
        host_observer.verify_bundle(json.dumps(swapped).encode(), directory)
    changed = json.loads((directory / "scenario-off.json").read_bytes())
    changed["offers"][0]["class"] = "batch"
    (directory / "scenario-off.json").write_text(json.dumps(changed))
    with pytest.raises(host_perf.ReceiptError, match="scenario artifact differs"):
        host_observer.verify_bundle(bundle_bytes, directory)


def test_snapshot_study_requires_positive_cadence() -> None:
    assert [host_observer.mode_for(index) for index in range(4)] == [
        "on",
        "off",
        "off",
        "on",
    ]
    with pytest.raises(host_perf.ReceiptError, match="cadence"):
        host_observer.scenario_pair(SCENARIO.read_bytes(), 0)
