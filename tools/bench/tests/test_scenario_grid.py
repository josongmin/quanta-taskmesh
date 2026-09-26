"""Finite rate point expansion preserves work and exposes its exact denominator."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_perf  # noqa: E402
from scenario_grid import uniform_rate_point  # noqa: E402

TEMPLATE = host_perf.REPO / "tools/bench/scenarios/h1-blocking-rate-template.json"


def test_uniform_rate_expansion_keeps_body_and_absolute_schedule() -> None:
    source = json.loads(TEMPLATE.read_bytes())
    result = uniform_rate_point(source, 25)
    assert len(result["offers"]) == 1500
    assert result["load"]["max_records"] == 1500
    assert result["offers"][0]["send_time_ns"] == 0
    assert result["offers"][-1]["send_time_ns"] == 59_960_000_000
    assert {row["class"] for row in result["offers"]} == {"work"}
    assert {row["path"] for row in result["offers"]} == {"blocking"}
    assert {json.dumps(row["body"], sort_keys=True) for row in result["offers"]} == {
        json.dumps(source["offers"][0]["body"], sort_keys=True)
    }
    assert result["topology"] == source["topology"]
    assert result["load"]["max_outstanding"] == source["load"]["max_outstanding"]
    for invalid in (0, -1, True):
        with pytest.raises(ValueError, match="positive integer"):
            uniform_rate_point(source, invalid)
    with pytest.raises(ValueError, match="record bound"):
        uniform_rate_point(source, 1_000_000)
    multiple = json.loads(TEMPLATE.read_bytes())
    multiple["offers"].append(multiple["offers"][0])
    with pytest.raises(ValueError, match="exactly one"):
        uniform_rate_point(multiple, 25)
