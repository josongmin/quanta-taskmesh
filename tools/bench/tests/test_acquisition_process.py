"""Real child-process execution witnesses for bounded acquisition ownership."""

from __future__ import annotations

import json
import os
import select
import signal
import subprocess
import sys
import time
from pathlib import Path

import psutil
import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import acquisition_process as acquisition  # noqa: E402


def gone(pid: int) -> bool:
    try:
        return psutil.Process(pid).status() == psutil.STATUS_ZOMBIE
    except psutil.NoSuchProcess:
        return True


def wait_gone(pid: int) -> None:
    deadline = time.monotonic() + 3
    while time.monotonic() < deadline and not gone(pid):
        time.sleep(0.01)
    assert gone(pid), f"child {pid} survived cleanup"


def atomic_pid_marker_source() -> str:
    """Publish a child PID only after its entire marker is written."""
    return (
        "marker=Path(os.environ['MARKER']); "
        "staged=marker.with_name(marker.name+'.staged'); "
        "staged.write_text(str(os.getpid())); staged.replace(marker); "
    )


def wait_pid_marker(marker: Path) -> int:
    deadline = time.monotonic() + 3
    while not marker.is_file() and time.monotonic() < deadline:
        time.sleep(0.01)
    assert marker.is_file(), f"child did not publish {marker}"
    content = marker.read_text()
    assert content.isdecimal(), f"invalid child PID marker: {content!r}"
    return int(content)


def owned_child(directory: Path, call: str) -> subprocess.Popen:
    child = subprocess.Popen(
        [
            sys.executable,
            "-c",
            f"import os,time; print(os.environ[{acquisition.OWNER_CALL!r}], flush=True); "
            "time.sleep(60)",
        ],
        start_new_session=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        env={
            **os.environ,
            acquisition.OWNER_DIRECTORY: str(directory),
            acquisition.OWNER_CALL: call,
        },
    )
    assert child.stdout is not None
    try:
        # Popen completion can precede observable /proc environment publication.
        # Scan an initialized owned child, rather than racing its exec startup.
        readable, _, _ = select.select([child.stdout], [], [], 3)
        assert readable, "owned child did not publish its startup identity"
        assert child.stdout.readline() == (call + "\n").encode()
        return child
    except BaseException:
        if child.poll() is None:
            os.killpg(child.pid, signal.SIGKILL)
        child.wait(timeout=3)
        raise
    finally:
        child.stdout.close()


def test_transient_system_error_retries_environment_observation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    directory = tmp_path / "registry"
    directory.mkdir(mode=0o700)
    actual = psutil.Process(os.getpid())

    class TransientProcess:
        pid = actual.pid
        reads = 0

        def environ(self):
            self.reads += 1
            if self.reads == 1:
                raise SystemError("transient macOS environ failure")
            return actual.environ()

        def status(self):
            return actual.status()

    observed = TransientProcess()
    monkeypatch.setattr(acquisition.psutil, "process_iter", lambda *_: iter([observed]))
    incomplete, diagnostics = acquisition._cleanup(directory, "a" * 32, root_owner=True)
    assert observed.reads == 2
    assert not incomplete, diagnostics
    assert diagnostics == []


def test_persistent_system_error_fails_closed_and_keeps_scanning_owned_child(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    directory = tmp_path / "registry"
    directory.mkdir(mode=0o700)
    call = "a" * 32
    child = owned_child(directory, call)

    class UnreadableProcess:
        pid = os.getpid()
        reads = 0

        def environ(self):
            self.reads += 1
            raise SystemError("persistent macOS environ failure")

    unreadable = UnreadableProcess()
    monkeypatch.setattr(
        acquisition.psutil,
        "process_iter",
        lambda *_: iter([unreadable, psutil.Process(child.pid)]),
    )
    try:
        incomplete, diagnostics = acquisition._cleanup(directory, call, root_owner=True)
        assert unreadable.reads == 2
        assert incomplete
        assert any("owner environment unavailable" in item for item in diagnostics)
        assert any("owned descendant group killed" in item for item in diagnostics), diagnostics
        wait_gone(child.pid)
    finally:
        if not gone(child.pid):
            os.killpg(child.pid, signal.SIGKILL)
        child.wait(timeout=3)


def test_persistent_environment_error_returns_typed_incomplete_result(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    class UnreadableProcess:
        pid = os.getpid()

        def environ(self):
            raise SystemError("persistent macOS environ failure")

    monkeypatch.setattr(
        acquisition.psutil,
        "process_iter",
        lambda *_: iter([UnreadableProcess()]),
    )
    result = acquisition.run_acquisition(
        [sys.executable, "-c", "print('completed')"], cwd=tmp_path, timeout_seconds=3
    )
    assert result.returncode is None
    assert result.aborted_early
    assert not result.timed_out
    assert "owner environment unavailable" in result.stderr
    assert result.stdout == "completed\n"
    assert acquisition.execution_record(result, 3)["aborted_early"] is True


def test_registered_process_observation_error_does_not_skip_other_cleanup(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    directory = tmp_path / "registry"
    directory.mkdir(mode=0o700)
    call = "a" * 32
    child = owned_child(directory, call)
    actual = psutil.Process(child.pid)
    (directory / f"{call}.json").write_text(
        json.dumps(
            {"call": call, "parent": None, "pid": child.pid, "created": actual.create_time()}
        )
    )
    original_process = acquisition.psutil.Process

    class UnreadableRegisteredProcess:
        def create_time(self):
            raise SystemError("registered process observation failure")

    monkeypatch.setattr(
        acquisition.psutil,
        "Process",
        lambda pid: UnreadableRegisteredProcess() if pid == child.pid else original_process(pid),
    )
    monkeypatch.setattr(acquisition.psutil, "process_iter", lambda *_: iter([actual]))
    try:
        incomplete, diagnostics = acquisition._cleanup(directory, call, root_owner=True)
        monkeypatch.setattr(acquisition.psutil, "Process", original_process)
        assert incomplete
        assert any("owner cleanup failed" in item for item in diagnostics)
        assert any("owned descendant group killed" in item for item in diagnostics), diagnostics
        wait_gone(child.pid)
    finally:
        monkeypatch.setattr(acquisition.psutil, "Process", original_process)
        if child.poll() is None:
            child.kill()
        child.wait(timeout=3)


@pytest.mark.parametrize("value", [True, 0, -1, float("nan"), float("inf")])
def test_deadline_rejects_nonpositive_nonfinite_and_boolean(tmp_path: Path, value) -> None:
    with pytest.raises(ValueError, match="positive and finite"):
        acquisition.run_acquisition(
            [sys.executable, "-c", "pass"], cwd=tmp_path, timeout_seconds=value
        )


@pytest.mark.parametrize("value", [True, 0, -1, float("nan"), float("inf")])
def test_termination_grace_rejects_invalid_values_before_launch(tmp_path: Path, value) -> None:
    with pytest.raises(ValueError, match="positive and finite"):
        acquisition.run_acquisition(
            [sys.executable, "-c", "pass"],
            cwd=tmp_path,
            timeout_seconds=1,
            termination_grace_seconds=value,
        )


def test_hung_child_retains_partial_output_and_deadline(tmp_path: Path) -> None:
    marker = tmp_path / "hung-child.pid"
    pids = []
    ready_at = []

    def on_started(process) -> None:
        pids.append(process.pid)
        assert wait_pid_marker(marker) == process.pid
        ready_at.append(time.monotonic())

    result = acquisition.run_acquisition(
        [
            sys.executable,
            "-c",
            "import os,time; from pathlib import Path; print('partial', flush=True); "
            f"{atomic_pid_marker_source()}time.sleep(60)",
        ],
        cwd=tmp_path,
        timeout_seconds=0.2,
        env={**os.environ, "MARKER": str(marker)},
        on_started=on_started,
    )
    assert result.timed_out and result.returncode != 0
    assert result.stdout == "partial\n"
    assert time.monotonic() - ready_at[0] < 3
    wait_gone(pids[0])


def test_on_started_exposes_exact_payload_pid_without_reap_capability(tmp_path: Path) -> None:
    started = []
    result = acquisition.run_acquisition(
        [sys.executable, "-c", "import os; print(os.getpid())"],
        cwd=tmp_path,
        timeout_seconds=2,
        on_started=started.append,
    )
    assert result.returncode == 0
    assert len(started) == 1 and started[0].pid == int(result.stdout)
    assert not hasattr(started[0], "wait") and not hasattr(started[0], "poll")


def test_capture_limit_is_bounded_and_not_success(tmp_path: Path, monkeypatch) -> None:
    monkeypatch.setattr(acquisition, "MAX_CAPTURE_BYTES", 4096)
    result = acquisition.run_acquisition(
        [sys.executable, "-c", "import os; os.write(1, b'x' * 100000); os.write(2,b'y' * 100000)"],
        cwd=tmp_path,
        timeout_seconds=2,
    )
    assert result.aborted_early
    assert len(result.stdout.encode()) <= 4096
    assert len(result.stdout.encode()) + len(result.stderr.encode()) < 4300
    assert "capture byte limit exceeded" in result.stderr


def test_regular_file_stdin_does_not_block_capture(tmp_path: Path) -> None:
    payload = b"input" * 500000
    result = acquisition.run_acquisition(
        [sys.executable, "-c", "import sys; print(len(sys.stdin.buffer.read()))"],
        cwd=tmp_path,
        timeout_seconds=3,
        stdin_data=payload,
    )
    assert result.returncode == 0 and not result.aborted_early
    assert int(result.stdout) == len(payload)


def test_closed_pipe_survivor_is_killed_and_cannot_turn_leader_exit_green(tmp_path: Path) -> None:
    marker = tmp_path / "survivor.pid"
    child_pids = []

    def on_started(_process) -> None:
        child_pid = wait_pid_marker(marker)
        assert not gone(child_pid), f"child {child_pid} exited before leader"
        child_pids.append(child_pid)

    leader = (
        "import select,subprocess,sys\n"
        "child=subprocess.Popen([sys.executable,'-c',\"import os,time; "
        f"from pathlib import Path; {atomic_pid_marker_source()}"
        "print('READY:'+str(os.getpid()),flush=True); time.sleep(60)\"],"
        "stdout=subprocess.PIPE,stderr=subprocess.DEVNULL)\n"
        "ready,_,_=select.select([child.stdout],[],[],3)\n"
        "if not ready: raise RuntimeError('child readiness absent')\n"
        "if child.stdout.readline()!=('READY:'+str(child.pid)+'\\n').encode(): "
        "raise RuntimeError('child readiness invalid')\n"
        "child.stdout.close()\n"
    )
    result = acquisition.run_acquisition(
        [sys.executable, "-c", leader],
        cwd=tmp_path,
        timeout_seconds=3,
        env={**os.environ, "MARKER": str(marker)},
        on_started=on_started,
    )
    assert not result.timed_out
    assert result.aborted_early and result.returncode != 0
    wait_gone(child_pids[0])


def test_nested_helper_session_survives_collector_exit_but_is_owned(tmp_path: Path) -> None:
    marker = tmp_path / "nested.pid"
    helper_path = str(acquisition.REPO / "tools/bench")
    nested = (
        "import sys\n"
        f"sys.path.insert(0,{helper_path!r})\n"
        "from acquisition_process import run_acquisition\n"
        "from pathlib import Path\n"
        "run_acquisition([sys.executable,'-c',\"import os,time; from pathlib import Path; "
        f"{atomic_pid_marker_source()}time.sleep(60)\"], "
        "cwd=Path.cwd(),timeout_seconds=60)\n"
    )
    leader = (
        "import os,subprocess,sys,time\n"
        f"subprocess.Popen([sys.executable,'-c',{nested!r}],start_new_session=True,"
        "stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\n"
        "until=time.monotonic()+3\n"
        "while not os.path.exists(os.environ['MARKER']) and time.monotonic()<until:\n"
        "    time.sleep(.01)\n"
        "if not os.path.exists(os.environ['MARKER']): "
        "raise RuntimeError('child readiness absent')\n"
    )
    result = acquisition.run_acquisition(
        [sys.executable, "-c", leader],
        cwd=tmp_path,
        timeout_seconds=5,
        env={**os.environ, "MARKER": str(marker)},
    )
    assert result.aborted_early and result.returncode != 0
    wait_gone(wait_pid_marker(marker))


@pytest.mark.parametrize("signum", [signal.SIGINT, signal.SIGTERM])
def test_signal_accounts_for_child_cleanup(tmp_path: Path, signum: int) -> None:
    marker = tmp_path / "signal.pid"
    wrapper = (
        "import json,sys\n"
        f"sys.path.insert(0,{str(acquisition.REPO / 'tools/bench')!r})\n"
        "from acquisition_process import run_acquisition,execution_record\n"
        "from pathlib import Path\n"
        "result=run_acquisition([sys.executable,'-c',\"import os,signal; from pathlib import Path; "
        "signal.signal(signal.SIGTERM,signal.SIG_IGN); "
        f"{atomic_pid_marker_source()}signal.pause()\"],"
        "cwd=Path.cwd(),timeout_seconds=60)\n"
        "print(json.dumps(execution_record(result,60)))\n"
    )
    process = subprocess.Popen(
        [sys.executable, "-c", wrapper],
        cwd=tmp_path,
        env={**os.environ, "MARKER": str(marker)},
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    child_pid = None
    try:
        child_pid = wait_pid_marker(marker)
        process.send_signal(signum)
        stdout, stderr = process.communicate(timeout=5)
        assert process.returncode == 0, stderr
        record = json.loads(stdout)
        assert record["interrupted_by_signal"] == signum
        assert record["returncode"] != 0
        wait_gone(child_pid)
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=3)
        if child_pid is not None and not gone(child_pid):
            os.kill(child_pid, signal.SIGKILL)


def test_nested_timeout_kills_registered_child_after_collector_is_killed(tmp_path: Path) -> None:
    marker = tmp_path / "nested-timeout.pid"
    nested = (
        "import sys,signal\n"
        f"sys.path.insert(0,{str(acquisition.REPO / 'tools/bench')!r})\n"
        "from acquisition_process import run_acquisition\n"
        "from pathlib import Path\n"
        "run_acquisition([sys.executable,'-c',\"import os,signal; from pathlib import Path; "
        "signal.signal(signal.SIGTERM,signal.SIG_IGN); "
        f"{atomic_pid_marker_source()}signal.pause()\"],"
        "cwd=Path.cwd(),timeout_seconds=60)\n"
    )
    leader = (
        "import subprocess,sys,signal\n"
        f"subprocess.Popen([sys.executable,'-c',{nested!r}],start_new_session=True)\n"
        "signal.pause()\n"
    )
    result = acquisition.run_acquisition(
        [sys.executable, "-c", leader],
        cwd=tmp_path,
        timeout_seconds=0.5,
        env={**os.environ, "MARKER": str(marker)},
        on_started=lambda _process: wait_pid_marker(marker),
    )
    assert result.timed_out and result.returncode != 0
    wait_gone(wait_pid_marker(marker))


@pytest.mark.parametrize(
    "mutation",
    [
        {"call": "bad", "parent": [], "pid": 1, "created": 0},
        {"parent": []},
        {"pid": 0},
        {"pid": True},
        {"created": True},
        {"created": float("nan")},
        {"created": float("inf")},
        {"call": []},
    ],
)
def test_malformed_registry_fails_receipt_and_still_cleans_valid_live_child(
    tmp_path: Path, mutation: dict
) -> None:
    marker = tmp_path / "malformed-survivor.pid"
    leader = (
        (
            "import os,json,subprocess,sys,time,uuid,psutil\n"
            "from pathlib import Path\n"
            "directory=Path(os.environ['TASKMESH_ACQUISITION_OWNER_DIRECTORY'])\n"
            "call=uuid.uuid4().hex\n"
            "child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)'], "
            "start_new_session=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\n"
            "created=psutil.Process(child.pid).create_time()\n"
            "good={'call':call,'parent':os.environ['TASKMESH_ACQUISITION_OWNER_CALL'],"
            "'pid':child.pid,'created':created}\n"
            "(directory/(call+'.json')).write_text(json.dumps(good))\n"
            "badcall=uuid.uuid4().hex\n"
            "bad={'call':badcall,'parent':None,'pid':child.pid,'created':created}\n"
            f"bad.update({mutation!r})\n"
            "(directory/(badcall+'.json')).write_text(json.dumps(bad))\n"
            "Path(os.environ['MARKER']).write_text(str(child.pid))\n"
        )
        .replace("nan", "float('nan')")
        .replace("inf", "float('inf')")
    )
    result = acquisition.run_acquisition(
        [sys.executable, "-c", leader],
        cwd=tmp_path,
        timeout_seconds=3,
        env={**os.environ, "MARKER": str(marker)},
    )
    assert marker.exists(), result.stderr
    wait_gone(int(marker.read_text()))
    assert result.returncode != 0 and result.aborted_early
    assert "owner registry unavailable" in result.stderr
    receipt = acquisition.execution_record(result, 3)
    assert receipt["aborted_early"] and receipt["returncode"] != 0


@pytest.mark.parametrize("shape", ["oversized", "symlink", "fifo", "self"])
def test_untrusted_registry_files_cannot_block_cleanup_or_signal_controller(
    tmp_path: Path, shape: str, monkeypatch
) -> None:
    directory = tmp_path / "registry"
    directory.mkdir(mode=0o700)
    call = "a" * 32
    path = directory / (call + ".json")
    record = {
        "call": call,
        "parent": None,
        "pid": os.getpid(),
        "created": psutil.Process().create_time(),
    }
    if shape == "oversized":
        path.write_bytes(b"x" * (acquisition.MAX_OWNER_RECORD_BYTES + 1))
    elif shape == "symlink":
        target = tmp_path / "record"
        target.write_text(json.dumps(record))
        path.symlink_to(target)
    elif shape == "fifo":
        os.mkfifo(path)
    else:
        path.write_text(json.dumps(record))
    monkeypatch.setattr(acquisition.os, "killpg", lambda *_: pytest.fail("unsafe group signal"))
    started = time.monotonic()
    incomplete, diagnostics = acquisition._cleanup(directory, call)
    assert incomplete and diagnostics
    assert time.monotonic() - started < 2


def test_root_cleanup_owns_nested_session_even_after_parent_graph_corruption(
    tmp_path: Path,
) -> None:
    marker = tmp_path / "corrupt-nested.pid"
    nested = (
        "import sys\n"
        f"sys.path.insert(0,{str(acquisition.REPO / 'tools/bench')!r})\n"
        "from acquisition_process import run_acquisition\n"
        "from pathlib import Path\n"
        "run_acquisition([sys.executable,'-c',\"import os,time,json; from pathlib import Path; "
        "folder=Path(os.environ['TASKMESH_ACQUISITION_OWNER_DIRECTORY']); "
        "call=os.environ['TASKMESH_ACQUISITION_OWNER_CALL']; path=folder/(call+'.json'); "
        "until=time.monotonic()+3; "
        "\\nwhile not path.exists() and time.monotonic()<until: time.sleep(.01)"
        "\\nrecord=json.loads(path.read_text()); record['parent']=[]; "
        "path.write_text(json.dumps(record)); "
        f"{atomic_pid_marker_source()}time.sleep(60)\"],"
        "cwd=Path.cwd(),timeout_seconds=60)\n"
    )
    leader = (
        "import os,subprocess,sys,time\n"
        f"subprocess.Popen([sys.executable,'-c',{nested!r}],start_new_session=True,"
        "stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\n"
        "until=time.monotonic()+3\n"
        "while not os.path.exists(os.environ['MARKER']) and time.monotonic()<until:\n"
        "    time.sleep(.01)\n"
        "if not os.path.exists(os.environ['MARKER']): "
        "raise RuntimeError('child readiness absent')\n"
    )
    result = acquisition.run_acquisition(
        [sys.executable, "-c", leader],
        cwd=tmp_path,
        timeout_seconds=5,
        env={**os.environ, "MARKER": str(marker)},
    )
    assert result.aborted_early and result.returncode != 0
    assert "invalid owner parent identity" in result.stderr
    wait_gone(wait_pid_marker(marker))
    receipt = acquisition.execution_record(result, 5)
    assert receipt["aborted_early"] and receipt["returncode"] != 0


def test_nested_cleanup_does_not_kill_cooperative_sibling_group(tmp_path: Path) -> None:
    directory = tmp_path / "registry"
    directory.mkdir(mode=0o700)
    own_call, sibling_call = "a" * 32, "b" * 32
    children = []
    try:
        for call in (own_call, sibling_call):
            process = subprocess.Popen(
                [sys.executable, "-c", "import time; time.sleep(60)"],
                start_new_session=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                env={
                    **os.environ,
                    acquisition.OWNER_DIRECTORY: str(directory),
                    acquisition.OWNER_CALL: call,
                },
            )
            children.append(process)
            record = {
                "call": call,
                "parent": None,
                "pid": process.pid,
                "created": psutil.Process(process.pid).create_time(),
            }
            (directory / (call + ".json")).write_text(json.dumps(record))
        acquisition._cleanup(directory, own_call, root_owner=False)
        wait_gone(children[0].pid)
        assert children[1].poll() is None
    finally:
        for process in children:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=3)
