"""Separate host-mode receipts remain structural and reject tampered rows."""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_perf  # noqa: E402
import host_special_run  # noqa: E402

from tools.process_supervisor import SupervisedProcess  # noqa: E402


def receipt_directory(tmp_path: Path) -> Path:
    identity = {"features": []}
    artifacts = {
        "scenario": b"{}",
        "raw": b'{"checksum": 4}',
        "topology": b"{}",
        "runner": b"runner",
        "validator": b"validator",
        "resources": b"{}",
    }
    for name, data in artifacts.items():
        (tmp_path / name).write_bytes(data)
    receipt = {
        "schema_version": host_special_run.SPECIAL_RECEIPT_VERSION,
        "status": "complete",
        "reason": None,
        "performance_status": "UNQUALIFIED",
        "mode": "composite",
        **{f"{name}_sha256": host_perf.sha256(data) for name, data in artifacts.items()},
        "runner_pid": 123,
        "runner_exit_code": 0,
        "validator_exit_code": 0,
        "build_commands": [
            [
                "cargo",
                "build",
                "--locked",
                "-p",
                "taskmesh-bench",
                "--example",
                example,
                "--message-format=json",
            ]
            for example in ("host_composite_probe", "host_special_validate")
        ],
        "features": [],
        "build_artifact_features": ["default"],
        "start_identity": identity,
        "end_identity": identity,
        "source_content_sha256": "a" * 64,
    }
    (tmp_path / "receipt.json").write_text(json.dumps(receipt))
    return tmp_path


def test_special_receipt_checks_digest_then_retained_typed_validator(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    directory = receipt_directory(tmp_path)
    monkeypatch.setattr(host_perf, "validate_identity", lambda _identity: None)
    monkeypatch.setattr(host_perf, "validate_resource_artifact", lambda _data, _pid: None)
    calls = []

    def checked(command: list[str], **_kwargs: object) -> SupervisedProcess:
        calls.append(command)
        return SupervisedProcess(0, "valid", "", False, None)

    monkeypatch.setattr(host_special_run, "run_acquisition", checked)
    assert host_special_run.verify_receipt(directory)["performance_status"] == "UNQUALIFIED"
    assert len(calls) == 1

    (directory / "raw").write_bytes(b'{"checksum": 5}')
    with pytest.raises(host_perf.ReceiptError, match="raw digest differs"):
        host_special_run.verify_receipt(directory)
    assert len(calls) == 1

    receipt = json.loads((directory / "receipt.json").read_text())
    receipt["raw_sha256"] = host_perf.sha256((directory / "raw").read_bytes())
    (directory / "receipt.json").write_text(json.dumps(receipt))
    monkeypatch.setattr(
        host_special_run,
        "run_acquisition",
        lambda command, **_kwargs: SupervisedProcess(
            1, "", "caller-owned keyed reduction differs", False, None
        ),
    )
    with pytest.raises(host_perf.ReceiptError, match="typed special raw/topology rejected"):
        host_special_run.verify_receipt(directory)


@pytest.mark.parametrize(
    ("field", "value", "error"),
    [
        ("schema_version", True, "version"),
        ("source_content_sha256", None, "source content digest"),
        ("build_artifact_features", "default", "build artifact features"),
        ("runner_exit_code", False, "runner or validator exit"),
    ],
)
def test_special_receipt_rejects_malformed_identity_fields(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    field: str,
    value: object,
    error: str,
) -> None:
    directory = receipt_directory(tmp_path)
    receipt = json.loads((directory / "receipt.json").read_text())
    receipt[field] = value
    (directory / "receipt.json").write_text(json.dumps(receipt))
    monkeypatch.setattr(host_perf, "validate_identity", lambda _identity: None)
    monkeypatch.setattr(host_perf, "validate_resource_artifact", lambda _data, _pid: None)
    monkeypatch.setattr(
        host_special_run,
        "run_acquisition",
        lambda command, **_kwargs: SupervisedProcess(0, "valid", "", False, None),
    )
    with pytest.raises(host_perf.ReceiptError, match=error):
        host_special_run.verify_receipt(directory)


def test_v1_special_receipt_requires_reacquisition(tmp_path: Path) -> None:
    directory = receipt_directory(tmp_path)
    receipt = json.loads((directory / "receipt.json").read_bytes())
    receipt["schema_version"] = 1
    (directory / "receipt.json").write_text(json.dumps(receipt))
    with pytest.raises(host_perf.ReceiptError, match="reacquire execution evidence"):
        host_special_run.verify_receipt(directory)


def test_special_validator_uses_only_digest_checked_snapshot(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    directory = receipt_directory(tmp_path)
    monkeypatch.setattr(host_perf, "validate_identity", lambda _identity: None)
    monkeypatch.setattr(host_perf, "validate_resource_artifact", lambda _data, _pid: None)

    def checked(command: list[str], **kwargs: object) -> SupervisedProcess:
        (directory / "raw").write_bytes(b"changed after digest check")
        (directory / "validator").write_bytes(b"changed executable")
        assert Path(command[0]).read_bytes() == b"validator"
        assert Path(command[3]).read_bytes() == b'{"checksum": 4}'
        assert Path(command[0]).parent != directory
        assert kwargs["timeout_seconds"] == 7.25
        return SupervisedProcess(0, "valid", "", False, None)

    monkeypatch.setattr(host_special_run, "run_acquisition", checked)
    assert (
        host_special_run.verify_receipt(directory, timeout_seconds=7.25)["performance_status"]
        == "UNQUALIFIED"
    )
    with pytest.raises(host_perf.ReceiptError, match="raw digest differs"):
        host_special_run.verify_receipt(directory)


@pytest.mark.parametrize(
    "returncode,timed_out,signum,aborted",
    [
        (0, True, None, False),
        (0, False, 15, False),
        (None, False, None, False),
        (0, False, None, True),
    ],
)
def test_special_validator_timeout_or_interrupt_cannot_complete(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    returncode: object,
    timed_out: bool,
    signum: object,
    aborted: bool,
) -> None:
    directory = receipt_directory(tmp_path)
    monkeypatch.setattr(host_perf, "validate_identity", lambda _identity: None)
    monkeypatch.setattr(host_perf, "validate_resource_artifact", lambda _data, _pid: None)
    monkeypatch.setattr(
        host_special_run,
        "run_acquisition",
        lambda *a, **k: SupervisedProcess(
            returncode, "partial", "incomplete execution", timed_out, signum, aborted
        ),
    )
    with pytest.raises(host_perf.ReceiptError, match="typed special raw/topology rejected"):
        host_special_run.verify_receipt(directory)


def test_special_snapshot_executes_a_real_retained_validator(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    directory = receipt_directory(tmp_path)
    monkeypatch.setattr(host_perf, "validate_identity", lambda _identity: None)
    monkeypatch.setattr(host_perf, "validate_resource_artifact", lambda _data, _pid: None)
    validator = b"""#!/usr/bin/env python3
import pathlib
import sys
assert sys.argv[1] == "composite"
assert pathlib.Path(sys.argv[2]).read_bytes() == b"{}"
assert pathlib.Path(sys.argv[3]).read_bytes() == b'{"checksum": 4}'
assert pathlib.Path(sys.argv[4]).read_bytes() == b"{}"
"""
    (directory / "validator").write_bytes(validator)
    receipt = json.loads((directory / "receipt.json").read_bytes())
    receipt["validator_sha256"] = host_perf.sha256(validator)
    (directory / "receipt.json").write_text(json.dumps(receipt))
    assert host_special_run.verify_receipt(directory)["status"] == "complete"


@pytest.mark.parametrize(
    "artifact", ["receipt.json", "scenario", "raw", "topology", "validator", "resources"]
)
def test_special_receipt_nonregular_ingress_rejects_before_launch(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path, artifact: str
) -> None:
    directory = receipt_directory(tmp_path)
    (directory / artifact).unlink()
    os.mkfifo(directory / artifact)
    monkeypatch.setattr(host_perf, "validate_identity", lambda _identity: None)
    monkeypatch.setattr(
        host_special_run, "run_acquisition", lambda *args, **kwargs: pytest.fail("must not launch")
    )
    with pytest.raises(OSError, match="regular file required"):
        host_special_run.verify_receipt(directory)


def test_special_scenario_fifo_rejects_before_build(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    scenario = tmp_path / "scenario"
    os.mkfifo(scenario)
    monkeypatch.setattr(
        host_special_run.host_run,
        "build_runner",
        lambda *args, **kwargs: pytest.fail("must not build"),
    )
    with pytest.raises(OSError, match="regular file required"):
        host_special_run.acquire("composite", scenario, tmp_path / "output", [])
    assert not (tmp_path / "output").exists()
