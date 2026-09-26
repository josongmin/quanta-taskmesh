"""Measured-control budgets are diagnostic and fail closed on missing evidence."""

from __future__ import annotations

import copy
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
            "counts": {
                "intended": 10,
                "responded": responded,
                "not_submitted": 0,
                "unanswered_at_settlement": 0,
            },
            "max_producer_lag_ns": 5,
            "late_snapshot_samples": 0,
        },
    }


def retain_summary(directory: Path, index: int, run: dict) -> None:
    if run["mode"] == "minimal":
        run["summary_sha256"] = None
        return
    data = encoded({"metrics": run["metrics"]})
    host_control_assess.host_aa.paths(directory, index)["summary"].write_bytes(data)
    run["summary_sha256"] = host_perf.sha256(data)


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
        "scenario_sha256": host_perf.sha256(scenario.read_bytes()),
        "host_binary_sha256": "one-binary",
        "observed_span_ns": 250,
        "boot_time_ns": 1,
        "resource_cadence_ms": 250,
        "generator_not_submitted": 0,
        "host_not_submitted": 0,
        "generator_max_lag_ns": 5,
        "host_max_lag_ns": 5,
        "artifact_sha256": {"target": {}, "generator": {}},
    }
    target_summary = encoded({"metrics": study_run("full")["metrics"]})
    summary.write_bytes(target_summary)
    bundle["artifact_sha256"]["target"]["summary"] = host_perf.sha256(target_summary)

    def verified_bundle(*_args: object) -> dict:
        for label, directory in (
            ("aa_bundle", aa),
            ("snapshot_bundle", snapshot),
            ("recorder_bundle", recorder),
        ):
            bundle["artifact_sha256"][label] = host_perf.sha256(
                (directory / f"{directory.name}-bundle.json").read_bytes()
            )
        return bundle

    monkeypatch.setattr(host_control_assess.host_controls, "verify_bundle", verified_bundle)
    for path, start in ((raw, 10), (generator, 30)):
        data = encoded({"sampling_started_epoch_ns": start, "sampling_ended_epoch_ns": start + 10})
        host_control_assess.host_controls.sidecars(path)["resources"].write_bytes(data)
        key = "target" if path == raw else "generator"
        bundle["artifact_sha256"][key]["resources"] = host_perf.sha256(data)
    aa_runs = [
        study_run("A1" if index % 2 == 0 else "A2", 2.0 if index % 2 == 0 else 2.1)
        for index in range(4)
    ]
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
        for index, run in enumerate(runs):
            start = first_start + index * 20
            data = encoded(
                {"sampling_started_epoch_ns": start, "sampling_ended_epoch_ns": start + 10}
            )
            host_control_assess.host_aa.paths(directory, index)["resources"].write_bytes(data)
            run["resources_sha256"] = host_perf.sha256(data)
            if directory == recorder:
                run["probe_process_duration_ns"] = 10
            retain_summary(directory, index, run)
        (directory / f"{directory.name}-bundle.json").write_bytes(encoded({"runs": runs}))
    sampler_runs = []
    for index, mode in enumerate(("on", "off", "off", "on")):
        row = study_run(mode, 2.1 if mode == "on" else 2.0)
        start = 300 + index * 20
        row["window_epoch_ns"] = [start, start + 10]
        row["boot_time_ns"] = 1
        row["resource_cadence_ms"] = 250
        retain_summary(sampler, index, row)
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
        "schema_version": 2,
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
        "max_recorder_process_duration_relative_delta": 0.1,
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


def test_control_assessment_rejects_artifacts_changed_after_verification(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    policy, paths = setup_assessment(tmp_path, monkeypatch)
    verified = copy.deepcopy(host_control_assess.host_controls.verify_bundle())
    monkeypatch.setattr(host_control_assess.host_controls, "verify_bundle", lambda *_args: verified)
    aa_path = paths[4] / "aa-bundle.json"
    original = aa_path.read_bytes()
    aa_path.write_bytes(original + b" ")
    with pytest.raises(host_perf.ReceiptError, match="A/A bundle changed"):
        host_control_assess.assess(encoded(policy), *paths)

    aa_path.write_bytes(original)
    resource_path = host_control_assess.host_controls.sidecars(paths[1])["resources"]
    resource_path.write_bytes(resource_path.read_bytes() + b" ")
    with pytest.raises(host_perf.ReceiptError, match="control resources changed"):
        host_control_assess.assess(encoded(policy), *paths)


def test_control_assessment_rejects_control_scenario_swap(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    policy, paths = setup_assessment(tmp_path, monkeypatch)
    verified = host_control_assess.host_controls.verify_bundle()
    verified["scenario_sha256"] = "b" * 64
    with pytest.raises(host_perf.ReceiptError, match="scenario differs"):
        host_control_assess.assess(encoded(policy), *paths)


@pytest.mark.parametrize("directory_index", [4, 5, 6, 7])
@pytest.mark.parametrize(
    ("field", "value"),
    [
        ("not_submitted", 1),
        ("unanswered_at_settlement", 1),
        ("max_producer_lag_ns", 11),
        ("late_snapshot_samples", 1),
    ],
)
def test_control_assessment_checks_every_control_arm_health(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    directory_index: int,
    field: str,
    value: int,
) -> None:
    policy, paths = setup_assessment(tmp_path, monkeypatch)
    directory = paths[directory_index]
    bundle_path = directory / f"{directory.name}-bundle.json"
    bundle = json.loads(bundle_path.read_bytes())
    run = bundle["runs"][0]
    metrics = run["metrics"]
    if field in ("not_submitted", "unanswered_at_settlement"):
        metrics["counts"][field] = value
    else:
        metrics[field] = value
    retain_summary(directory, 0, run)
    bundle_path.write_bytes(encoded(bundle))
    report = host_control_assess.assess(encoded(policy), *paths)
    assert report["status"] == "BUDGET_FAIL"
    label = {4: "A/A", 5: "Snapshot", 6: "recorder", 7: "sampler"}[directory_index]
    assert report["violations"] == [f"{label}/0/{field}"]


@pytest.mark.parametrize(
    ("label", "modes"),
    [("Snapshot", ("on", "off")), ("recorder", ("full", "minimal")), ("sampler", ("on", "off"))],
)
def test_control_budget_rejects_unbalanced_control_order(
    label: str, modes: tuple[str, str]
) -> None:
    forward = [{"mode": modes[0]}, {"mode": modes[1]}]
    reverse = list(reversed(forward))
    assert len(host_control_assess.balanced_pairs(forward + reverse, 2, label, modes)) == 2
    with pytest.raises(host_perf.ReceiptError, match="order is unbalanced"):
        host_control_assess.balanced_pairs(forward + reverse + forward, 2, label, modes)
    with pytest.raises(host_perf.ReceiptError, match="both control arms"):
        host_control_assess.balanced_pairs(forward + forward[:1] * 2, 2, label, modes)


@pytest.mark.parametrize(
    ("field", "value"),
    [("not_submitted", 1), ("unanswered_at_settlement", 1), ("max_producer_lag_ns", 11)],
)
def test_control_budget_checks_minimal_recorder_without_fabricating_latency(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, field: str, value: int
) -> None:
    policy, paths = setup_assessment(tmp_path, monkeypatch)
    bundle_path = paths[6] / "recorder-bundle.json"
    bundle = json.loads(bundle_path.read_bytes())
    run = bundle["runs"][1]
    assert run["summary_sha256"] is None
    if field == "max_producer_lag_ns":
        run["metrics"][field] = value
    else:
        run["metrics"]["counts"][field] = value
    bundle_path.write_bytes(encoded(bundle))
    report = host_control_assess.assess(encoded(policy), *paths)
    assert report["status"] == "BUDGET_FAIL"
    assert report["violations"] == [f"recorder/1/{field}"]


@pytest.mark.parametrize(
    ("field", "value"),
    [
        ("not_submitted", 1),
        ("unanswered_at_settlement", 1),
        ("max_producer_lag_ns", 11),
        ("late_snapshot_samples", 1),
    ],
)
def test_control_budget_checks_target_settlement_and_observer(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, field: str, value: int
) -> None:
    policy, paths = setup_assessment(tmp_path, monkeypatch)
    target = json.loads(paths[2].read_bytes())
    if field in ("not_submitted", "unanswered_at_settlement"):
        target["metrics"]["counts"][field] = value
    else:
        target["metrics"][field] = value
    paths[2].write_bytes(encoded(target))
    verified = host_control_assess.host_controls.verify_bundle()
    verified["artifact_sha256"]["target"]["summary"] = host_perf.sha256(paths[2].read_bytes())
    report = host_control_assess.assess(encoded(policy), *paths)
    assert report["status"] == "BUDGET_FAIL"
    assert report["violations"] == [f"target/0/{field}"]


def test_recorder_duration_budget_detects_cost_with_unchanged_response_counts(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    policy, paths = setup_assessment(tmp_path, monkeypatch)
    bundle_path = paths[6] / "recorder-bundle.json"
    bundle = json.loads(bundle_path.read_bytes())
    bundle["runs"][0]["probe_process_duration_ns"] = 20
    bundle_path.write_bytes(encoded(bundle))
    report = host_control_assess.assess(encoded(policy), *paths)
    assert report["status"] == "BUDGET_FAIL"
    assert report["observations"]["max_recorder_response_fraction_delta"] == 0.1
    assert report["violations"] == ["max_recorder_process_duration_relative_delta"]
