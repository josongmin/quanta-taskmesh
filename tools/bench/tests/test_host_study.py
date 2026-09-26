"""A failed planned attempt cannot disappear from the study denominator."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_perf  # noqa: E402
import host_study  # noqa: E402


def plan(root: Path) -> bytes:
    for name in ("scenario", "calibration"):
        (root / name).write_bytes(b"{}")
    return host_perf.canonical(
        {
            "schema_version": 1,
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
        host_study.subprocess,
        "run",
        lambda *args, **kwargs: subprocess.CompletedProcess([], 1, b"output", b"failure"),
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
        host_study.subprocess, "run", lambda *a, **k: pytest.fail("must reject before launch")
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
        host_study.subprocess,
        "run",
        lambda *args, **kwargs: subprocess.CompletedProcess([], 1, b"", b"failed"),
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

    def run(command: list[str], **_kwargs: object) -> subprocess.CompletedProcess:
        directory = Path(command[3]).parent
        for name in host_study.ARTIFACTS.values():
            (directory / name).write_bytes(b"{}")
        return subprocess.CompletedProcess(command, 0, b"completed", b"")

    monkeypatch.setattr(host_study.subprocess, "run", run)
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
