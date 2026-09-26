"""Sampler on/off receipts remain source-bound diagnostics."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_perf  # noqa: E402
import host_sampler  # noqa: E402


def encoded(value: dict) -> bytes:
    return host_perf.canonical(value) + b"\n"


def resource(pid: int, start: int, mode: str) -> bytes:
    sampled = mode == "on"
    return encoded(
        {
            "schema_version": 2,
            "status": "complete" if sampled else "unavailable",
            "reason": None if sampled else host_sampler.OFF_REASON,
            "pid": pid,
            "process_create_time_ns": start if sampled else None,
            "boot_time_ns": 1,
            "cadence_ms": 250,
            "sampling_started_monotonic_ns": start,
            "sampling_ended_monotonic_ns": start + 10,
            "sampling_started_epoch_ns": start,
            "sampling_ended_epoch_ns": start + 10,
            "samples": [
                {
                    "monotonic_ns": start + 1,
                    "cpu_user_ns": 1,
                    "cpu_system_ns": 1,
                    "rss_bytes": 4096,
                    "threads": 1,
                }
            ]
            if sampled
            else [],
        }
    )


def setup_study(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> tuple[dict, bytes]:
    identity = {"source_head": "clean", "features": []}
    scenario = b"scenario"
    monkeypatch.setattr(host_perf, "validate_identity", lambda _identity: None)

    def verified(_directory: Path, index: int, _scenario: bytes) -> tuple[dict, dict]:
        row = {key: f"{key}-{index}" for key in host_sampler.host_aa.RUN_KEYS}
        row["index"] = index
        row["pair"] = index // 2
        row["member"] = "A1" if index % 2 == 0 else "A2"
        row["binary_sha256"] = "one-binary"
        row["metrics"] = {"cohort_goodput": {"c/io": {"slo_per_sec": 2.0}}}
        return row, identity

    monkeypatch.setattr(host_sampler.host_aa, "verified_run", verified)
    for index in range(4):
        paths = host_sampler.host_aa.paths(tmp_path, index)
        paths["provenance"].write_bytes(encoded({"runner_pid": index + 1}))
        paths["resources"].write_bytes(
            resource(index + 1, 10 + index * 20, host_sampler.mode_for(index))
        )
    bundle = {
        "schema_version": 1,
        "kind": "resource_sampler_on_off",
        "status": "diagnostic_complete",
        "performance": "UNQUALIFIED",
        "scenario_sha256": host_perf.sha256(scenario),
        "binary_sha256": "one-binary",
        "identity": identity,
        "max_span_ns": 100,
        "runs": [host_sampler.verified_run(tmp_path, index, scenario)[0] for index in range(4)],
        "failures": [],
    }
    return bundle, scenario


def test_sampler_bundle_binds_balanced_modes_and_resource_state(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    bundle, scenario = setup_study(tmp_path, monkeypatch)
    assert [row["mode"] for row in bundle["runs"]] == ["on", "off", "off", "on"]
    assert host_sampler.verify_bundle(encoded(bundle), tmp_path, scenario) == bundle

    forged = json.loads(encoded(bundle))
    forged["runs"][1]["mode"] = "on"
    with pytest.raises(host_perf.ReceiptError, match="artifact differs"):
        host_sampler.verify_bundle(encoded(forged), tmp_path, scenario)

    off_resource = host_sampler.host_aa.paths(tmp_path, 1)["resources"]
    off_resource.write_bytes(resource(2, 30, "on"))
    with pytest.raises(host_perf.ReceiptError, match="sampler-off arm"):
        host_sampler.verify_bundle(encoded(bundle), tmp_path, scenario)


def test_sampler_bundle_rejects_overlap_and_stale_span(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    bundle, scenario = setup_study(tmp_path, monkeypatch)
    bundle["max_span_ns"] = 20
    with pytest.raises(host_perf.ReceiptError, match="declared span"):
        host_sampler.verify_bundle(encoded(bundle), tmp_path, scenario)

    bundle["max_span_ns"] = 100
    second = host_sampler.host_aa.paths(tmp_path, 1)["resources"]
    second.write_bytes(resource(2, 15, "off"))
    bundle["runs"][1] = host_sampler.verified_run(tmp_path, 1, scenario)[0]
    with pytest.raises(host_perf.ReceiptError, match="windows overlap"):
        host_sampler.verify_bundle(encoded(bundle), tmp_path, scenario)


def test_sampler_rejects_unbalanced_pair_count_before_acquisition(tmp_path: Path) -> None:
    with pytest.raises(host_perf.ReceiptError, match="pairs must be even"):
        host_sampler.acquire(tmp_path / "missing", tmp_path / "missing", tmp_path / "out", 3, 1)
    assert not (tmp_path / "out").exists()


def test_sampler_rejects_indexed_order_that_differs_from_execution_time(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    bundle, scenario = setup_study(tmp_path, monkeypatch)
    for index, start in enumerate((10, 50, 70, 30)):
        host_sampler.host_aa.paths(tmp_path, index)["resources"].write_bytes(
            resource(index + 1, start, host_sampler.mode_for(index))
        )
    bundle["runs"] = [host_sampler.verified_run(tmp_path, index, scenario)[0] for index in range(4)]
    with pytest.raises(host_perf.ReceiptError, match="index order reversed"):
        host_sampler.verify_bundle(encoded(bundle), tmp_path, scenario)


@pytest.mark.parametrize("label", ["A/A", "Snapshot", "recorder"])
def test_shared_study_window_verifier_rejects_permuted_time_and_artifact_drift(
    tmp_path: Path, label: str
) -> None:
    runs = []
    for index in range(4):
        artifacts = {
            "provenance": encoded({"runner_pid": index + 1}),
            "resources": resource(index + 1, 10 + index * 20, "on"),
        }
        row = {}
        for name, data in artifacts.items():
            host_sampler.host_aa.paths(tmp_path, index)[name].write_bytes(data)
            row[f"{name}_sha256"] = host_perf.sha256(data)
        runs.append(row)
    host_sampler.host_aa.validate_study_windows(tmp_path, runs, label)
    second = host_sampler.host_aa.paths(tmp_path, 1)["resources"]
    second.write_bytes(resource(2, 70, "on"))
    with pytest.raises(host_perf.ReceiptError, match="changed during verification"):
        host_sampler.host_aa.validate_study_windows(tmp_path, runs, label)
    runs[1]["resources_sha256"] = host_perf.sha256(second.read_bytes())
    with pytest.raises(host_perf.ReceiptError, match="indexed order is reversed"):
        host_sampler.host_aa.validate_study_windows(tmp_path, runs, label)
