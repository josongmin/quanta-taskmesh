"""Measured-control budgets are diagnostic and fail closed on missing evidence."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_control_assess  # noqa: E402
import host_perf  # noqa: E402


def encoded(value: dict) -> bytes:
    return host_perf.canonical(value) + b"\n"


def study_run(mode: str, rate: float = 2.0, responded: int = 10) -> dict:
    return {
        "mode": mode,
        "metrics": {
            "cohort_goodput": {"c/io": {"slo_per_sec": rate}},
            "success_latency_by_class_path": {"c/io": {"count": 10, "p99_ns": 100}},
            "counts": {"intended": 10, "responded": responded},
        },
    }


def setup_assessment(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> tuple[dict, tuple]:
    scenario = tmp_path / "scenario.json"
    scenario.write_bytes(b"scenario")
    raw = tmp_path / "target.json"
    summary = tmp_path / "summary.json"
    generator = tmp_path / "generator.json"
    aa = tmp_path / "aa"
    snapshot = tmp_path / "snapshot"
    recorder = tmp_path / "recorder"
    sampler = tmp_path / "sampler"
    control_bundle = tmp_path / "control-bundle.json"
    for directory in (aa, snapshot, recorder, sampler):
        directory.mkdir()
    control_bundle.write_bytes(b"{}")
    identity = {"source_dirty": False, "source_head": "a" * 40}
    bundle = {
        "identity": identity,
        "host_binary_sha256": "one-binary",
        "observed_span_ns": 250,
        "boot_time_ns": 1,
        "resource_cadence_ms": 250,
        "generator_not_submitted": 0,
        "host_not_submitted": 0,
        "generator_max_lag_ns": 5,
        "host_max_lag_ns": 5,
    }
    monkeypatch.setattr(host_control_assess.host_controls, "verify_bundle", lambda *_args: bundle)
    for path, start in ((raw, 10), (generator, 30)):
        host_control_assess.host_controls.sidecars(path)["resources"].write_bytes(
            encoded({"sampling_started_epoch_ns": start, "sampling_ended_epoch_ns": start + 10})
        )
    aa_runs = [study_run("A1", 2.0), study_run("A2", 2.1)] * 2
    snapshot_runs = [
        study_run("on", 2.1),
        study_run("off", 2.0),
        study_run("off", 2.0),
        study_run("on", 2.1),
    ]
    recorder_runs = [
        study_run("full", responded=9),
        study_run("minimal"),
        study_run("minimal"),
        study_run("full", responded=9),
    ]
    for directory, runs, first_start in (
        (aa, aa_runs, 50),
        (snapshot, snapshot_runs, 130),
        (recorder, recorder_runs, 210),
    ):
        (directory / f"{directory.name}-bundle.json").write_bytes(encoded({"runs": runs}))
        for index in range(len(runs)):
            start = first_start + index * 20
            host_control_assess.host_aa.paths(directory, index)["resources"].write_bytes(
                encoded({"sampling_started_epoch_ns": start, "sampling_ended_epoch_ns": start + 10})
            )
    sampler_runs = []
    for index, mode in enumerate(("on", "off", "off", "on")):
        row = study_run(mode, 2.1 if mode == "on" else 2.0)
        start = 300 + index * 20
        row["window_epoch_ns"] = [start, start + 10]
        row["boot_time_ns"] = 1
        row["resource_cadence_ms"] = 250
        sampler_runs.append(row)
    sampler_bundle = {
        "identity": identity,
        "binary_sha256": "one-binary",
        "scenario_sha256": host_perf.sha256(scenario.read_bytes()),
        "runs": sampler_runs,
    }
    (sampler / "sampler-bundle.json").write_bytes(encoded(sampler_bundle))
    monkeypatch.setattr(
        host_control_assess.host_sampler,
        "verify_bundle",
        lambda data, *_args: host_perf.parse_object(data, "sampler fixture"),
    )
    policy = {
        "schema_version": 1,
        "scenario_sha256": host_perf.sha256(scenario.read_bytes()),
        "source_head": "a" * 40,
        "min_pairs": 2,
        "min_success_samples_per_cohort": 10,
        "max_span_ns": 1000,
        "max_generator_lag_ns": 10,
        "max_host_lag_ns": 10,
        "max_aa_relative_delta": 0.1,
        "max_aa_p99_relative_delta": 0.1,
        "max_snapshot_relative_delta": 0.1,
        "max_snapshot_p99_relative_delta": 0.1,
        "max_recorder_response_fraction_delta": 0.2,
        "max_sampler_relative_delta": 0.1,
        "max_sampler_p99_relative_delta": 0.1,
    }
    return policy, (
        scenario,
        raw,
        summary,
        generator,
        aa,
        snapshot,
        recorder,
        sampler,
        control_bundle,
    )


def test_control_assessment_recomputes_budgets_and_never_qualifies_performance(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    policy, paths = setup_assessment(tmp_path, monkeypatch)
    report = host_control_assess.assess(encoded(policy), *paths)
    assert report["status"] == "BUDGET_PASS_DIAGNOSTIC"
    assert report["performance"] == "UNQUALIFIED"
    assert report["observations"]["sampler_pairs"] == 2
    assert report["observations"]["combined_span_ns"] == 360

    policy["max_sampler_relative_delta"] = 0.01
    rejected = host_control_assess.assess(encoded(policy), *paths)
    assert rejected["status"] == "BUDGET_FAIL"
    assert rejected["violations"] == ["max_sampler_relative_delta"]

    sampler_path = paths[7] / "sampler-bundle.json"
    sampler = json.loads(sampler_path.read_bytes())
    sampler["runs"][0]["metrics"]["success_latency_by_class_path"]["c/io"]["p99_ns"] = 150
    sampler_path.write_bytes(encoded(sampler))
    policy["max_sampler_relative_delta"] = 0.1
    rejected = host_control_assess.assess(encoded(policy), *paths)
    assert rejected["violations"] == ["max_sampler_p99_relative_delta"]

    policy["min_success_samples_per_cohort"] = 11
    with pytest.raises(host_perf.ReceiptError, match="thin p99 population"):
        host_control_assess.assess(encoded(policy), *paths)


def test_control_assessment_rejects_unfrozen_or_overlapping_evidence(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    policy, paths = setup_assessment(tmp_path, monkeypatch)
    policy["max_aa_relative_delta"] = None
    with pytest.raises(host_perf.ReceiptError, match="finite fraction"):
        host_control_assess.assess(encoded(policy), *paths)
    policy["max_aa_relative_delta"] = 0.1
    sampler_path = paths[7] / "sampler-bundle.json"
    sampler = json.loads(sampler_path.read_bytes())
    sampler["runs"][0]["window_epoch_ns"] = [15, 25]
    sampler_path.write_bytes(encoded(sampler))
    with pytest.raises(host_perf.ReceiptError, match="windows overlap"):
        host_control_assess.assess(encoded(policy), *paths)


@pytest.mark.parametrize(
    ("directory_index", "bundle_name", "violation"),
    [
        (4, "aa-bundle.json", "max_aa_p99_relative_delta"),
        (5, "snapshot-bundle.json", "max_snapshot_p99_relative_delta"),
    ],
)
def test_control_assessment_enforces_each_observer_p99_budget(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    directory_index: int,
    bundle_name: str,
    violation: str,
) -> None:
    policy, paths = setup_assessment(tmp_path, monkeypatch)
    bundle_path = paths[directory_index] / bundle_name
    bundle = json.loads(bundle_path.read_bytes())
    bundle["runs"][0]["metrics"]["success_latency_by_class_path"]["c/io"]["p99_ns"] = 150
    bundle_path.write_bytes(encoded(bundle))
    report = host_control_assess.assess(encoded(policy), *paths)
    assert report["status"] == "BUDGET_FAIL"
    assert report["violations"] == [violation]
