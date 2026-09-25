from __future__ import annotations

import importlib.util
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

import pytest

MODULE_PATH = Path(__file__).parents[1] / "run.py"
SPEC = importlib.util.spec_from_file_location("modelcheck_run", MODULE_PATH)
assert SPEC and SPEC.loader
run = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(run)


def manifest() -> dict:
    return {
        "checkers": {
            "loom": {
                "bounds": {
                    "max_threads": 5,
                    "max_branches": 1000,
                    "max_permutations": "exhaustive",
                    "preemption_bound": "unbounded",
                },
                "required_models": ["loom.a.v1"],
            },
            "shuttle": {
                "scheduler": "random",
                "max_steps": 100,
                "required_models": {"shuttle.a.v1": {"seed": 7, "requested": 4}},
            },
        }
    }


def test_complete_model_witnesses_pass() -> None:
    text = (
        "taskmesh-model-witness checker=loom model_id=loom.a.v1 max_threads=5 "
        "max_branches=1000 max_permutations=exhaustive preemption_bound=unbounded "
        "completed=3\n"
        "taskmesh-model-witness checker=shuttle model_id=shuttle.a.v1 scheduler=random "
        "seed=7 requested=4 completed=4 max_steps=100\n"
    )
    records, problems = run.parse_model_log(text, manifest())
    assert problems == []
    assert len(records) == 2


def test_zero_or_partial_exploration_fails() -> None:
    text = (
        "taskmesh-model-witness checker=loom model_id=loom.a.v1 max_threads=5 "
        "max_branches=1000 max_permutations=exhaustive preemption_bound=unbounded "
        "completed=0\n"
        "taskmesh-model-witness checker=shuttle model_id=shuttle.a.v1 scheduler=random "
        "seed=7 requested=4 completed=3 max_steps=100\n"
    )
    _, problems = run.parse_model_log(text, manifest())
    assert any("positive" in problem for problem in problems)
    assert any("completed 3 of 4" in problem for problem in problems)


def test_missing_model_and_seed_drift_fail() -> None:
    text = (
        "taskmesh-model-witness checker=shuttle model_id=shuttle.a.v1 scheduler=random "
        "seed=8 requested=4 completed=4 max_steps=100\n"
    )
    _, problems = run.parse_model_log(text, manifest())
    assert any("seed" in problem for problem in problems)
    assert any("loom model set" in problem for problem in problems)


def test_missing_and_tampered_replay_artifact_fail(tmp_path: Path) -> None:
    artifact, problems = run.replay_artifact(tmp_path)
    assert artifact is None
    assert any("found 0" in problem for problem in problems)

    schedule = tmp_path / "schedule000.txt"
    schedule.write_text("original")
    original_sha = run.sha256(schedule)
    schedule.write_text("tampered")
    artifact, problems = run.replay_artifact(
        tmp_path, expected_sha256=original_sha, relative_to=tmp_path
    )
    assert artifact is not None
    assert problems == ["replay artifact changed between generation and replay"]


def test_source_change_during_model_run_is_rejected() -> None:
    before = {"head": "a", "paths_digest": "one", "dirty": True}
    assert run.source_drift_problems(before, dict(before)) == []
    assert run.source_drift_problems(before, {**before, "head": "b"}) == [
        "source changed during model execution: head"
    ]


def test_wedged_model_process_times_out_and_is_reaped(tmp_path: Path) -> None:
    result = run.run_command(
        [run.sys.executable, "-c", "import time; time.sleep(30)"],
        {},
        tmp_path / "timeout.log",
        timeout_seconds=0.05,
        termination_grace_seconds=0.05,
    )
    assert result["timed_out"] is True
    assert result["exit_code"] != 0
    assert result["signal"] in {9, 15}
    assert "timed_out=true" in (tmp_path / "timeout.log").read_text()


def test_parent_signal_reaps_owned_model_command(tmp_path: Path) -> None:
    marker = tmp_path / "model-child.pid"
    child = (
        "import os, signal, sys; from pathlib import Path; "
        "p = sys.argv[1]; tmp = p + '.tmp'; "
        "Path(tmp).write_text(str(os.getpid())); "
        "os.replace(tmp, p); signal.pause()"
    )
    wrapper = tmp_path / "model_wrapper.py"
    wrapper.write_text(
        f"import sys\nsys.path.insert(0, {str(MODULE_PATH.parents[2])!r})\n"
        "from pathlib import Path\n"
        "from tools.modelcheck.run import run_command\n"
        f"run_command([sys.executable, '-c', {child!r}, {str(marker)!r}], "
        f"{{}}, Path({str(tmp_path / 'model.log')!r}), timeout_seconds=30)\n",
        encoding="utf-8",
    )
    parent = subprocess.Popen([sys.executable, str(wrapper)], cwd=tmp_path)
    child_pid: int | None = None
    try:
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and not marker.exists():
            if parent.poll() is not None:
                break
            time.sleep(0.01)
        assert marker.exists(), "model child did not start"
        child_pid = int(marker.read_text(encoding="utf-8"))
        parent.send_signal(signal.SIGTERM)
        parent.wait(timeout=5)
        assert parent.returncode == 128 + signal.SIGTERM
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            state = subprocess.run(
                ["ps", "-o", "stat=", "-p", str(child_pid)],
                capture_output=True,
                text=True,
                check=False,
            ).stdout.strip()
            if not state or state.startswith("Z"):
                break
            time.sleep(0.01)
        else:
            pytest.fail("model command survived its parent termination")
    finally:
        if parent.poll() is None:
            parent.kill()
            parent.wait(timeout=5)
        if child_pid is not None:
            try:
                os.kill(child_pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
