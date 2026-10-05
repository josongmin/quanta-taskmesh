"""Release producers retain execution custody and reject incomplete evidence."""

from __future__ import annotations

import json
import signal
import sys
import time
from pathlib import Path

import psutil
import pytest

from tools.process_supervisor import SupervisedBinaryProcess
from tools.release import execution, finding_proof, semver


@pytest.mark.parametrize("producer", ["semver", "finding"])
@pytest.mark.parametrize("signum", [None, signal.SIGINT, signal.SIGTERM])
def test_release_producer_stops_launching_after_real_signal(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, producer: str, signum: int | None
) -> None:
    # Only metadata is a fixture. Each witness is a real supervised process;
    # the first sends a signal while its supervisor owns the handler.
    launches = tmp_path / "launches"
    signal_number = None if signum is None else int(signum)
    child = (
        "import os,signal,sys,time\nfrom pathlib import Path\n"
        f"marker=Path({str(launches)!r})\n"
        "with marker.open('a') as stream: stream.write('launched\\n')\n"
        f"if {signal_number!r} is not None and len(marker.read_text().splitlines()) == 1:\n"
        f"    os.kill(os.getppid(), {signal_number!r})\n"
        "    time.sleep(60)\n"
        "print('running 1 test\\ntest regression ... ok\\n'"
        "      'test result: ok. 1 passed; 0 failed; 0 ignored;')\n"
    )
    argv = [sys.executable, "-c", child]
    source = {"dirty": False, "paths_digest": "stable", "head": "a" * 40, "tree": "b" * 40}
    module = semver if producer == "semver" else finding_proof
    monkeypatch.setattr(module, "REPO", tmp_path)
    monkeypatch.setattr(module, "source_identity", lambda: source)
    if producer == "semver":
        policy = json.loads(semver.POLICY.read_text())
        monkeypatch.setattr(
            semver,
            "git",
            lambda *args: (
                policy["baseline_sha"]
                if args[0] == "rev-parse"
                else f'[workspace.package]\nversion = "{policy["baseline_version"]}"\n'
            ),
        )
        monkeypatch.setattr(semver, "command", lambda *_args: argv)

        def run(command: list[str], **kwargs: object) -> SupervisedBinaryProcess:
            if "--version" in command:
                return SupervisedBinaryProcess(0, policy["semver_tool"].encode(), b"", False, None)
            return execution.run_release_command(command, **kwargs)

        monkeypatch.setattr(semver, "run_release_command", run)
        expected_launches = len(policy["public_crates"])
        manifest_name = "semver-manifest.json"
    else:
        witnesses = [
            {
                "id": f"TM21-{n:03d}",
                "kind": "rust",
                "test": "regression",
                "path": f"crates/taskmesh-engine/tests/witness{n}.rs",
            }
            for n in range(1, 4)
        ]
        for row in witnesses:
            path = tmp_path / row["path"]
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("#[test] fn regression() {}\n")
        spec, plan = tmp_path / "spec.json", tmp_path / "plan.json"
        spec.write_text(json.dumps({"witnesses": witnesses}))
        plan.write_text("{}")
        gates = tmp_path / "tools/gates/required.json"
        gates.parent.mkdir(parents=True)
        gates.write_text('{"required": []}')
        monkeypatch.setattr(finding_proof, "SPEC", spec)
        monkeypatch.setattr(finding_proof, "PLAN", plan)
        monkeypatch.setattr(finding_proof, "spec_problems", lambda *_args: [])
        monkeypatch.setattr(finding_proof, "test_command", lambda _row: argv)
        expected_launches = len(witnesses)
        manifest_name = "finding-proof-manifest.json"
    out = tmp_path / "target/proof"
    assert module.produce(out) == (0 if signum is None else 1)
    rows = json.loads((out / manifest_name).read_bytes())["results"]
    assert len(launches.read_text().splitlines()) == (expected_launches if signum is None else 1)
    assert len(rows) == (expected_launches if signum is None else 1)
    if signum is not None:
        assert rows[0]["interrupted_by_signal"] == signum
        assert rows[0]["exit_code"] is None


def test_release_command_retains_binary_output(tmp_path: Path) -> None:
    result = execution.run_release_command(
        [sys.executable, "-c", "import os; os.write(1,b'out\\xff'); os.write(2,b'err\\xfe')"],
        cwd=tmp_path,
        timeout_seconds=3,
    )
    assert execution.settled_exit_code(result) == 0
    assert result.stdout == b"out\xff"
    assert result.stderr == b"err\xfe"


def test_release_timeout_terminates_grandchild(tmp_path: Path) -> None:
    leader = (
        "import subprocess,sys,time\n"
        "child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)'],"
        "stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\n"
        "print(child.pid,flush=True)\n"
        "time.sleep(60)\n"
    )
    result = execution.run_release_command(
        [sys.executable, "-c", leader], cwd=tmp_path, timeout_seconds=0.5
    )
    assert result.timed_out and execution.settled_exit_code(result) is None
    pid = int(result.stdout.strip())
    deadline = time.monotonic() + 3
    while True:
        try:
            alive = psutil.Process(pid).status() != psutil.STATUS_ZOMBIE
        except psutil.NoSuchProcess:
            alive = False
        if not alive or time.monotonic() >= deadline:
            break
        time.sleep(0.01)
    assert not alive, f"grandchild {pid} survived release command timeout"


def test_release_capture_overflow_cannot_report_completed_exit(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(execution, "MAX_CAPTURE_BYTES", 4096)
    result = execution.run_release_command(
        [sys.executable, "-c", "import os; os.write(1,b'x'*100000)"],
        cwd=tmp_path,
        timeout_seconds=3,
    )
    assert result.aborted_early
    assert execution.settled_exit_code(result) is None
    assert len(result.stdout) <= 4096


@pytest.mark.parametrize("exit_code", [0, 100])
def test_live_child_invalidates_both_clean_and_findings_exit(
    tmp_path: Path, exit_code: int
) -> None:
    leader = (
        "import subprocess,sys\n"
        "child=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)'],"
        "stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\n"
        "print(child.pid,flush=True)\n"
        f"sys.exit({exit_code})\n"
    )
    result = execution.run_release_command(
        [sys.executable, "-c", leader], cwd=tmp_path, timeout_seconds=3
    )
    assert execution.settled_exit_code(result) is None
    assert result.returncode is None
    assert b"live process group" in result.stderr
    pid = int(result.stdout.strip())
    deadline = time.monotonic() + 3
    while True:
        try:
            alive = psutil.Process(pid).status() != psutil.STATUS_ZOMBIE
        except psutil.NoSuchProcess:
            alive = False
        if not alive or time.monotonic() >= deadline:
            break
        time.sleep(0.01)
    assert not alive, f"child {pid} survived leader exit {exit_code}"


@pytest.mark.parametrize(
    ("timed_out", "interrupted", "aborted", "expected"),
    [
        (False, None, False, 0),
        (True, None, False, None),
        (False, 15, False, None),
        (False, None, True, None),
    ],
)
def test_finding_producer_rejects_partial_success_output(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    timed_out: bool,
    interrupted: int | None,
    aborted: bool,
    expected: int | None,
) -> None:
    row = {
        "id": "TM21-001",
        "kind": "rust",
        "path": "crates/taskmesh-engine/tests/witness.rs",
        "test": "regression",
    }
    source = tmp_path / row["path"]
    source.parent.mkdir(parents=True)
    source.write_text("#[test] fn regression() {}\n")
    stdout = (
        b"running 1 test\ntest regression ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored;\n"
    )
    monkeypatch.setattr(
        finding_proof,
        "run_release_command",
        lambda *args, **kwargs: SupervisedBinaryProcess(
            0, stdout, b"", timed_out, interrupted, aborted
        ),
    )
    result = finding_proof.run_one(tmp_path, tmp_path, row)
    assert result["exit_code"] == expected
    assert result["status"] == ("PASS" if expected == 0 else "FAIL")
    assert (tmp_path / "TM21-001.stdout.log").read_bytes() == stdout
    assert result["interrupted_by_signal"] == interrupted
    assert result["aborted_early"] == aborted


@pytest.mark.parametrize(
    ("timed_out", "interrupted", "aborted", "expected"),
    [
        (False, None, False, "CLEAN"),
        (True, None, False, "TOOL_FAILURE"),
        (False, 15, False, "TOOL_FAILURE"),
        (False, None, True, "TOOL_FAILURE"),
    ],
)
def test_semver_producer_rejects_incomplete_zero_exit(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    timed_out: bool,
    interrupted: int | None,
    aborted: bool,
    expected: str,
) -> None:
    policy = json.loads(semver.POLICY.read_text())
    source = {"dirty": False, "paths_digest": "stable", "head": "a" * 40, "tree": "b" * 40}
    monkeypatch.setattr(semver, "REPO", tmp_path)
    monkeypatch.setattr(semver, "source_identity", lambda: source)
    monkeypatch.setattr(
        semver,
        "git",
        lambda *args: (
            policy["baseline_sha"]
            if args[0] == "rev-parse"
            else f'[workspace.package]\nversion = "{policy["baseline_version"]}"\n'
        ),
    )

    def run(argv: list[str], **kwargs: object) -> SupervisedBinaryProcess:
        if "--version" in argv:
            return SupervisedBinaryProcess(0, policy["semver_tool"].encode(), b"", False, None)
        assert kwargs["timeout_seconds"] == 3600
        return SupervisedBinaryProcess(0, b"audit\xff", b"", timed_out, interrupted, aborted)

    monkeypatch.setattr(semver, "run_release_command", run)
    out = tmp_path / "target/semver"
    assert semver.produce(out) == (0 if expected == "CLEAN" else 1)
    manifest = json.loads((out / "semver-manifest.json").read_bytes())
    assert len(manifest["results"]) == (4 if interrupted is None else 1)
    for row in manifest["results"]:
        assert row["status"] == expected
        assert (tmp_path / row["stdout"]["path"]).read_bytes() == b"audit\xff"
