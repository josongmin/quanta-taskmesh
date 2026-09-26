"""Artifact publication must preserve another writer's files under races."""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path
from typing import BinaryIO

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_perf  # noqa: E402
import host_run  # noqa: E402


@pytest.mark.parametrize("executable", [False, True])
def test_publication_race_never_overwrites_or_deletes_other_writer(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, executable: bool
) -> None:
    source = tmp_path / "built-runner"
    source.write_bytes(b"my executable")
    source.chmod(0o755)
    destination = tmp_path / "artifact"
    original_link = os.link

    def racing_link(temporary: Path, target: Path) -> None:
        assert temporary.read_bytes() == b"my executable"
        target.write_bytes(b"other writer")
        original_link(temporary, target)

    monkeypatch.setattr(host_run.os, "link", racing_link)
    with pytest.raises(host_perf.ReceiptError, match="refusing to overwrite"):
        if executable:
            host_run.retain_executable(source, destination)
        else:
            host_run.write_new(destination, source.read_bytes())
    assert destination.read_bytes() == b"other writer"
    assert list(tmp_path.glob("artifact.tmp-*")) == []


def test_executable_copy_failure_keeps_other_writer_and_removes_only_private_partial(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    source = tmp_path / "source"
    source.write_bytes(b"runner")
    destination = tmp_path / "artifact"

    def failing_copy(_input: BinaryIO, output: BinaryIO) -> None:
        output.write(b"partial")
        destination.write_bytes(b"other writer")
        raise OSError("injected copy failure")

    monkeypatch.setattr(host_run.shutil, "copyfileobj", failing_copy)
    with pytest.raises(OSError, match="copy failure"):
        host_run.retain_executable(source, destination)
    assert destination.read_bytes() == b"other writer"
    assert list(tmp_path.glob("artifact.tmp-*")) == []


def test_artifact_publication_preserves_complete_bytes_and_executable_mode(tmp_path: Path) -> None:
    destination = tmp_path / "receipt"
    host_run.write_new(destination, b"complete receipt")
    assert destination.read_bytes() == b"complete receipt"
    with pytest.raises(host_perf.ReceiptError, match="refusing to overwrite"):
        host_run.write_new(destination, b"replacement")
    source = tmp_path / "source"
    source.write_bytes(b"runner")
    source.chmod(0o755)
    binary = tmp_path / "runner"
    assert host_run.retain_executable(source, binary) == b"runner"
    assert binary.read_bytes() == b"runner"
    assert binary.stat().st_mode & 0o777 == 0o755
    assert list(tmp_path.glob("*.tmp-*")) == []


@pytest.mark.parametrize("error_type", [OSError, host_perf.ReceiptError])
def test_final_control_validation_failure_is_retained_as_incomplete(
    tmp_path: Path, error_type: type[Exception]
) -> None:
    path = tmp_path / "bundle.json"
    bundle = {"status": "diagnostic_complete", "runs": [{}, {}], "failures": []}

    def reject(data: bytes) -> None:
        assert json.loads(data)["status"] == "diagnostic_complete"
        assert not path.exists()
        raise error_type("injected final verification failure")

    with pytest.raises(host_perf.ReceiptError, match="acquisition incomplete"):
        host_run.retain_control_bundle(path, bundle, reject, "control")
    retained = json.loads(path.read_bytes())
    assert retained["status"] == "incomplete"
    assert retained["runs"] == [{}, {}]
    assert retained["failures"][0]["phase"] == "bundle_validation"
    assert retained["failures"][0]["reason"] == "injected final verification failure"


def test_control_bundle_never_verifies_an_already_failed_acquisition(tmp_path: Path) -> None:
    path = tmp_path / "bundle.json"
    bundle = {
        "status": "incomplete",
        "runs": [],
        "failures": [{"index": 0, "reason": "runner failed"}],
    }

    def unexpected_verify(_data: bytes) -> None:
        pytest.fail("a failed acquisition must not reach success verification")

    with pytest.raises(host_perf.ReceiptError, match="runner failed"):
        host_run.retain_control_bundle(path, bundle, unexpected_verify, "control")
    assert json.loads(path.read_bytes()) == bundle
