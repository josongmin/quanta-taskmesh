"""Real subprocess execution ownership; synthetic artifacts are not performance proof."""

from __future__ import annotations

import signal
import sys
import time
from pathlib import Path

import psutil
import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import bench_process  # noqa: E402
import host_aa  # noqa: E402
import host_observer  # noqa: E402
import host_perf  # noqa: E402
import host_recorder  # noqa: E402
import host_run  # noqa: E402
import host_sampler  # noqa: E402
from process_resource import sample_subprocess  # noqa: E402

from tools.process_supervisor import SupervisedProcess  # noqa: E402


def gone(pid: int) -> bool:
    try:
        return psutil.Process(pid).status() == psutil.STATUS_ZOMBIE
    except psutil.NoSuchProcess:
        return True


@pytest.mark.parametrize("samples", [False, True])
@pytest.mark.parametrize("leader_exits", [False, True])
def test_probe_timeout_and_orphan_cleanup_never_pass(
    tmp_path: Path, samples: bool, leader_exits: bool
) -> None:
    pidfile = tmp_path / "child.pid"
    script = (
        "import subprocess, sys, pathlib, time; "
        "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'], "
        "stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL); "
        f"pathlib.Path({str(pidfile)!r}).write_text(str(child.pid)); "
        + ("sys.exit(0)" if leader_exits else "time.sleep(60)")
    )
    start = time.monotonic()
    code, pid, stderr, resources = sample_subprocess(
        [sys.executable, "-c", script],
        tmp_path,
        cadence_ms=10,
        sample_resources=samples,
        timeout_seconds=0.4,
    )
    assert time.monotonic() - start < 5
    assert pid > 0 and code != 0
    assert resources["status"] == "unavailable"
    assert "timeout" in stderr or "settled process group" in stderr
    child = int(pidfile.read_text())
    deadline = time.monotonic() + 2
    while not gone(child) and time.monotonic() < deadline:
        time.sleep(0.01)
    assert gone(child)
    host_perf.validate_resource_artifact(host_perf.canonical(resources), pid)


@pytest.mark.parametrize("timeout", [0, -1, True, float("nan"), float("inf")])
def test_invalid_execution_deadline_rejects_before_launch(tmp_path: Path, timeout: object) -> None:
    with pytest.raises(ValueError, match="finite and positive"):
        sample_subprocess(["must-not-launch"], tmp_path, timeout_seconds=timeout)


def test_stdin_and_pid_callback_survive_multiple_capture_polls(tmp_path: Path) -> None:
    started = []
    result = bench_process.run_bench(
        [
            sys.executable,
            "-c",
            "import sys,time; text=sys.stdin.read(); "
            "time.sleep(0.5); print(text); print('done', file=sys.stderr)",
        ],
        cwd=tmp_path,
        timeout_seconds=3,
        input_text="typed scenario",
        on_started=started.append,
    )
    assert result.returncode == 0
    assert result.stdout == "typed scenario\n" and result.stderr == "done\n"
    assert len(started) == 1 and started[0] > 0


def test_callback_failure_cleans_up_the_launched_probe(tmp_path: Path) -> None:
    started = []

    def failed(pid: int) -> None:
        started.append(pid)
        raise RuntimeError("observer failed")

    with pytest.raises(RuntimeError, match="observer failed"):
        bench_process.run_bench(
            [sys.executable, "-c", "import time; time.sleep(60)"],
            cwd=tmp_path,
            timeout_seconds=2,
            on_started=failed,
        )
    assert gone(started[0])


@pytest.mark.parametrize("mode", ["aa", "snapshot", "recorder", "sampler"])
@pytest.mark.parametrize("failure", ["timeout", "interrupt", "launch"])
def test_control_failure_retains_incomplete_bundle_and_stops_next_arm(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, failure: str, mode: str
) -> None:
    scenario = tmp_path / "scenario"
    scenario.write_bytes(
        (host_perf.REPO / "tools/bench/scenarios/h1-default-send-smoke.json").read_bytes()
    )
    calibration = tmp_path / "calibration"
    calibration.write_bytes(b"{}")
    calls = []

    def failed(*args: object, **kwargs: object) -> SupervisedProcess:
        calls.append((args, kwargs))
        if failure == "launch":
            raise OSError("cannot launch")
        # Even a zero leader status cannot erase timeout or interruption.
        return SupervisedProcess(
            0,
            "partial",
            "failure",
            failure == "timeout",
            signal.SIGTERM if failure == "interrupt" else None,
        )

    monkeypatch.setattr(bench_process, "run_process", failed)
    directory = tmp_path / mode
    with pytest.raises(host_perf.ReceiptError, match="acquisition incomplete"):
        if mode == "aa":
            host_aa.acquire(scenario, calibration, directory, 2)
        elif mode == "snapshot":
            host_observer.acquire(scenario, calibration, directory, 2, 10)
        elif mode == "recorder":
            host_recorder.acquire(scenario, calibration, directory, 2)
        else:
            host_sampler.acquire(scenario, calibration, directory, 2, 10**12)
    assert len(calls) == 1
    bundle = next(directory.glob("*-bundle.json"))
    rejected = host_perf.parse_object(bundle.read_bytes(), "bundle")
    assert rejected["status"] == "incomplete" and len(rejected["failures"]) == 1
    assert "timeout" in rejected["failures"][0]["reason"] or failure != "timeout"


def test_diagnostic_build_timeout_is_a_rejection(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("TASKMESH_BENCH_BUILD_WITNESSES", raising=False)
    monkeypatch.setattr(
        bench_process,
        "run_process",
        lambda *a, **k: SupervisedProcess(0, "partial cargo output", "", True, None),
    )
    with pytest.raises(host_perf.ReceiptError, match="(?s)host runner build failed.*timeout"):
        host_run.build_runner([])


def test_outer_timeout_settles_nested_probe_group(tmp_path: Path) -> None:
    pidfile = tmp_path / "probe.pid"
    child_script = (
        "import os,signal,pathlib,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); "
        f"pathlib.Path({str(pidfile)!r}).write_text(str(os.getpid())); time.sleep(60)"
    )
    wrapper = (
        "import sys; "
        f"sys.path.insert(0, {str(host_perf.REPO / 'tools/bench')!r}); "
        "from process_resource import sample_subprocess; from pathlib import Path; "
        f"result=sample_subprocess([sys.executable,'-c',{child_script!r}], "
        f"Path({str(tmp_path)!r}), "
        "sample_resources=False, timeout_seconds=60); sys.exit(1 if result[0] else 0)"
    )
    result = bench_process.run_bench(
        [sys.executable, "-c", wrapper],
        cwd=tmp_path,
        timeout_seconds=1,
        termination_grace_seconds=bench_process.CONTROL_GRACE_SECONDS,
    )
    assert result.timed_out and result.returncode is None
    assert pidfile.exists(), "nested probe must start before this custody check"
    deadline = time.monotonic() + 2
    pid = int(pidfile.read_text())
    while not gone(pid) and time.monotonic() < deadline:
        time.sleep(0.01)
    assert gone(pid)
