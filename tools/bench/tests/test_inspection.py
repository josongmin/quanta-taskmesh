"""Metadata/process custody and regular descriptor ingress; no performance claims."""

from __future__ import annotations

import io
import os
import sys
import tarfile
import time
from pathlib import Path

import psutil
import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import allocation_gate  # noqa: E402
import bench_process  # noqa: E402
import host_build  # noqa: E402
import host_perf  # noqa: E402
import host_run  # noqa: E402
import host_study  # noqa: E402
import iai_gate  # noqa: E402

from tools import inspection  # noqa: E402
from tools.process_supervisor import (  # noqa: E402
    SupervisedBinaryProcess,
    SupervisedCommand,
    SupervisedProcess,
    run_process,
    run_process_batch,
)


def test_binary_capture_preserves_invalid_utf8_nul_and_newlines(tmp_path: Path) -> None:
    payload = b"\xff\x00\r\n\xc3\xa9\n"
    result = inspection.inspect_process(
        [sys.executable, "-c", f"import os; os.write(1,{payload!r}); os.write(2,{payload!r})"],
        cwd=tmp_path,
    )
    assert result.returncode == 0 and result.stdout == payload and result.stderr == payload


def test_git_archive_and_nul_paths_remain_byte_exact(tmp_path: Path) -> None:
    def git(*args: str) -> bytes:
        return inspection.metadata_output(["git", *args], cwd=tmp_path)

    git("init", "-q")
    name, payload = "한글\nfile", b"\xff\x00\r\n"
    (tmp_path / name).write_bytes(payload)
    git("add", "--", name)
    git("-c", "user.name=test", "-c", "user.email=test@example.com", "commit", "-qm", "fixture")
    assert git("ls-files", "-z") == os.fsencode(name) + b"\x00"
    with tarfile.open(fileobj=io.BytesIO(git("archive", "HEAD"))) as archive:
        stream = archive.extractfile(name)
        assert stream is not None and stream.read() == payload


@pytest.mark.parametrize("pipes", [False, True])
def test_incomplete_metadata_never_promotes_partial_output_and_reaps_group(
    tmp_path: Path, pipes: bool
) -> None:
    pidfile = tmp_path / "pid"
    script = (
        "import subprocess,sys,pathlib,time,os; "
        "child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)']"
        + ("); " if pipes else ",stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); ")
        + f"pathlib.Path({str(pidfile)!r}).write_text(str(child.pid)); "
        "os.write(1,b'partial\\xff'); sys.exit(0)"
    )
    started = time.monotonic()
    try:
        with pytest.raises(inspection.InspectionExecutionError):
            inspection.metadata_output(
                [sys.executable, "-c", script], cwd=tmp_path, timeout_seconds=0.4
            )
        assert time.monotonic() - started < 4
        child = psutil.Process(int(pidfile.read_text()))
        deadline = time.monotonic() + 2
        while child.is_running() and child.status() != psutil.STATUS_ZOMBIE:
            if time.monotonic() >= deadline:
                pytest.fail("owned metadata descendant survived")
            time.sleep(0.01)
    except psutil.NoSuchProcess:
        pass
    finally:
        if pidfile.exists():
            try:
                psutil.Process(int(pidfile.read_text())).kill()
            except psutil.NoSuchProcess:
                pass


def test_metadata_optional_query_does_not_hide_incomplete_execution(monkeypatch) -> None:
    def incomplete(*args, **kwargs):
        raise inspection.InspectionExecutionError("timeout")

    monkeypatch.setattr(host_perf, "metadata_output", incomplete)
    with pytest.raises(inspection.InspectionExecutionError):
        host_perf._command_output("anything")


@pytest.mark.parametrize("kind", ["fifo", "fifo_symlink", "directory", "device", "missing"])
def test_nonregular_artifact_rejects_without_waiting(tmp_path: Path, kind: str) -> None:
    path = tmp_path / "input"
    if kind.startswith("fifo"):
        os.mkfifo(path)
        if kind == "fifo_symlink":
            link = tmp_path / "link"
            link.symlink_to(path)
            path = link
    elif kind == "directory":
        path.mkdir()
    elif kind == "device":
        path = Path(os.devnull)
    started = time.monotonic()
    with pytest.raises(OSError):
        inspection.read_regular_bytes(path)
    assert time.monotonic() - started < 1


def test_opened_descriptor_survives_path_replacement(tmp_path: Path, monkeypatch) -> None:
    path = tmp_path / "input"
    path.write_bytes(b"original")
    path.chmod(0o751)
    original_open = os.open

    def replaced(*args, **kwargs):
        descriptor = original_open(*args, **kwargs)
        path.unlink()
        os.mkfifo(path)
        return descriptor

    monkeypatch.setattr(inspection.os, "open", replaced)
    assert inspection.read_regular_file(path) == (b"original", 0o751)


@pytest.mark.parametrize("owner", ["tool_context", "build_config_digest"])
@pytest.mark.parametrize("kind", ["fifo", "directory", "dangling_symlink"])
def test_present_nonregular_cargo_config_is_not_silently_skipped(
    tmp_path: Path, monkeypatch, owner: str, kind: str
) -> None:
    root = tmp_path / "source"
    config = root / ".cargo" / "config"
    config.parent.mkdir(parents=True)
    if kind == "fifo":
        os.mkfifo(config)
    elif kind == "directory":
        config.mkdir()
    else:
        config.symlink_to(tmp_path / "missing")
    monkeypatch.setenv("CARGO_HOME", str(tmp_path / "home"))
    with pytest.raises(OSError):
        if owner == "tool_context":
            iai_gate.cargo_tool_context(
                root, environment_source={"CARGO_HOME": str(tmp_path / "home"), "PATH": ""}
            )
        else:
            host_build.cargo_config_sha256(root)


def test_regular_symlink_and_executable_copy_preserve_bytes_and_mode(tmp_path: Path) -> None:
    source = tmp_path / "source"
    source.write_bytes(b"\xff\x00\r\n")
    source.chmod(0o751)
    link = tmp_path / "link"
    link.symlink_to(source)
    target = tmp_path / "target"
    inspection.copy_regular_file(link, target)
    assert inspection.read_regular_file(target) == (source.read_bytes(), 0o751)
    assert host_run.retain_executable(link, tmp_path / "retained") == source.read_bytes()
    with pytest.raises(FileExistsError):
        inspection.copy_regular_file(source, target)


def test_text_reader_preserves_encoding_errors_contract(tmp_path: Path) -> None:
    path = tmp_path / "text"
    path.write_bytes(b"\xff")
    with pytest.raises(UnicodeDecodeError):
        inspection.read_regular_text(path, encoding="utf-8")
    assert inspection.read_regular_text(path, encoding="utf-8", errors="replace") == "\ufffd"
    path.write_bytes(b"a\r\nb\rc\n")
    assert inspection.read_regular_text(path) == path.read_text()


@pytest.mark.parametrize("field", ["timeout_seconds", "termination_grace_seconds"])
@pytest.mark.parametrize("value", [True, 0, -1, float("nan"), float("inf")])
def test_invalid_supervisor_budget_rejects_before_launch(
    tmp_path: Path, field: str, value: object
) -> None:
    args = {"timeout_seconds": 1, field: value}
    with pytest.raises(ValueError, match="finite and positive"):
        run_process(["must-not-launch"], cwd=tmp_path, env={}, **args)


@pytest.mark.parametrize("value", [True, 0, -1, float("nan"), float("inf")])
def test_invalid_batch_deadline_rejects_before_launch(tmp_path: Path, value: object) -> None:
    with pytest.raises(ValueError, match="finite and positive"):
        run_process_batch([SupervisedCommand(["must-not-launch"], tmp_path, {}, value)])


@pytest.mark.parametrize("binary", [False, True])
def test_large_stdin_backpressure_survives_capture_polls(tmp_path: Path, binary: bool) -> None:
    payload = "x" * 2_000_000
    result = run_process(
        [sys.executable, "-c", "import sys,time; time.sleep(.5); print(len(sys.stdin.read()))"],
        cwd=tmp_path,
        env=os.environ.copy(),
        timeout_seconds=4,
        input_text=payload,
        binary_output=binary,
    )
    assert result.returncode == 0
    assert result.stdout == (b"2000000\n" if binary else "2000000\n")


def test_fifo_scenario_rejects_before_build(tmp_path: Path, monkeypatch) -> None:
    scenario = tmp_path / "scenario"
    os.mkfifo(scenario)
    monkeypatch.setattr(
        sys,
        "argv",
        ["host_run", str(scenario), str(tmp_path / "raw"), str(tmp_path / "summary"), "cal"],
    )
    monkeypatch.setattr(host_run, "build_runner", lambda *args: pytest.fail("must not build"))
    assert host_run.main() == 1


@pytest.mark.parametrize("kind", ["fifo", "symlink"])
def test_ledger_replacement_rejects_without_waiting(tmp_path: Path, kind: str) -> None:
    path = tmp_path / "ledger"
    os.mkfifo(path)
    if kind == "symlink":
        link = tmp_path / "link"
        link.symlink_to(path)
        path = link
    with pytest.raises(OSError):
        host_study.append(path, {})


def test_incomplete_allocation_producer_exits_failure(monkeypatch) -> None:
    monkeypatch.setattr(
        bench_process,
        "run_bench",
        lambda *args, **kwargs: SupervisedProcess(None, "partial", "timeout", True, None),
    )
    with pytest.raises(SystemExit) as caught:
        allocation_gate.run_producer()
    assert caught.value.code == 1


def test_runner_installation_timeout_cannot_use_cached_version(monkeypatch) -> None:
    def incomplete(command, **kwargs):
        if command == [iai_gate.RUNNER]:
            return SupervisedBinaryProcess(1, b"", b"iai-callgrind-runner (0.14.2)", False, None)
        raise inspection.InspectionExecutionError("timeout")

    monkeypatch.setattr(iai_gate, "inspect_process", incomplete)
    version, reason = iai_gate.detect_runner_version()
    assert version is None and "incomplete" in reason
