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
        'Path(sys.argv[1]).write_text(str(os.getpid())); signal.pause()"\n'
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
