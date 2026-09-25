"""A/A pairs bind actual host artifacts but do not set a timing threshold."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_aa  # noqa: E402
import host_perf  # noqa: E402

SCENARIO = host_perf.REPO / "tools/bench/scenarios/h1-default-send-smoke.json"


def test_fresh_process_aa_pair_rejects_swapped_identity_and_incomplete_pair(
    tmp_path: Path,
) -> None:
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
    directory = tmp_path / "aa"
    bundle = host_aa.acquire(SCENARIO, calibration, directory, 1)
    assert bundle["status"] == "diagnostic_complete"
    assert len(bundle["runs"]) == 2
    assert bundle["runs"][0]["binary_sha256"] == bundle["runs"][1]["binary_sha256"]
    bundle_bytes = (directory / "aa-bundle.json").read_bytes()
    host_aa.verify_bundle(bundle_bytes, directory, SCENARIO.read_bytes())

    swapped = json.loads(bundle_bytes)
    swapped["runs"][1]["binary_sha256"] = "0" * 64
    with pytest.raises(host_perf.ReceiptError, match="artifact or metric differs"):
        host_aa.verify_bundle(json.dumps(swapped).encode(), directory, SCENARIO.read_bytes())
    incomplete = json.loads(bundle_bytes)
    incomplete["runs"].pop()
    with pytest.raises(host_perf.ReceiptError, match="complete pairs"):
        host_aa.verify_bundle(json.dumps(incomplete).encode(), directory, SCENARIO.read_bytes())


def test_aa_bundle_rejects_missing_or_self_asserted_runs(tmp_path: Path) -> None:
    claimed = {
        "schema_version": 1,
        "kind": "same_binary_aa",
        "status": "diagnostic_complete",
        "scenario_sha256": host_perf.sha256(SCENARIO.read_bytes()),
        "binary_sha256": "0" * 64,
        "identity": {},
        "runs": [{"index": 0}, {"index": 1}],
        "failures": [],
    }
    with pytest.raises(host_perf.ReceiptError):
        host_aa.verify_bundle(json.dumps(claimed).encode(), tmp_path, SCENARIO.read_bytes())
