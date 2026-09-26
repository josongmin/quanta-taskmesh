"""Pairing rules are structural; synthetic values never qualify performance."""

from __future__ import annotations

import copy
import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_compare  # noqa: E402
import host_perf  # noqa: E402


def manifest(tmp_path: Path) -> dict:
    scenario = {
        "load": {"injection_ms": 1000, "max_records": 4},
        "topology": {"cpu_workers": 1},
        "classes": [{"name": "c", "slo_ms": 10}],
        "offers": [
            {
                "send_time_ns": index * 250_000_000,
                "class": "c",
                "path": "io",
                "body": {"kind": "noop"},
            }
            for index in range(4)
        ],
    }
    (tmp_path / "scenario.json").write_text(json.dumps(scenario))
    return {
        "schema_version": 1,
        "max_span_ns": 1_000,
        "points": [
            {
                "rate_per_second": 4,
                "scenario": "scenario.json",
                "pairs": [
                    {
                        "order": "baseline_candidate" if index % 2 == 0 else "candidate_baseline",
                        "baseline": {"index": index, "arm": "baseline"},
                        "candidate": {"index": index, "arm": "candidate"},
                    }
                    for index in range(4)
                ],
            }
        ],
    }


def fake_run(_root: Path, files: dict, _scenario: bytes, _label: str) -> dict:
    index, arm = files["index"], files["arm"]
    baseline_first = index % 2 == 0
    first = (arm == "baseline") == baseline_first
    start = 100 + index * 20 + (0 if first else 10)
    return {
        "identity": {
            "source_head": arm,
            "source_dirty": False,
            "rustc": "rustc same",
            "features": [],
            "topology_fingerprint": "topology",
            "host_fingerprint": "host",
            "host_environment": {"machine": "same"},
            "build_environment": {},
        },
        "metrics": {
            "cohort_goodput": {
                "c/io": {
                    "intended": 4,
                    "slo_ns": 10_000_000,
                    "slo_per_sec": 2.0 if arm == "baseline" else 3.0,
                }
            }
        },
        "binary_sha256": arm,
        "provenance_sha256": f"{index}-{arm}",
        "artifact_sha256": {"raw": f"raw-{index}-{arm}"},
        "window": (start, start + 5),
        "boot_time_ns": 1,
        "resource_cadence_ms": 250,
    }


def test_balanced_independent_pairs_report_only_descriptive_effect(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.setattr(host_compare, "read_run", fake_run)
    candidate = manifest(tmp_path)
    report = host_compare.compare(json.dumps(candidate).encode(), tmp_path)
    assert report["performance"] == "UNQUALIFIED"
    assert report["status"] == "DESCRIPTIVE_ONLY"
    assert len(report["run_artifacts"]) == 8
    effect = report["points"][0]["cohorts"]["c/io"]
    assert effect["paired_differences"] == [1.0] * 4
    assert effect["descriptive_bootstrap_95_interval"] == [1.0, 1.0]


@pytest.mark.parametrize(
    ("change", "error"),
    [
        ("unbalanced", "arm order is not balanced"),
        ("rate", "declared rate differs"),
        ("reused", "run was reused"),
        ("overlap", "arm windows overlap"),
        ("span", "exceeds declared span"),
    ],
)
def test_comparison_rejects_nonindependent_or_changed_population(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path, change: str, error: str
) -> None:
    monkeypatch.setattr(host_compare, "read_run", fake_run)
    candidate = manifest(tmp_path)
    point = candidate["points"][0]
    if change == "unbalanced":
        for index in (4, 6):
            extra = copy.deepcopy(point["pairs"][0])
            extra["baseline"]["index"] = index
            extra["candidate"]["index"] = index
            point["pairs"].append(extra)
    elif change == "rate":
        point["rate_per_second"] = 5
    elif change == "reused":
        point["pairs"].append(copy.deepcopy(point["pairs"][0]))
    elif change == "span":
        candidate["max_span_ns"] = 1
    else:
        point["pairs"][0]["order"] = "candidate_baseline"
    with pytest.raises(host_perf.ReceiptError, match=error):
        host_compare.compare(json.dumps(candidate).encode(), tmp_path)


def test_effect_interval_rejects_request_level_pseudoreplication() -> None:
    with pytest.raises(host_perf.ReceiptError, match="independent pairs"):
        host_compare.effect_interval([1.0, 1.0, 1.0])


def test_rejected_comparison_retains_failure_receipt(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    source = tmp_path / "manifest.json"
    output = tmp_path / "comparison.json"
    source.write_text('{"schema_version":1,"max_span_ns":1,"points":[]}')
    monkeypatch.setattr(sys, "argv", ["host_compare.py", str(source), str(output)])
    assert host_compare.main() == 1
    rejected = json.loads(output.read_bytes())
    assert rejected["status"] == "REJECTED"
    assert rejected["performance"] == "UNQUALIFIED"
    assert rejected["manifest_sha256"] == host_perf.sha256(source.read_bytes())
