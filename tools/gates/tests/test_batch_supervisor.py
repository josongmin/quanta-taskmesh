"""Real process-group lifecycle proof for parallel verification gates."""

from __future__ import annotations

import json
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]


@pytest.mark.parametrize("mode", ["single", "batch"])
def test_bounded_capture_keeps_normal_output_across_poll_intervals(
    tmp_path: Path, mode: str
) -> None:
    from tools.process_supervisor import SupervisedCommand, run_process, run_process_batch

    argv = [
        sys.executable,
        "-c",
        "import sys, time; print('before', flush=True); time.sleep(0.6); "
        "print('after', flush=True); print('stderr', file=sys.stderr, flush=True)",
    ]
    if mode == "single":
        result = run_process(argv, cwd=tmp_path, env=os.environ.copy(), timeout_seconds=2)
    else:
        result = run_process_batch([SupervisedCommand(argv, tmp_path, os.environ.copy(), 2)])[
            0
        ].process
    assert result.returncode == 0
    assert result.timed_out is False
    assert result.stdout == "before\nafter\n"
    assert result.stderr == "stderr\n"


@pytest.mark.parametrize("mode", ["single", "batch"])
def test_undecodable_gate_output_is_recorded_as_fail(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, mode: str
) -> None:
    from tools.gates import run

    executable = tmp_path / "just"
    executable.write_text(
        "#!/usr/bin/env python3\nimport os\nos.write(1, b'\\xff')\n",
        encoding="utf-8",
    )
    executable.chmod(0o755)
    monkeypatch.setenv("PATH", f"{tmp_path}{os.pathsep}{os.environ['PATH']}")
    gate = {"id": "bad-output", "recipe": "fixture", "platforms": ["any"]}

    result = run.run_gate(gate) if mode == "single" else run.run_gate_batch([gate])[0]

    assert result["status"] == "FAIL"
    assert result["exit_code"] is None
    assert "UnicodeDecodeError" in result["output_tail"]


def test_sigterm_reaps_every_parallel_gate_process_group(tmp_path: Path) -> None:
    markers = [tmp_path / "first.pid", tmp_path / "second.pid"]
    wrapper = tmp_path / "batch.py"
    wrapper.write_text(
        "import json, os, sys\n"
        f"sys.path.insert(0, {str(REPO)!r})\n"
        "from pathlib import Path\n"
        "import tools.process_supervisor as supervisor\n"
        "supervisor.TERMINATION_GRACE_SECONDS = 0.1\n"
        'child = "from pathlib import Path; import os, signal, sys; '
        "signal.signal(signal.SIGTERM, signal.SIG_IGN); "
        "marker = Path(sys.argv[1]); pending = marker.with_suffix('.tmp'); "
        'pending.write_text(str(os.getpid())); os.replace(pending, marker); signal.pause()"\n'
        "commands = [supervisor.SupervisedCommand("
        "[sys.executable, '-c', child, marker], Path.cwd(), os.environ.copy(), 60) "
        "for marker in sys.argv[1:]]\n"
        "results = supervisor.run_process_batch(commands)\n"
        "print(json.dumps([{'returncode': item.process.returncode, "
        "'interrupted_by_signal': item.process.interrupted_by_signal} "
        "for item in results]))\n",
        encoding="utf-8",
    )
    parent = subprocess.Popen(
        [sys.executable, str(wrapper), *(str(marker) for marker in markers)],
        cwd=tmp_path,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    child_pids: list[int] = []
    try:
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and not all(marker.exists() for marker in markers):
            if parent.poll() is not None:
                break
            time.sleep(0.01)
        assert all(marker.exists() for marker in markers), "parallel children did not start"
        child_pids = [int(marker.read_text(encoding="utf-8")) for marker in markers]

        parent.send_signal(signal.SIGTERM)
        stdout, stderr = parent.communicate(timeout=5)
        assert parent.returncode == 0, stderr
        results = json.loads(stdout)
        assert len(results) == 2
        assert all(item["interrupted_by_signal"] == signal.SIGTERM for item in results)
        assert all(item["returncode"] == -signal.SIGKILL for item in results)
        for child_pid in child_pids:
            with pytest.raises(ProcessLookupError):
                os.kill(child_pid, 0)
    finally:
        if parent.poll() is None:
            parent.kill()
            parent.wait(timeout=5)
        for child_pid in child_pids:
            try:
                os.kill(child_pid, 0)
            except ProcessLookupError:
                continue
            os.kill(child_pid, signal.SIGKILL)
            raise AssertionError("parallel child survived cancellation")


def test_batch_successful_leader_with_live_child_is_not_success(tmp_path: Path) -> None:
    from tools.process_supervisor import SupervisedCommand, run_process_batch

    marker = tmp_path / "child.pid"
    child = (
        "from pathlib import Path; import os, signal, sys; "
        "signal.signal(signal.SIGTERM, signal.SIG_IGN); "
        "marker = Path(sys.argv[1]); pending = marker.with_suffix('.tmp'); "
        "pending.write_text(f'{os.getpid()} {os.getpgrp()}'); "
        "os.replace(pending, marker); signal.pause()"
    )
    parent = (
        "import os, subprocess, sys, time\n"
        "marker, child = sys.argv[1:]\n"
        "subprocess.Popen([sys.executable, '-c', child, marker], "
        "stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)\n"
        "deadline = time.monotonic() + 2\n"
        "while not os.path.exists(marker) and time.monotonic() < deadline:\n"
        "    time.sleep(0.01)\n"
        "sys.exit(0 if os.path.exists(marker) else 1)\n"
    )
    pgid: int | None = None
    try:
        results = run_process_batch(
            [
                SupervisedCommand(
                    [sys.executable, "-c", parent, str(marker), child],
                    tmp_path,
                    os.environ.copy(),
                    5,
                )
            ]
        )
        assert marker.exists(), results[0].process.stderr
        child_pid, pgid = map(int, marker.read_text(encoding="utf-8").split())
        assert len(results) == 1
        assert results[0].process.returncode is None
        assert "live process group" in results[0].process.stderr
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            status = subprocess.run(
                ["ps", "-o", "stat=", "-p", str(child_pid)],
                capture_output=True,
                text=True,
                check=False,
            ).stdout.strip()
            if not status or status.startswith("Z"):
                break
            time.sleep(0.01)
        else:
            raise AssertionError("orphaned owned child survived batch supervisor return")
    finally:
        if pgid is not None:
            try:
                os.killpg(pgid, signal.SIGKILL)
            except ProcessLookupError:
                pass


@pytest.mark.parametrize("mode", ["single", "batch"])
def test_escaped_descendant_cannot_hold_capture_pipes_past_deadline(
    tmp_path: Path, mode: str
) -> None:
    marker = tmp_path / "escaped.pid"
    # Fork instead of spawning a second interpreter. The child first leaves the
    # owned group and only then publishes its pid, so the fixture deterministically
    # holds the inherited capture pipes before the supervisor's deadline.
    leader = (
        "from pathlib import Path\n"
        "import os, signal, sys, time\n"
        "marker = Path(sys.argv[1])\n"
        "child = os.fork()\n"
        "if child == 0:\n"
        "    os.setsid()\n"
        "    pending = marker.with_suffix('.tmp')\n"
        "    pending.write_text(str(os.getpid()))\n"
        "    os.replace(pending, marker)\n"
        "    signal.pause()\n"
        "    os._exit(0)\n"
        "deadline = time.monotonic() + 2\n"
        "while not marker.exists() and time.monotonic() < deadline:\n"
        "    time.sleep(0.01)\n"
        "if not marker.exists():\n"
        "    sys.exit(1)\n"
        "signal.pause()\n"
    )
    wrapper = tmp_path / "wrapper.py"
    run = (
        "result = supervisor.run_process(argv, cwd=Path.cwd(), "
        "env=os.environ.copy(), timeout_seconds=2.0)\n"
        if mode == "single"
        else "result = supervisor.run_process_batch([supervisor.SupervisedCommand("
        "argv, Path.cwd(), os.environ.copy(), 2.0)])[0].process\n"
    )
    wrapper.write_text(
        "import json, os, sys\n"
        f"sys.path.insert(0, {str(REPO)!r})\n"
        "from pathlib import Path\n"
        "import tools.process_supervisor as supervisor\n"
        "supervisor.TERMINATION_GRACE_SECONDS = 0.1\n"
        f"argv = [sys.executable, '-c', {leader!r}, sys.argv[1]]\n"
        + run
        + "print(json.dumps({'returncode': result.returncode, "
        "'timed_out': result.timed_out, 'stderr': result.stderr}))\n",
        encoding="utf-8",
    )
    try:
        completed = subprocess.run(
            [sys.executable, str(wrapper), str(marker)],
            cwd=tmp_path,
            capture_output=True,
            text=True,
            timeout=8,
            check=False,
        )
        assert completed.returncode == 0, completed.stderr
        result = json.loads(completed.stdout)
        assert marker.exists(), "escaped descendant never established the hostile fixture"
        escaped_pid = int(marker.read_text(encoding="utf-8"))
        try:
            os.kill(escaped_pid, 0)
        except ProcessLookupError as error:
            raise AssertionError(
                "escaped descendant was not alive when the supervisor returned"
            ) from error
        assert result["timed_out"] is True
        assert isinstance(result["returncode"], int) and result["returncode"] != 0
        assert "capture pipes remained open" in result["stderr"]
    finally:
        if marker.exists():
            escaped_pid = int(marker.read_text(encoding="utf-8"))
            try:
                os.killpg(escaped_pid, signal.SIGKILL)
            except ProcessLookupError:
                pass


def test_capture_read_error_cannot_hang_exception_cleanup(tmp_path: Path) -> None:
    """A capture failure must not reopen an unbounded wait on escaped pipes."""
    marker = tmp_path / "escaped-capture.pid"
    escaped = (
        "from pathlib import Path; import os, signal, sys; "
        "marker = Path(sys.argv[1]); pending = marker.with_suffix('.tmp'); "
        "pending.write_text(str(os.getpid())); os.replace(pending, marker); "
        "signal.pause()"
    )
    leader = (
        "import os, subprocess, sys, time\n"
        "marker, escaped = sys.argv[1:]\n"
        "subprocess.Popen([sys.executable, '-c', escaped, marker], start_new_session=True)\n"
        "deadline = time.monotonic() + 2\n"
        "while not os.path.exists(marker) and time.monotonic() < deadline:\n"
        "    time.sleep(0.01)\n"
        "os.write(1, b'output before capture failure')\n"
    )
    wrapper = tmp_path / "capture_wrapper.py"
    wrapper.write_text(
        "import os, sys, time\n"
        f"sys.path.insert(0, {str(REPO)!r})\n"
        "from pathlib import Path\n"
        "from tools.process_supervisor import run_process\n"
        "import tools.process_supervisor as supervisor\n"
        "supervisor.TERMINATION_GRACE_SECONDS = 0.1\n"
        "original_communicate = supervisor.subprocess.Popen.communicate\n"
        "def fail_first_capture(self, *args, **kwargs):\n"
        "    if not getattr(self, '_capture_failure_injected', False):\n"
        "        self._capture_failure_injected = True\n"
        "        deadline = time.monotonic() + 2\n"
        "        while not os.path.exists(sys.argv[1]) and time.monotonic() < deadline:\n"
        "            time.sleep(0.01)\n"
        "        raise OSError('capture read failed')\n"
        "    return original_communicate(self, *args, **kwargs)\n"
        "supervisor.subprocess.Popen.communicate = fail_first_capture\n"
        f"run_process([sys.executable, '-c', {leader!r}, sys.argv[1], {escaped!r}], "
        "cwd=Path.cwd(), env=os.environ.copy(), timeout_seconds=0.2)\n",
        encoding="utf-8",
    )
    try:
        completed = subprocess.run(
            [sys.executable, str(wrapper), str(marker)],
            cwd=tmp_path,
            capture_output=True,
            text=True,
            timeout=3,
            check=False,
        )
        assert marker.exists(), completed.stderr
        assert completed.returncode != 0
        assert "OSError: capture read failed" in completed.stderr
    finally:
        if marker.exists():
            escaped_pid = int(marker.read_text(encoding="utf-8"))
            try:
                os.killpg(escaped_pid, signal.SIGKILL)
            except ProcessLookupError:
                pass


def test_batch_capture_failure_reaps_the_owned_leader(tmp_path: Path, monkeypatch) -> None:
    from tools import process_supervisor as supervisor

    marker = tmp_path / "batch-error-ready"
    child = (
        "from pathlib import Path; import os, signal, sys; "
        "signal.signal(signal.SIGTERM, signal.SIG_IGN); "
        "marker = Path(sys.argv[1]); pending = marker.with_suffix('.tmp'); "
        "pending.write_text(str(os.getpid())); os.replace(pending, marker); "
        "signal.pause()"
    )
    held: list[subprocess.Popen[str]] = []

    def failed_capture(process: subprocess.Popen[str], _stop) -> tuple[str, str, bool]:
        held.append(process)
        deadline = time.monotonic() + 2
        while not marker.exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        assert marker.exists(), "worker did not start before capture failure"
        raise OSError("capture read failed")

    monkeypatch.setattr(supervisor, "_drain_capture", failed_capture)
    try:
        with pytest.raises(OSError, match="capture read failed"):
            supervisor.run_process_batch(
                [
                    supervisor.SupervisedCommand(
                        [sys.executable, "-c", child, str(marker)],
                        tmp_path,
                        os.environ.copy(),
                        5,
                    )
                ]
            )
        assert len(held) == 1
        assert held[0].returncode == -signal.SIGKILL
    finally:
        for process in held:
            if process.returncode is None:
                process.kill()
                process.wait(timeout=2)
