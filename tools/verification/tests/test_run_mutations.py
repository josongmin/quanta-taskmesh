"""Strict process, classification, and isolation tests for curated mutations."""

from __future__ import annotations

import importlib.util
import json
import os
import select
import signal
import subprocess
import sys
import time
from copy import deepcopy
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[3]
RUNNER = REPO / "tools/verification/run_mutations.py"
_spec = importlib.util.spec_from_file_location("run_mutations", RUNNER)
assert _spec and _spec.loader
rm = importlib.util.module_from_spec(_spec)
sys.modules["run_mutations"] = rm
_spec.loader.exec_module(rm)
campaign_module = sys.modules["campaign"]


def test_timeout_exit_race_preserves_process_truth(tmp_path: Path, monkeypatch) -> None:
    class ExitingProcess:
        pid = 12345
        returncode = 0
        calls = 0

        def communicate(self, timeout=None):
            self.calls += 1
            if self.calls == 1:
                raise subprocess.TimeoutExpired(["probe"], timeout)
            return "finished", ""

    process = ExitingProcess()
    import tools.process_supervisor as supervisor

    monkeypatch.setattr(supervisor.subprocess, "Popen", lambda *_a, **_kw: process)

    clock_calls = 0

    def monotonic_after_timeout() -> float:
        nonlocal clock_calls
        clock_calls += 1
        return 0.0 if clock_calls <= 2 else 2.0

    monkeypatch.setattr(campaign_module.time, "monotonic", monotonic_after_timeout)

    def already_exited(_pid: int, _sig: int) -> None:
        raise ProcessLookupError

    monkeypatch.setattr(campaign_module.os, "killpg", already_exited)
    result = campaign_module.execute(["probe"], cwd=tmp_path, env={}, timeout_seconds=1)
    assert result.timed_out is True
    assert result.returncode == 0
    assert result.stdout == "finished"


def test_parent_sigterm_kills_and_reaps_the_owned_process_group(tmp_path: Path) -> None:
    marker = tmp_path / "child.pid"
    wrapper = tmp_path / "wrapper.py"
    wrapper.write_text(
        "import json, os, sys\n"
        f"sys.path.insert(0, {str(REPO)!r})\n"
        "from pathlib import Path\n"
        "from tools.verification.campaign import execute\n"
        "import tools.process_supervisor as supervisor\n"
        "supervisor.TERMINATION_GRACE_SECONDS = 0.1\n"
        "marker = Path(sys.argv[1])\n"
        'child = "from pathlib import Path; import os, signal, sys; '
        "signal.signal(signal.SIGTERM, signal.SIG_IGN); "
        "marker = Path(sys.argv[1]); pending = marker.with_suffix('.tmp'); "
        'pending.write_text(str(os.getpid())); os.replace(pending, marker); signal.pause()"\n'
        "result = execute([sys.executable, '-c', child, str(marker)], "
        "cwd=Path.cwd(), env=os.environ.copy(), timeout_seconds=60)\n"
        "print(json.dumps({'returncode': result.returncode, 'signal': result.signal, "
        "'interrupted_by_signal': result.interrupted_by_signal}))\n",
        encoding="utf-8",
    )
    parent = subprocess.Popen(
        [sys.executable, str(wrapper), str(marker)],
        cwd=REPO,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    child_pid: int | None = None
    try:
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and not marker.exists():
            if parent.poll() is not None:
                break
            time.sleep(0.01)
        assert marker.exists(), "owned child never reached its blocking point"
        child_pid = int(marker.read_text(encoding="utf-8"))
        parent.send_signal(signal.SIGTERM)
        stdout, stderr = parent.communicate(timeout=5)
        assert parent.returncode == 0, stderr
        result = json.loads(stdout)
        assert result["interrupted_by_signal"] == signal.SIGTERM
        assert result["signal"] == signal.SIGKILL
    finally:
        if parent.poll() is None:
            parent.kill()
            parent.wait(timeout=5)
        if child_pid is not None:
            try:
                os.kill(child_pid, 0)
            except ProcessLookupError:
                pass
            else:
                os.kill(child_pid, signal.SIGKILL)
                raise AssertionError("owned child survived parent cancellation")


def test_timeout_escalates_and_reaps_the_owned_process_group(tmp_path: Path, monkeypatch) -> None:
    marker = tmp_path / "child.pid"
    child = (
        "from pathlib import Path; import os, signal, sys; "
        "signal.signal(signal.SIGTERM, signal.SIG_IGN); "
        "marker = Path(sys.argv[1]); pending = marker.with_suffix('.tmp'); "
        "pending.write_text(str(os.getpid())); os.replace(pending, marker); "
        "sys.stderr.write('READY\\n'); sys.stderr.flush(); signal.pause()"
    )
    import tools.process_supervisor as supervisor

    original_popen = supervisor.subprocess.Popen

    def popen_after_child_ready(*args, **kwargs):
        process = original_popen(*args, **kwargs)
        assert process.stderr is not None
        readable, _, _ = select.select([process.stderr], [], [], 5)
        if not readable:
            os.killpg(process.pid, signal.SIGKILL)
            process.communicate()
            raise AssertionError("owned child never reached its blocking point")
        assert process.stderr.readline() == "READY\n"
        return process

    previous_grace = supervisor.TERMINATION_GRACE_SECONDS
    supervisor.TERMINATION_GRACE_SECONDS = 0.1
    try:
        with monkeypatch.context() as patch:
            patch.setattr(supervisor.subprocess, "Popen", popen_after_child_ready)
            result = campaign_module.execute(
                [sys.executable, "-c", child, str(marker)],
                cwd=tmp_path,
                env=os.environ.copy(),
                timeout_seconds=0.1,
            )
    finally:
        supervisor.TERMINATION_GRACE_SECONDS = previous_grace
    assert result.timed_out is True
    assert result.interrupted_by_signal is None
    assert result.signal == signal.SIGKILL
    child_pid = int(marker.read_text(encoding="utf-8"))
    with pytest.raises(ProcessLookupError):
        os.kill(child_pid, 0)


def test_successful_leader_with_live_detached_pipe_child_is_not_success(tmp_path: Path) -> None:
    import tools.process_supervisor as supervisor

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
        result = supervisor.run_process(
            [sys.executable, "-c", parent, str(marker), child],
            cwd=tmp_path,
            env=os.environ.copy(),
            timeout_seconds=5,
        )
        assert marker.exists(), result.stderr
        child_pid, pgid = map(int, marker.read_text(encoding="utf-8").split())
        assert result.returncode is None, "a live owned child must invalidate leader exit 0"
        assert result.timed_out is False
        assert "live process group" in result.stderr
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
            raise AssertionError("orphaned owned child survived supervisor return")
    finally:
        if pgid is not None:
            try:
                os.killpg(pgid, signal.SIGKILL)
            except ProcessLookupError:
                pass


def test_curated_campaign_records_unstarted_rows_after_interruption(
    tmp_path: Path, monkeypatch
) -> None:
    first = {**MUT, "id": "first"}
    second = {**MUT, "id": "second"}
    identity = rm.command_identity(first)
    baseline = SimpleNamespace(
        identity_sha256="baseline",
        interrupted_by_signal=None,
    )
    calls: list[str] = []

    def interrupt_one(mutation, _campaign, _baseline, _raw_dir):
        calls.append(mutation["id"])
        return rm.Outcome(
            mutation_id=mutation["id"],
            finding=mutation["finding"],
            status="INTERRUPTED",
            detail="cancelled",
            duration_s=0.1,
            interrupted_by_signal=signal.SIGTERM,
        )

    monkeypatch.setattr(rm, "run_one", interrupt_one)
    outcomes, interrupted = rm.run_selected_mutations(
        [first, second], {identity: baseline}, SimpleNamespace(), tmp_path
    )
    assert calls == ["first"]
    assert interrupted == signal.SIGTERM
    assert [outcome.status for outcome in outcomes] == ["INTERRUPTED", "NOT_RUN_INTERRUPTED"]
    assert outcomes[1].interrupted_by_signal == signal.SIGTERM


MUT = {
    "id": "m",
    "finding": "F",
    "file": "src.rs",
    "find": "a",
    "replace": "b",
    "package": "p",
    "test_target": "t",
    "expect_failing_test": "the_test",
    "expect_message": "the property",
    "expect_cofailures": [],
}


def cargo_run(*failed: str, total: int = 3, messages: dict[str, str] | None = None) -> str:
    lines = [f"running {total} tests"]
    for name in failed:
        lines.append(f"test {name} ... FAILED")
    if failed:
        lines.append("\nfailures:\n")
        for name in failed:
            message = (messages or {}).get(name, "the property")
            lines.append(f"---- {name} stdout ----\n{message}\n")
        lines.append("failures:")
    lines.append(f"test result: {'FAILED' if failed else 'ok'}. 0 passed")
    return "\n".join(lines) + "\n"


def process(
    stdout: str,
    *,
    exit_code: int | None = 0,
    process_signal: int | None = None,
    timed_out: bool = False,
) -> rm.ProcessResult:
    return rm.ProcessResult(
        argv=["cargo", "test"],
        returncode=-process_signal if process_signal is not None else exit_code,
        exit_code=exit_code,
        signal=process_signal,
        timed_out=timed_out,
        stdout=stdout,
        stderr="",
        started_at="2026-09-21T00:00:00Z",
        finished_at="2026-09-21T00:00:01Z",
        duration_s=1.0,
    )


def init_repo(path: Path) -> None:
    subprocess.run(["git", "init", "-q"], cwd=path, check=True)
    subprocess.run(["git", "config", "user.email", "test@example.invalid"], cwd=path, check=True)
    subprocess.run(["git", "config", "user.name", "test"], cwd=path, check=True)
    (path / "src.rs").write_text("a\n", encoding="utf-8")
    (path / "target").mkdir()
    (path / "target/sentinel").write_text("normal-target", encoding="utf-8")
    subprocess.run(["git", "add", "src.rs"], cwd=path, check=True)
    subprocess.run(["git", "commit", "-qm", "base"], cwd=path, check=True)


def test_kill_requires_nonzero_exit_exact_failure_set_and_named_reason() -> None:
    killed = process(cargo_run("the_test"), exit_code=101)
    assert rm.evaluate(MUT, killed)[0] == "KILLED"
    assert rm.evaluate(MUT, process(cargo_run("the_test"), exit_code=0))[0] == (
        "INVALID_EXIT_SUCCESS"
    )
    unrelated = process(cargo_run("the_test", "other"), exit_code=101)
    assert rm.evaluate(MUT, unrelated)[0] == "UNRELATED_FAILURE_SET"
    wrong_reason = process(
        cargo_run("the_test", messages={"the_test": "unrelated reason"}), exit_code=101
    )
    assert rm.evaluate(MUT, wrong_reason)[0] == "WRONG_REASON"


def test_declared_cofailure_is_exact_not_allow_all() -> None:
    mutation = {**MUT, "expect_cofailures": ["known_peer"]}
    exact = process(cargo_run("the_test", "known_peer"), exit_code=101)
    assert rm.evaluate(mutation, exact)[0] == "KILLED"
    missing = process(cargo_run("the_test"), exit_code=101)
    assert rm.evaluate(mutation, missing)[0] == "UNRELATED_FAILURE_SET"
    extra = process(cargo_run("the_test", "known_peer", "stranger"), exit_code=101)
    assert rm.evaluate(mutation, extra)[0] == "UNRELATED_FAILURE_SET"


@pytest.mark.parametrize(
    ("result", "status"),
    [
        (process(cargo_run(), exit_code=0), "SURVIVED"),
        (process(cargo_run(), exit_code=101), "INVALID_NO_FAILURE_DETAIL"),
        (process("running 0 tests\ntest result: ok. 0 passed\n", exit_code=0), "INVALID_NO_TESTS"),
        (process("running 3 tests\n", exit_code=101), "INVALID_PARTIAL"),
        (process(cargo_run("the_test"), exit_code=None, process_signal=9), "INVALID_SIGNAL"),
        (process(cargo_run("the_test"), exit_code=None, timed_out=True), "INVALID_TIMEOUT"),
    ],
)
def test_abnormal_process_truth_is_never_killed(result: rm.ProcessResult, status: str) -> None:
    assert rm.evaluate(MUT, result)[0] == status


def test_control_and_baseline_require_clean_completion() -> None:
    control = {**MUT, "finding": "control", "expect_no_failure": True}
    assert rm.evaluate(control, process(cargo_run(), exit_code=0))[0] == "CONTROL_GREEN"
    assert rm.evaluate(control, process(cargo_run(), exit_code=101))[0] == "CONTROL_BAD_EXIT"
    assert rm.baseline_evaluate(MUT, process(cargo_run(), exit_code=0))[0] == "PASS"
    assert rm.baseline_evaluate(MUT, process(cargo_run("other"), exit_code=101))[0] == "FAIL"


def test_pytest_failure_reason_is_scoped_to_the_expected_test() -> None:
    mutation = {**MUT, "runner": "pytest", "test_target": "x.py::the_test"}
    output = (
        "__ the_test __\nE AssertionError: wrong\n"
        "=========================== short test summary info ============================\n"
        "FAILED x.py::the_test - AssertionError: wrong\n1 failed in 0.1s\n"
    )
    assert rm.evaluate(mutation, process(output, exit_code=1))[0] == "WRONG_REASON"
    assert (
        rm.evaluate(mutation, process(output.replace("wrong", "the property"), exit_code=1))[0]
        == "KILLED"
    )
    fixture_error = "ERROR x.py::the_test - RuntimeError: the property\n1 error in 0.1s\n"
    assert rm.evaluate(mutation, process(fixture_error, exit_code=1))[0] == (
        "INVALID_HARNESS_ERROR"
    )


def test_a_long_pytest_failure_header_keeps_its_assertion_reason() -> None:
    name = "test_a_long_pytest_failure_header_keeps_its_assertion_reason"
    reason = "a long pytest header lost its assertion reason"
    mutation = {
        **MUT,
        "runner": "pytest",
        "test_target": f"x.py::{name}",
        "expect_failing_test": name,
        "expect_message": reason,
    }
    # Pytest shortens a long test's failure header to two underscores. The
    # summary intentionally omits the assertion reason, so only the scoped
    # header block can establish that this mutation was actually killed.
    output = (
        f"__ {name} __\nE AssertionError: {reason}\n"
        "=========================== short test summary info ============================\n"
        f"FAILED x.py::{name} - AssertionError\n1 failed in 0.1s\n"
    )
    assert rm.evaluate(mutation, process(output, exit_code=1))[0] == "KILLED", reason


def test_cargo_failure_reason_supports_module_qualified_test_names() -> None:
    mutation = {
        **MUT,
        "expect_failing_test": "runtime::tests::the_test",
        "expect_message": "the property",
    }
    output = cargo_run("runtime::tests::the_test")
    assert rm.evaluate(mutation, process(output, exit_code=101))[0] == "KILLED"


def test_inventory_contract_and_command_identity_are_deterministic(tmp_path: Path) -> None:
    inventory = tmp_path / "inventory.json"
    inventory.write_text(
        json.dumps(
            {
                "mutations": [
                    MUT,
                    {**MUT, "id": "control", "finding": "control", "expect_no_failure": True},
                ]
            }
        ),
        encoding="utf-8",
    )
    loaded = rm.load_inventory(inventory)
    assert len(loaded) == 2
    assert rm.command_identity(MUT) == rm.command_identity(dict(reversed(list(MUT.items()))))
    assert rm.test_command(MUT)[:5] == ["cargo", "test", "-p", "p", "--test"]


def test_each_cargo_mutation_uses_its_primary_oracle_exactly_and_single_threaded() -> None:
    assert rm.test_command(MUT)[-5:] == [
        "--",
        "the_test",
        "--exact",
        "--test-threads",
        "1",
    ]
    control = {**MUT, "finding": "control", "expect_no_failure": True}
    assert rm.test_command(control)[-3:] == ["--", "--test-threads", "4"]


@pytest.mark.parametrize(
    ("broken", "reason"),
    [
        ({**MUT, "expect_message": ""}, "expect_message"),
        ({**MUT, "replace": "a"}, "identical"),
        ({**MUT, "runner": "unknown"}, "unknown runner"),
        ({**MUT, "expect_cofailures": "peer"}, "expect_cofailures"),
        ({**MUT, "env": {"X": 1}}, "env must be"),
        ({**MUT, "expect_failing_test": "--ignored"}, "exact Rust test name"),
        (
            {**MUT, "expect_cofailures": ["peer"]},
            "cannot declare cofailures",
        ),
    ],
)
def test_invalid_inventory_entries_fail_closed(broken: dict, reason: str) -> None:
    with pytest.raises(SystemExit, match=reason):
        rm.validate_entry(broken)


@pytest.mark.parametrize("path", ["/tmp/src.rs", "../src.rs", "dir/../src.rs", "./src.rs", "a//b"])
def test_manifest_file_must_be_normalized_repo_relative(path: str) -> None:
    with pytest.raises(SystemExit, match="normalized repository-relative"):
        rm.validate_entry({**MUT, "file": path})


@pytest.mark.parametrize("key", sorted(rm.RUNNER_OWNED_ENV))
def test_manifest_env_cannot_override_runner_isolation_authority(key: str) -> None:
    with pytest.raises(SystemExit, match="runner-owned isolation"):
        rm.validate_entry({**MUT, "env": {key: "attacker-value"}})


def test_runner_owned_environment_wins_defensively() -> None:
    campaign = SimpleNamespace(target=Path("/isolated/target"))
    mutation = {**MUT, "env": {"RUST_BACKTRACE": "1", "CARGO_TARGET_DIR": "/escape"}}
    _actual, recorded = rm.command_environment(mutation, campaign)
    assert recorded["CARGO_TARGET_DIR"] == "/isolated/target"
    assert recorded["PYTHONDONTWRITEBYTECODE"] == "1"
    assert recorded["RUST_BACKTRACE"] == "1"


@pytest.mark.parametrize(
    "key",
    [
        "RUSTDOCFLAGS",
        "RUSTC_WRAPPER",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS",
    ],
)
def test_curated_campaign_rejects_unrecorded_build_environment(
    key: str, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv(key, "unexpected")
    campaign = SimpleNamespace(target=Path("/isolated/target"))
    with pytest.raises(ValueError, match=key):
        rm.command_environment(MUT, campaign)


def test_parent_symlink_escape_is_rejected(tmp_path: Path) -> None:
    source = tmp_path / "source"
    outside = tmp_path / "outside"
    source.mkdir()
    outside.mkdir()
    (outside / "src.rs").write_text("a\n", encoding="utf-8")
    (source / "escape").symlink_to(outside, target_is_directory=True)
    with pytest.raises(RuntimeError, match="escapes the isolated source"):
        rm.apply_mutation(source, {**MUT, "file": "escape/src.rs"})


def test_two_campaigns_have_disjoint_source_and_target_and_preserve_original(
    tmp_path: Path,
) -> None:
    init_repo(tmp_path)
    before = (tmp_path / "src.rs").read_bytes()
    from tools.process_supervisor import SupervisedCommand, run_process_batch

    # Actual parallel producers own Python signal handlers in their main thread.
    # Exercise two processes, retaining the original simultaneous isolation proof.
    script = (
        "import json, sys\n"
        f"sys.path.insert(0, {str(REPO)!r})\n"
        "from dataclasses import asdict\n"
        "from pathlib import Path\n"
        "from tools.verification.campaign import create_isolated_campaign\n"
        "print(json.dumps(asdict(create_isolated_campaign(Path(sys.argv[1]))), default=str))\n"
    )
    results = run_process_batch(
        [
            SupervisedCommand(
                argv=[sys.executable, "-c", script, str(tmp_path)],
                cwd=REPO,
                env=os.environ.copy(),
                timeout_seconds=10,
            )
            for _index in range(2)
        ]
    )
    campaigns = []
    for result in results:
        process = result.process
        if process.returncode == 0:
            values = json.loads(process.stdout)
            for field in ("root", "source", "target"):
                values[field] = Path(values[field])
            campaigns.append(campaign_module.IsolatedCampaign(**values))
    try:
        assert len(campaigns) == 2, [result.process.stderr for result in results]
        assert all(
            not result.process.timed_out
            and result.process.interrupted_by_signal is None
            and not result.process.aborted_early
            for result in results
        )
        assert campaigns[0].source != campaigns[1].source
        assert campaigns[0].target != campaigns[1].target
        for campaign in campaigns:
            rm.apply_mutation(campaign.source, MUT)
            assert (campaign.source / "src.rs").read_text(encoding="utf-8") == "b\n"
        assert (tmp_path / "src.rs").read_bytes() == before
        assert (tmp_path / "target/sentinel").read_text(encoding="utf-8") == "normal-target"
    finally:
        for campaign in campaigns:
            campaign.cleanup()


def test_snapshot_copies_only_git_visible_paths_and_preserves_symlink(tmp_path: Path) -> None:
    init_repo(tmp_path)
    (tmp_path / ".gitignore").write_text("ignored.txt\nignored-dir/\n", encoding="utf-8")
    (tmp_path / "link.rs").symlink_to("src.rs")
    subprocess.run(["git", "add", ".gitignore", "link.rs"], cwd=tmp_path, check=True)
    subprocess.run(["git", "commit", "-qm", "bind ignore and symlink"], cwd=tmp_path, check=True)
    (tmp_path / "ignored.txt").write_text("not-bound", encoding="utf-8")
    (tmp_path / "ignored-dir").mkdir()
    (tmp_path / "ignored-dir/secret").write_text("not-bound", encoding="utf-8")
    (tmp_path / "visible-untracked.rs").write_text("untracked", encoding="utf-8")

    campaign = rm.create_isolated_campaign(tmp_path)
    try:
        assert not (campaign.source / "ignored.txt").exists()
        assert not (campaign.source / "ignored-dir").exists()
        assert (campaign.source / "link.rs").is_symlink()
        assert os.readlink(campaign.source / "link.rs") == "src.rs"
        assert sorted(campaign.source_paths) == campaign_module.git_source_paths(tmp_path)
        snapshot_tracked = subprocess.run(
            ["git", "ls-files", "--cached", "-z"],
            cwd=campaign.source,
            capture_output=True,
            check=True,
        ).stdout
        original_tracked = subprocess.run(
            ["git", "ls-files", "--cached", "-z"],
            cwd=tmp_path,
            capture_output=True,
            check=True,
        ).stdout
        assert snapshot_tracked == original_tracked
        assert (campaign.source / "visible-untracked.rs").is_file()
        assert b"visible-untracked.rs" not in snapshot_tracked
    finally:
        campaign.cleanup()


def test_snapshot_rejects_path_set_race(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    init_repo(tmp_path)
    original_copy = campaign_module.shutil.copy2
    raced = False

    def copy_and_race(*args: object, **kwargs: object) -> object:
        nonlocal raced
        copied = original_copy(*args, **kwargs)
        if not raced:
            raced = True
            (tmp_path / "late.rs").write_text("new git-visible source", encoding="utf-8")
        return copied

    monkeypatch.setattr(campaign_module.shutil, "copy2", copy_and_race)
    with pytest.raises(RuntimeError, match="source changed"):
        rm.create_isolated_campaign(tmp_path)


def test_abrupt_child_exit_can_only_dirty_the_isolated_copy(tmp_path: Path) -> None:
    init_repo(tmp_path)
    campaign = rm.create_isolated_campaign(tmp_path)
    before = (tmp_path / "src.rs").read_bytes()
    child = subprocess.Popen(
        [
            sys.executable,
            "-c",
            "from pathlib import Path; Path('src.rs').write_text('mutant\\n'); "
            "print('mutated', flush=True); import signal; signal.pause()",
        ],
        cwd=campaign.source,
        start_new_session=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    try:
        assert child.stdout is not None
        ready, _, _ = select.select([child.stdout], [], [], 5)
        assert ready, "mutant child did not report its completed write"
        assert child.stdout.readline().strip() == "mutated"
        os.killpg(child.pid, signal.SIGKILL)
        child.wait(timeout=5)
        assert (campaign.source / "src.rs").read_text(encoding="utf-8") == "mutant\n"
        assert (tmp_path / "src.rs").read_bytes() == before
        assert campaign.original_is_unchanged(tmp_path)
    finally:
        if child.poll() is None:
            os.killpg(child.pid, signal.SIGKILL)
            child.wait(timeout=5)
        if child.stdout is not None:
            child.stdout.close()
        campaign.cleanup()


def test_original_path_set_change_is_detected(tmp_path: Path) -> None:
    init_repo(tmp_path)
    campaign = rm.create_isolated_campaign(tmp_path)
    try:
        (tmp_path / "new.txt").write_text("new", encoding="utf-8")
        assert not campaign.original_is_unchanged(tmp_path)
        assert campaign.source_after(tmp_path).startswith("PATH_SET_CHANGED:")
    finally:
        campaign.cleanup()


def test_output_cleanup_is_confined_to_the_v02_artifact_root(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="must be below"):
        rm.prepare_output_dir(tmp_path, tmp_path / "unrelated")
    with pytest.raises(ValueError, match="must name a campaign"):
        rm.prepare_output_dir(tmp_path, tmp_path / "target/sep21/v02")
    output = tmp_path / "target/sep21/v02/safe-run"
    output.mkdir(parents=True)
    (output / "old").write_text("stale", encoding="utf-8")
    assert rm.prepare_output_dir(tmp_path, output) == output
    assert not (output / "old").exists()


def test_baseline_identity_and_manifest_bind_all_evidence_dimensions(tmp_path: Path) -> None:
    record: dict[str, object] = {
        "definition_sha256": "definition",
        "source_head": "head",
        "source_snapshot_sha256": "source",
        "tools_sha256": "tools",
        "status": "PASS",
        "detail": "green",
        "command": ["cargo", "test"],
        "command_sha256": "command",
        "environment": {"CARGO_TARGET_DIR": "/isolated"},
        "profile": "debug",
        "exit_code": 0,
        "signal": None,
        "timed_out": False,
        "interrupted_by_signal": None,
        "completed": True,
        "test_count": 3,
        "observed_failures": [],
        "started_at": "start",
        "finished_at": "finish",
        "duration_s": 1.0,
        "stdout_artifact": "raw/baseline.stdout.log",
        "stderr_artifact": "raw/baseline.stderr.log",
        "stdout_sha256": "stdout",
        "stderr_sha256": "stderr",
    }
    original = rm.baseline_identity(record)
    for key, replacement in (
        ("source_snapshot_sha256", "different-source"),
        ("tools_sha256", "different-tools"),
        ("command_sha256", "different-command"),
        ("exit_code", 1),
        ("interrupted_by_signal", signal.SIGTERM),
        ("stdout_sha256", "different-stdout"),
    ):
        changed = deepcopy(record)
        changed[key] = replacement
        assert rm.baseline_identity(changed) != original

    init_repo(tmp_path)
    campaign = rm.create_isolated_campaign(tmp_path)
    try:
        baseline = rm.Baseline(identity_sha256=original, **record)
        baseline_payload = vars(baseline).copy()
        assert baseline_payload.pop("identity_sha256") == original
        assert rm.baseline_identity(baseline_payload) == original
        tools = [{"name": "cargo", "version": "cargo 1", "identity_sha256": "tool-id"}]
        manifest = rm.build_baseline_manifest(campaign, tools, [baseline])
        payload = {key: value for key, value in manifest.items() if key != "manifest_sha256"}
        assert manifest["manifest_sha256"] == rm.canonical_digest(payload)
        assert manifest["source"]["snapshot_sha256"] == campaign.snapshot_digest
        assert manifest["tools_sha256"] == rm.canonical_digest(tools)
        assert manifest["baselines"][0]["identity_sha256"] == original
    finally:
        campaign.cleanup()


def test_current_inventory_has_one_control_and_explicit_exact_policy() -> None:
    inventory = json.loads(rm.INVENTORY.read_text(encoding="utf-8"))
    mutations = rm.load_inventory(rm.INVENTORY)
    assert inventory["failure_set_policy"] == "cargo_primary_exact_pytest_declared_cofailures_exact"
    assert len(mutations) == 107
    assert sum(bool(mutation.get("expect_no_failure")) for mutation in mutations) == 1


def test_every_current_inventory_anchor_matches_exactly_once() -> None:
    drifted = []
    for mutation in rm.load_inventory(rm.INVENTORY):
        count = (REPO / mutation["file"]).read_text(encoding="utf-8").count(mutation["find"])
        if count != 1:
            drifted.append((mutation["id"], count))
    assert not drifted, f"current-source mutation anchors drifted: {drifted}"
