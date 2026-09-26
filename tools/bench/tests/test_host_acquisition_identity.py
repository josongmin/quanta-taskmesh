"""A built executable must never acquire another source's execution identity."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import generator_run  # noqa: E402
import host_perf  # noqa: E402
import host_run  # noqa: E402


@pytest.mark.parametrize("mode", ["full", "generator", "minimal"])
@pytest.mark.parametrize("changed_field", ["source_head", "source_content_sha256"])
def test_prelaunch_drift_rejects_before_process_creation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, mode: str, changed_field: str
) -> None:
    scenario = tmp_path / "scenario.json"
    scenario.write_text("{}")
    calibration = tmp_path / "calibration.json"
    calibration.write_text("{}")
    binary = tmp_path / "binary"
    binary.write_bytes(b"sealed executable")
    binary.chmod(0o755)
    raw = tmp_path / "raw.json"
    baseline = {"source_head": "A", "source_content_sha256": "A", "source_dirty": True}
    identities = iter([baseline, baseline, {**baseline, changed_field: "B"}])
    monkeypatch.setattr(host_perf, "local_identity", lambda *_args: next(identities))
    module = host_run if mode == "full" else generator_run
    monkeypatch.setattr(
        module, "build_runner", lambda *_args, **_kwargs: (binary, [], ["mock build"])
    )

    def unexpected_launch(*_args: object, **_kwargs: object) -> None:
        pytest.fail("source drift must reject before creating a child process")

    monkeypatch.setattr(module, "sample_subprocess", unexpected_launch)
    argv = ["runner", str(scenario), str(raw)]
    if mode == "full":
        argv.extend([str(tmp_path / "summary.json"), str(calibration)])
    monkeypatch.setattr(sys, "argv", argv)
    options = (
        {
            "example_name": "host_load_probe",
            "raw_kind": "minimal",
            "runner_flag": "--recorder-minimal",
        }
        if mode == "minimal"
        else {}
    )
    assert module.main(**options) == 1
    assert not raw.exists()
    assert not raw.with_name(raw.name + ".provenance.json").exists()
    rejection = json.loads(raw.with_name(raw.name + ".rejection.json").read_bytes())
    assert rejection["status"] == "rejected"
    assert rejection["phase"] == "before_launch"
    assert "between build and launch" in rejection["reason"]


def source_repo(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    def git(*args: str) -> None:
        subprocess.run(["git", *args], cwd=tmp_path, check=True, capture_output=True)

    git("init", "-q")
    git("config", "user.name", "custody-test")
    git("config", "user.email", "custody-test@example.invalid")
    (tmp_path / "Cargo.lock").write_text("lock")
    source = tmp_path / "probe.rs"
    source.write_text("committed source")
    git("add", "Cargo.lock", "probe.rs")
    git("commit", "-qm", "fixture")
    monkeypatch.setattr(host_perf, "REPO", tmp_path)
    return source


@pytest.mark.parametrize("flag", ["--assume-unchanged", "--skip-worktree"])
def test_source_identity_rejects_index_hint_hidden_edits(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, flag: str
) -> None:
    source = source_repo(tmp_path, monkeypatch)
    clean = host_perf.source_identity()
    assert clean["source_dirty"] is False
    subprocess.run(["git", "update-index", flag, "probe.rs"], cwd=tmp_path, check=True)
    source.write_text("hidden edit")
    assert host_perf._git("status", "--porcelain=v1") == ""
    changed = host_perf.source_identity()
    assert changed["source_dirty"] is True
    assert changed["source_content_sha256"] != clean["source_content_sha256"]


def test_source_identity_distinguishes_dirty_source_contents(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    source = source_repo(tmp_path, monkeypatch)
    source.write_text("dirty A")
    first = host_perf.source_identity()
    source.write_text("dirty B")
    second = host_perf.source_identity()
    assert first["source_dirty"] is second["source_dirty"] is True
    assert first["source_head"] == second["source_head"]
    assert first["source_content_sha256"] != second["source_content_sha256"]


def test_source_identity_unavailable_git_rejects(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(host_perf, "REPO", tmp_path)
    with pytest.raises(host_perf.ReceiptError, match="source custody inspection failed"):
        host_perf.source_identity()
