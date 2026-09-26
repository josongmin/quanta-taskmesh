"""A/A pairs bind actual host artifacts but do not set a timing threshold."""

from __future__ import annotations

import importlib
import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_aa  # noqa: E402
import host_perf  # noqa: E402

from tools.process_supervisor import SupervisedProcess  # noqa: E402

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


def control_args(module: object, tmp_path: Path) -> tuple[list, str, int]:
    scenario = tmp_path / "scenario.json"
    source = json.loads(SCENARIO.read_bytes())
    source["load"]["snapshot_ms"] = 0
    scenario.write_text(json.dumps(source))
    calibration = tmp_path / "calibration.json"
    calibration.write_text("{}")
    name = module.__name__
    extras = [10] if name == "host_observer" else [1_000_000_000] if name == "host_sampler" else []
    pairs = 2 if name == "host_sampler" else 1
    bundle = {
        "host_aa": "aa-bundle.json",
        "host_observer": "snapshot-bundle.json",
        "host_recorder": "recorder-bundle.json",
        "host_sampler": "sampler-bundle.json",
    }[name]
    return [scenario, calibration, tmp_path / "output", pairs, *extras], bundle, pairs * 2


@pytest.mark.parametrize("name", ["host_aa", "host_observer", "host_recorder", "host_sampler"])
@pytest.mark.parametrize("failure", ["timeout", "signal", "aborted", "exit", "spawn"])
def test_control_failures_retain_complete_planned_accounting_without_next_arm(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, name: str, failure: str
) -> None:
    module = importlib.import_module(name)
    args, bundle_name, expected = control_args(module, tmp_path)
    calls = []

    def failed(command: list[str], **kwargs: object) -> SupervisedProcess:
        calls.append((command, kwargs))
        if failure == "spawn":
            raise OSError("unavailable launcher")
        return SupervisedProcess(
            3 if failure == "exit" else 0,
            "retained partial stdout",
            "retained partial stderr",
            failure == "timeout",
            15 if failure == "signal" else None,
            failure == "aborted",
        )

    monkeypatch.setattr(host_aa, "run_acquisition", failed)
    with pytest.raises(host_perf.ReceiptError, match="acquisition incomplete"):
        module.acquire(*args, timeout_seconds=0.5)
    assert len(calls) == 1
    assert calls[0][1]["timeout_seconds"] == 0.5
    directory = args[2]
    bundle = json.loads((directory / bundle_name).read_bytes())
    assert bundle["expected_runs"] == expected
    assert len(bundle["executions"]) == len(bundle["failures"]) == expected
    assert bundle["executions"][0]["status"] == "failed"
    assert all(row["status"] == "not_launched" for row in bundle["executions"][1:])
    for index, reference in enumerate(bundle["executions"]):
        paths = host_aa.execution_paths(directory, index)
        data = paths["execution.json"].read_bytes()
        assert host_perf.sha256(data) == reference["execution_sha256"]
        terminal = json.loads(data)
        for capture in ("stdout", "stderr"):
            assert host_perf.sha256(paths[capture].read_bytes()) == terminal[f"{capture}_sha256"]
    terminal = json.loads(host_aa.execution_paths(directory, 0)["execution.json"].read_bytes())
    assert terminal["timed_out"] is (failure == "timeout")
    assert terminal["interrupted_by_signal"] == (15 if failure == "signal" else None)


@pytest.mark.parametrize("name", ["host_aa", "host_observer", "host_recorder", "host_sampler"])
@pytest.mark.parametrize("timeout", [0, -1, True, float("inf"), float("nan")])
def test_control_deadline_is_validated_before_output_creation(
    tmp_path: Path, name: str, timeout: object
) -> None:
    module = importlib.import_module(name)
    args, _, _ = control_args(module, tmp_path)
    with pytest.raises(host_perf.ReceiptError, match="positive and finite"):
        module.acquire(*args, timeout_seconds=timeout)
    assert not args[2].exists()


@pytest.mark.parametrize("name", ["host_aa", "host_observer", "host_recorder", "host_sampler"])
def test_control_success_executes_every_declared_arm_and_binds_captures(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, name: str
) -> None:
    module = importlib.import_module(name)
    args, _, expected = control_args(module, tmp_path)
    calls = []

    def successful(command: list[str], **kwargs: object) -> SupervisedProcess:
        calls.append((command, kwargs))
        return SupervisedProcess(0, "completed", "", False, None)

    def verified(_directory: Path, index: int, *_args: object) -> tuple[dict, dict]:
        row = {"index": index, "binary_sha256": "binary"}
        if name == "host_recorder":
            row["mode"] = _args[-1]
        return row, {"identity": "constant"}

    def retain(path: Path, bundle: dict, *_args: object) -> None:
        host_aa.write_new(path, host_perf.canonical(bundle))

    monkeypatch.setattr(host_aa, "run_acquisition", successful)
    monkeypatch.setattr(module, "retain_control_bundle", retain)
    monkeypatch.setattr(host_aa if name == "host_observer" else module, "verified_run", verified)
    bundle = module.acquire(*args, timeout_seconds=7)
    assert len(calls) == len(bundle["runs"]) == len(bundle["executions"]) == expected
    host_aa.validate_executions(bundle, args[2], bundle["runs"])
    host_aa.execution_paths(args[2], 0)["stdout"].write_text("changed")
    with pytest.raises(host_perf.ReceiptError, match="capture changed"):
        host_aa.validate_executions(bundle, args[2], bundle["runs"])


@pytest.mark.parametrize(
    "field,value",
    [
        ("returncode", False),
        ("timed_out", True),
        ("aborted_early", True),
        ("interrupted_by_signal", 15),
        ("timeout_seconds", True),
        ("index", True),
        ("schema_version", True),
        ("status", "not_launched"),
    ],
)
def test_execution_flags_cannot_be_hidden_by_rebinding_the_metadata_digest(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, field: str, value: object
) -> None:
    monkeypatch.setattr(
        host_aa, "run_acquisition", lambda *a, **k: SupervisedProcess(0, "out", "err", False, None)
    )
    command = [sys.executable, "host_run.py", "scenario", str(host_aa.paths(tmp_path, 0)["raw"])]
    reference, reason = host_aa.execute_arm(tmp_path, 0, command, 1)
    assert reason is None
    bundle = {"expected_runs": 1, "timeout_seconds": 1, "executions": [reference]}
    host_aa.validate_executions(bundle, tmp_path, [{}])
    path = host_aa.execution_paths(tmp_path, 0)["execution.json"]
    row = json.loads(path.read_bytes())
    row[field] = value
    data = host_perf.canonical(row)
    path.write_bytes(data)
    reference["execution_sha256"] = host_perf.sha256(data)
    with pytest.raises(host_perf.ReceiptError):
        host_aa.validate_executions(bundle, tmp_path, [{}])


@pytest.mark.parametrize("name", ["host_aa", "host_observer", "host_recorder", "host_sampler"])
def test_control_cli_forwards_declared_timeout(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, name: str
) -> None:
    module = importlib.import_module(name)
    calls = []

    def acquire(*args: object, **kwargs: object) -> dict:
        calls.append(kwargs)
        return {"runs": []}

    monkeypatch.setattr(module, "acquire", acquire)
    options = (
        ["--snapshot-ms", "10"]
        if name == "host_observer"
        else (["--max-span-seconds", "1"] if name == "host_sampler" else [])
    )
    monkeypatch.setattr(
        sys,
        "argv",
        [name, "scenario", "calibration", str(tmp_path), *options, "--timeout-seconds", "2.5"],
    )
    assert module.main() == 0
    assert calls == [{"timeout_seconds": 2.5}]
