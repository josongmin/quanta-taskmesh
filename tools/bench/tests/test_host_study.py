"""A failed planned attempt cannot disappear from the study denominator."""

from __future__ import annotations

import json
import os
import signal
import sys
import time
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_perf  # noqa: E402
import host_study  # noqa: E402

from tools.process_supervisor import SupervisedProcess  # noqa: E402


def plan(root: Path) -> bytes:
    for name in ("scenario", "calibration"):
        (root / name).write_bytes(b"{}")
    return host_perf.canonical(
        {
            "schema_version": 2,
            "contract_sha256": "a" * 64,
            "attempts": [
                {
                    "id": arm,
                    "rate_per_second": 1,
                    "pair_index": 0,
                    "arm": arm,
                    "scenario": "scenario",
                    "scenario_sha256": host_perf.sha256(b"{}"),
                    "calibration": "calibration",
                    "calibration_sha256": host_perf.sha256(b"{}"),
                    "features": [],
                    "timeout_seconds": 1,
                }
                for arm in ("baseline", "candidate")
            ],
        }
    )


def test_failed_attempts_are_retained_and_chain_cannot_be_truncated(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    data = plan(tmp_path)
    monkeypatch.setattr(
        host_study,
        "run_process",
        lambda *args, **kwargs: SupervisedProcess(1, "output", "failure", False, None),
    )
    root = tmp_path / "study"
    report = host_study.collect(data, tmp_path, root)
    assert report["expected"] == report["attempted"] == 2
    assert report["failed"] == ["baseline", "candidate"]
    assert (root / "candidate/stderr").read_bytes() == b"failure"
    log = root / "events.jsonl"
    log.write_bytes(b"\n".join(log.read_bytes().splitlines()[:2]) + b"\n")
    with pytest.raises(host_perf.ReceiptError, match="digest/version differs"):
        host_study.verify(root)


def test_input_failure_counts_as_attempt_and_retains_reason(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    data = plan(tmp_path)
    (tmp_path / "scenario").write_bytes(b"changed")
    monkeypatch.setattr(
        host_study, "run_process", lambda *a, **k: pytest.fail("must reject before launch")
    )
    report = host_study.collect(data, tmp_path, tmp_path / "study")
    assert report["failed"] == ["baseline", "candidate"]
    assert all("predeclared digest" in a["reason"] for a in report["attempts"])


@pytest.mark.parametrize("change", ["duplicate", "missing_arm", "escape", "bool_rate"])
def test_plan_rejects_ambiguous_or_missing_population(tmp_path: Path, change: str) -> None:
    value = json.loads(plan(tmp_path))
    if change == "duplicate":
        value["attempts"][1]["id"] = "baseline"
    elif change == "missing_arm":
        value["attempts"].pop()
    elif change == "escape":
        value["attempts"][0]["id"] = "../outside"
    else:
        value["attempts"][0]["rate_per_second"] = True
    with pytest.raises(host_perf.ReceiptError):
        host_study.parse_plan(host_perf.canonical(value))


def test_tampering_retained_failed_artifact_is_rejected(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    data = plan(tmp_path)
    monkeypatch.setattr(
        host_study,
        "run_process",
        lambda *args, **kwargs: SupervisedProcess(1, "", "failed", False, None),
    )
    root = tmp_path / "study"
    host_study.collect(data, tmp_path, root)
    (root / "baseline/stderr").write_bytes(b"hidden failure")
    with pytest.raises(host_perf.ReceiptError, match="inventory or digest changed"):
        host_study.verify(root)


def test_completed_population_keeps_every_artifact_and_binds_event_chain(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    data = plan(tmp_path)

    def run(command: list[str], **_kwargs: object) -> SupervisedProcess:
        directory = Path(command[3]).parent
        for name in host_study.ARTIFACTS.values():
            (directory / name).write_bytes(b"{}")
        return SupervisedProcess(0, "completed", "", False, None)

    monkeypatch.setattr(host_study, "run_process", run)
    root = tmp_path / "study"
    report = host_study.collect(data, tmp_path, root)
    assert report["status"] == "ACCOUNTED"
    assert report["failed"] == report["excluded"] == []
    rows = (root / "events.jsonl").read_bytes().splitlines(keepends=True)
    altered = json.loads(rows[1])
    altered["payload"]["status"] = "excluded"
    rows[1] = host_perf.canonical(altered) + b"\n"
    (root / "events.jsonl").write_bytes(b"".join(rows))
    final = json.loads((root / "ledger.json").read_bytes())
    final["events_sha256"] = host_perf.sha256(b"".join(rows))
    (root / "ledger.json").write_bytes(host_perf.canonical(final))
    with pytest.raises(host_perf.ReceiptError, match="exclusion or unknown terminal"):
        host_study.verify(root)


@pytest.mark.parametrize("timeout", [None, True, 0, -1, 1.5, "1"])
def test_attempt_timeout_must_be_predeclared_positive_integer(
    tmp_path: Path, timeout: object
) -> None:
    value = json.loads(plan(tmp_path))
    value["attempts"][0]["timeout_seconds"] = timeout
    with pytest.raises(host_perf.ReceiptError):
        host_study.parse_plan(host_perf.canonical(value))


@pytest.mark.parametrize("role", ["scenario", "calibration"])
@pytest.mark.parametrize("path", ["../outside", "/absolute/input"])
def test_plan_inputs_cannot_escape_their_directory(tmp_path: Path, role: str, path: str) -> None:
    value = json.loads(plan(tmp_path))
    value["attempts"][0][role] = path
    with pytest.raises(host_perf.ReceiptError, match="under the plan directory"):
        host_study.parse_plan(host_perf.canonical(value))


def test_symlink_input_escape_is_accounted_as_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    inputs = tmp_path / "inputs"
    inputs.mkdir()
    data = plan(inputs)
    outside = tmp_path / "outside"
    outside.write_bytes(b"{}")
    (inputs / "scenario").unlink()
    (inputs / "scenario").symlink_to(outside)
    monkeypatch.setattr(host_study, "run_process", lambda *a, **k: pytest.fail("must not launch"))
    report = host_study.collect(data, inputs, tmp_path / "study")
    assert report["failed"] == ["baseline", "candidate"]
    assert all("escapes the plan directory" in a["reason"] for a in report["attempts"])


def test_fifo_input_cannot_block_before_execution_timeout(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    data = plan(tmp_path)
    (tmp_path / "scenario").unlink()
    os.mkfifo(tmp_path / "scenario")
    monkeypatch.setattr(host_study, "run_process", lambda *a, **k: pytest.fail("must not launch"))
    report = host_study.collect(data, tmp_path, tmp_path / "study")
    assert report["failed"] == ["baseline", "candidate"]
    assert all("regular file" in a["reason"] for a in report["attempts"])


def test_interruption_accounts_remaining_population_without_launching_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    calls = []

    def interrupted(*args: object, **kwargs: object) -> SupervisedProcess:
        calls.append(args)
        return SupervisedProcess(-signal.SIGTERM, "partial", "", False, signal.SIGTERM)

    monkeypatch.setattr(host_study, "run_process", interrupted)
    root = tmp_path / "study"
    report = host_study.collect(plan(tmp_path), tmp_path, root)
    assert len(calls) == 1
    assert report["failed"] == ["baseline", "candidate"]
    assert "interrupted by signal" in report["attempts"][0]["reason"]
    assert "not launched" in report["attempts"][1]["reason"]
    assert (root / "baseline/stdout").read_text() == "partial"


@pytest.mark.parametrize("leader_exits", [False, True])
def test_real_child_group_cannot_hang_or_false_pass_a_study(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, leader_exits: bool
) -> None:
    import psutil

    runner = tmp_path / "hung.py"
    runner.write_text(
        "import os, signal, subprocess, sys, time\n"
        "from pathlib import Path\n"
        "directory = Path(sys.argv[2]).parent\n"
        "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'], "
        "stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)\n"
        "(directory / 'child.pid').write_text(str(child.pid))\n"
        "print('partial output', flush=True)\n"
        + ("sys.exit(0)\n" if leader_exits else "time.sleep(60)\n")
    )
    real_run = host_study.run_process

    def redirected(command: list[str], **kwargs: object) -> SupervisedProcess:
        command[1] = str(runner)
        return real_run(command, **kwargs, termination_grace_seconds=0.1)

    monkeypatch.setattr(host_study, "run_process", redirected)
    root = tmp_path / "study"
    start = time.monotonic()
    report = host_study.collect(plan(tmp_path), tmp_path, root)
    assert time.monotonic() - start < 10
    assert report["failed"] == ["baseline", "candidate"]
    for attempt in report["attempts"]:
        directory = root / attempt["id"]
        assert (directory / "stdout").read_text() == "partial output\n"
        pid = int((directory / "child.pid").read_text())
        for _ in range(50):
            if not psutil.pid_exists(pid) or psutil.Process(pid).status() == psutil.STATUS_ZOMBIE:
                break
            time.sleep(0.02)
        else:
            os.kill(pid, signal.SIGKILL)
            pytest.fail("owned descendant still executing after terminal event")
    assert host_study.verify(root) == report
