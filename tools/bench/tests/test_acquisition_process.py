"""Real child-process execution witnesses for bounded acquisition ownership."""

from __future__ import annotations

import json
import os
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


@pytest.mark.parametrize("value", [True, 0, -1, float("nan"), float("inf")])
def test_deadline_rejects_nonpositive_nonfinite_and_boolean(tmp_path: Path, value) -> None:
    with pytest.raises(ValueError, match="positive and finite"):
        acquisition.run_acquisition(
            [sys.executable, "-c", "pass"], cwd=tmp_path, timeout_seconds=value
        )


def test_hung_child_retains_partial_output_and_deadline(tmp_path: Path) -> None:
    pids = []
    started = time.monotonic()
    result = acquisition.run_acquisition(
        [sys.executable, "-c", "import time; print('partial', flush=True); time.sleep(60)"],
        cwd=tmp_path,
        timeout_seconds=0.2,
        on_started=lambda process: pids.append(process.pid),
    )
    assert result.timed_out and result.returncode != 0
    assert result.stdout == "partial\n"
    assert time.monotonic() - started < 3
    wait_gone(pids[0])


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
    leader = (
        "import subprocess,sys,time\n"
        "child=subprocess.Popen([sys.executable,'-c',\"import os,time; "
        "from pathlib import Path; Path(os.environ['MARKER']).write_text(str(os.getpid())); "
        'time.sleep(60)"],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\n'
        "time.sleep(.2)\n"
    )
    result = acquisition.run_acquisition(
        [sys.executable, "-c", leader],
        cwd=tmp_path,
        timeout_seconds=3,
        env={**os.environ, "MARKER": str(marker)},
    )
    assert result.returncode != 0 or result.aborted_early
    assert marker.exists()
    wait_gone(int(marker.read_text()))


def test_nested_helper_session_survives_collector_exit_but_is_owned(tmp_path: Path) -> None:
    marker = tmp_path / "nested.pid"
    helper_path = str(acquisition.REPO / "tools/bench")
    nested = (
        "import sys\n"
        f"sys.path.insert(0,{helper_path!r})\n"
        "from acquisition_process import run_acquisition\n"
        "from pathlib import Path\n"
        "run_acquisition([sys.executable,'-c',\"import os,time; from pathlib import Path; "
        "Path(os.environ['MARKER']).write_text(str(os.getpid())); time.sleep(60)\"], "
        "cwd=Path.cwd(),timeout_seconds=60)\n"
    )
    leader = (
        "import os,subprocess,sys,time\n"
        f"subprocess.Popen([sys.executable,'-c',{nested!r}],start_new_session=True,"
        "stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\n"
        "until=time.monotonic()+3\n"
        "while not os.path.exists(os.environ['MARKER']) and time.monotonic()<until:\n"
        "    time.sleep(.01)\n"
    )
    result = acquisition.run_acquisition(
        [sys.executable, "-c", leader],
        cwd=tmp_path,
        timeout_seconds=5,
        env={**os.environ, "MARKER": str(marker)},
    )
    assert marker.exists(), result.stderr
    assert result.aborted_early and result.returncode != 0
    wait_gone(int(marker.read_text()))


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
        "Path(os.environ['MARKER']).write_text(str(os.getpid())); signal.pause()\"],"
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
    try:
        deadline = time.monotonic() + 5
        while not marker.exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        assert marker.exists()
        process.send_signal(signum)
        stdout, stderr = process.communicate(timeout=5)
        assert process.returncode == 0, stderr
        record = json.loads(stdout)
        assert record["interrupted_by_signal"] == signum
        assert record["returncode"] != 0
        wait_gone(int(marker.read_text()))
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=3)
        if marker.exists() and not gone(int(marker.read_text())):
            os.kill(int(marker.read_text()), signal.SIGKILL)


def test_nested_timeout_kills_registered_child_after_collector_is_killed(tmp_path: Path) -> None:
    marker = tmp_path / "nested-timeout.pid"
    nested = (
        "import sys,signal\n"
        f"sys.path.insert(0,{str(acquisition.REPO / 'tools/bench')!r})\n"
        "from acquisition_process import run_acquisition\n"
        "from pathlib import Path\n"
        "run_acquisition([sys.executable,'-c',\"import os,signal; from pathlib import Path; "
        "signal.signal(signal.SIGTERM,signal.SIG_IGN); "
        "Path(os.environ['MARKER']).write_text(str(os.getpid())); signal.pause()\"],"
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
    )
    assert marker.exists(), result.stderr
    assert result.timed_out and result.returncode != 0
    wait_gone(int(marker.read_text()))


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
        "Path(os.environ['MARKER']).write_text(str(os.getpid())); time.sleep(60)\"],"
        "cwd=Path.cwd(),timeout_seconds=60)\n"
    )
    leader = (
        "import os,subprocess,sys,time\n"
        f"subprocess.Popen([sys.executable,'-c',{nested!r}],start_new_session=True,"
        "stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\n"
        "until=time.monotonic()+3\n"
        "while not os.path.exists(os.environ['MARKER']) and time.monotonic()<until:\n"
        "    time.sleep(.01)\n"
    )
    result = acquisition.run_acquisition(
        [sys.executable, "-c", leader],
        cwd=tmp_path,
        timeout_seconds=5,
        env={**os.environ, "MARKER": str(marker)},
    )
    assert marker.exists(), result.stderr
    assert result.aborted_early and result.returncode != 0
    assert "invalid owner parent identity" in result.stderr
    wait_gone(int(marker.read_text()))
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
