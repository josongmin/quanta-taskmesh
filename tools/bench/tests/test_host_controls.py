"""Cross-control binding is distinct from each constituent's typed raw verifier."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_controls  # noqa: E402
import host_perf  # noqa: E402
from scenario_rate import doubled_generator_scenario  # noqa: E402

SCENARIO = host_perf.REPO / "tools/bench/scenarios/h1-default-send-smoke.json"


def encoded(value: dict) -> bytes:
    return host_perf.canonical(value) + b"\n"


def resource(pid: int, start: int, end: int, cadence: int = 250) -> bytes:
    return encoded(
        {
            "schema_version": 3,
            "execution": {
                "timeout_seconds": 1800,
                "returncode": 0,
                "timed_out": False,
                "interrupted_by_signal": None,
                "aborted_early": False,
            },
            "status": "complete",
            "reason": None,
            "pid": pid,
            "process_create_time_ns": pid,
            "boot_time_ns": 1,
            "cadence_ms": cadence,
            "sampling_started_monotonic_ns": start,
            "sampling_ended_monotonic_ns": end,
            "sampling_started_epoch_ns": start,
            "sampling_ended_epoch_ns": end,
            "samples": [
                {
                    "monotonic_ns": start + 1,
                    "cpu_user_ns": 1,
                    "cpu_system_ns": 1,
                    "rss_bytes": 4096,
                    "threads": 1,
                }
            ],
        }
    )


def setup_control_set(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> tuple:
    scenario = tmp_path / "scenario.json"
    scenario.write_bytes(SCENARIO.read_bytes())
    host_raw = tmp_path / "target.json"
    host_summary = tmp_path / "target.summary.json"
    generator_raw = tmp_path / "generator.json"
    host_raw.write_bytes(b"{}")
    host_summary.write_bytes(b"{}")
    generator_raw.write_bytes(encoded({"not_submitted": 0, "records": [{"scheduled_lag_ns": 4}]}))
    doubled = doubled_generator_scenario(json.loads(scenario.read_bytes()))
    generator_raw.with_name(generator_raw.name + ".scenario.json").write_bytes(encoded(doubled))
    identity = {"source": "same", "host": "same", "features": []}
    for raw, binary, pid, start, mode in (
        (host_raw, b"host-binary", 1, 10, "full"),
        (generator_raw, b"generator-binary", 2, 30, "generator"),
    ):
        paths = host_controls.sidecars(raw)
        paths["provenance"].write_bytes(encoded({"runner_pid": pid, "runner_mode": mode}))
        paths["binary"].write_bytes(binary)
        paths["topology"].write_bytes(b"one-topology")
        paths["resources"].write_bytes(resource(pid, start, start + 10))
    aa_dir = tmp_path / "aa"
    snapshot_dir = tmp_path / "snapshot"
    recorder_dir = tmp_path / "recorder"
    for directory in (aa_dir, snapshot_dir, recorder_dir):
        directory.mkdir()
    binary_digest = host_perf.sha256(b"host-binary")
    common = {"identity": identity, "binary_sha256": binary_digest, "runs": [{}, {}]}
    scenario_digest = host_perf.sha256(scenario.read_bytes())
    (aa_dir / "aa-bundle.json").write_bytes(encoded({**common, "scenario_sha256": scenario_digest}))
    off, on = host_controls.host_observer.scenario_pair(scenario.read_bytes(), 10)
    (snapshot_dir / "snapshot-bundle.json").write_bytes(
        encoded(
            {
                **common,
                "snapshot_ms": 10,
                "off_scenario_sha256": host_perf.sha256(off),
                "on_scenario_sha256": host_perf.sha256(on),
            }
        )
    )
    (recorder_dir / "recorder-bundle.json").write_bytes(
        encoded({**common, "scenario_sha256": scenario_digest})
    )
    for directory, first_start, first_pid in (
        (aa_dir, 50, 3),
        (snapshot_dir, 90, 5),
        (recorder_dir, 130, 7),
    ):
        for index in range(2):
            paths = host_controls.host_aa.paths(directory, index)
            pid = first_pid + index
            start = first_start + index * 20
            paths["provenance"].write_bytes(encoded({"runner_pid": pid}))
            paths["resources"].write_bytes(resource(pid, start, start + 10))
    monkeypatch.setattr(
        host_controls.host_perf,
        "verify_receipt",
        lambda *args, **kwargs: {
            "identity": identity,
            "metrics": {"counts": {"not_submitted": 0}, "max_producer_lag_ns": 5},
        },
    )
    monkeypatch.setattr(host_controls.host_perf, "validate_with_rust", lambda *args, **kwargs: None)
    monkeypatch.setattr(
        host_controls.host_perf, "validate_execution_provenance", lambda *args, **kwargs: None
    )
    for module in (host_controls.host_aa, host_controls.host_observer, host_controls.host_recorder):
        monkeypatch.setattr(
            module,
            "verify_bundle",
            lambda bundle_bytes, *args: host_perf.parse_object(bundle_bytes, "fixture bundle"),
        )
    return scenario, host_raw, host_summary, generator_raw, aa_dir, snapshot_dir, recorder_dir


def test_control_bundle_binds_roles_identity_cadence_and_time(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    paths = setup_control_set(tmp_path, monkeypatch)
    bundle = host_controls.make_bundle(*paths, max_span_ns=200)
    assert bundle["status"] == "diagnostic_complete"
    assert bundle["performance"] == "UNQUALIFIED"
    assert bundle["process_count"] == 8
    assert bundle["observed_span_ns"] == 150
    assert bundle["generator_max_lag_ns"] == 4
    assert bundle["generator_rate_factor"] == 2
    assert bundle["host_max_lag_ns"] == 5
    host_controls.verify_bundle(encoded(bundle), *paths)
    forged = dict(bundle)
    forged["generator_not_submitted"] = 1
    with pytest.raises(host_perf.ReceiptError, match="differs from revalidated"):
        host_controls.verify_bundle(encoded(forged), *paths)

    aa_path = paths[4] / "aa-bundle.json"
    aa = json.loads(aa_path.read_bytes())
    aa["identity"] = {"source": "other", "host": "same", "features": []}
    aa_path.write_bytes(encoded(aa))
    with pytest.raises(host_perf.ReceiptError, match="source, host or binary differs"):
        host_controls.make_bundle(*paths, max_span_ns=200)


def test_control_bundle_rejects_overlap_stale_and_sampler_change(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    paths = setup_control_set(tmp_path, monkeypatch)
    with pytest.raises(host_perf.ReceiptError, match="declared span"):
        host_controls.make_bundle(*paths, max_span_ns=100)
    resource_path = host_controls.host_aa.paths(paths[5], 0)["resources"]
    resource_path.write_bytes(resource(5, 55, 65))
    with pytest.raises(host_perf.ReceiptError, match="windows overlap"):
        host_controls.make_bundle(*paths, max_span_ns=200)
    resource_path.write_bytes(resource(5, 90, 100, cadence=100))
    with pytest.raises(host_perf.ReceiptError, match="sampling cadence differs"):
        host_controls.make_bundle(*paths, max_span_ns=200)
    changed_boot = json.loads(resource(5, 90, 100))
    changed_boot["boot_time_ns"] = 2
    resource_path.write_bytes(encoded(changed_boot))
    with pytest.raises(host_perf.ReceiptError, match="different host boots"):
        host_controls.make_bundle(*paths, max_span_ns=200)


def test_control_bundle_rejects_same_rate_or_altered_generator_schedule(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    paths = setup_control_set(tmp_path, monkeypatch)
    generator_scenario = paths[3].with_name(paths[3].name + ".scenario.json")
    expected = generator_scenario.read_bytes()
    generator_scenario.write_bytes(paths[0].read_bytes())
    with pytest.raises(host_perf.ReceiptError, match="exact 2x replay"):
        host_controls.make_bundle(*paths, max_span_ns=200)
    altered = json.loads(expected)
    altered["offers"][1]["send_time_ns"] += 2
    generator_scenario.write_bytes(encoded(altered))
    with pytest.raises(host_perf.ReceiptError, match="exact 2x replay"):
        host_controls.make_bundle(*paths, max_span_ns=200)


def test_control_bundle_accepts_sampled_target_with_off_recorder_arm(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    paths = setup_control_set(tmp_path, monkeypatch)
    scenario = json.loads(paths[0].read_bytes())
    scenario["load"]["snapshot_ms"] = 10
    paths[0].write_bytes(encoded(scenario))
    generator_scenario = paths[3].with_name(paths[3].name + ".scenario.json")
    generator_scenario.write_bytes(encoded(doubled_generator_scenario(scenario)))
    aa_path = paths[4] / "aa-bundle.json"
    aa = json.loads(aa_path.read_bytes())
    aa["scenario_sha256"] = host_perf.sha256(paths[0].read_bytes())
    aa_path.write_bytes(encoded(aa))
    off, on = host_controls.host_observer.scenario_pair(paths[0].read_bytes(), 10)
    snapshot_path = paths[5] / "snapshot-bundle.json"
    snapshot = json.loads(snapshot_path.read_bytes())
    snapshot["off_scenario_sha256"] = host_perf.sha256(off)
    snapshot["on_scenario_sha256"] = host_perf.sha256(on)
    snapshot_path.write_bytes(encoded(snapshot))
    recorder_path = paths[6] / "recorder-bundle.json"
    recorder = json.loads(recorder_path.read_bytes())
    recorder["scenario_sha256"] = host_perf.sha256(off)
    recorder_path.write_bytes(encoded(recorder))
    bundle = host_controls.make_bundle(*paths, max_span_ns=200)
    assert bundle["target_snapshot_ms"] == 10
    assert bundle["recorder_scenario_sha256"] == host_perf.sha256(off)
    host_controls.verify_bundle(encoded(bundle), *paths)
    scenario["load"]["snapshot_ms"] = 5
    paths[0].write_bytes(encoded(scenario))
    generator_scenario.write_bytes(encoded(doubled_generator_scenario(scenario)))
    with pytest.raises(host_perf.ReceiptError, match="target Snapshot cadence differs"):
        host_controls.make_bundle(*paths, max_span_ns=200)
