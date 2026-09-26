"""A built executable must never acquire another source's execution identity."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import generator_run  # noqa: E402
import host_perf  # noqa: E402
import host_run  # noqa: E402


@pytest.mark.parametrize("mode", ["full", "generator", "minimal"])
def test_prelaunch_drift_rejects_before_process_creation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, mode: str
) -> None:
    scenario = tmp_path / "scenario.json"
    scenario.write_text("{}")
    calibration = tmp_path / "calibration.json"
    calibration.write_text("{}")
    binary = tmp_path / "binary"
    binary.write_bytes(b"sealed executable")
    binary.chmod(0o755)
    raw = tmp_path / "raw.json"
    identities = iter([{"source_head": "A"}, {"source_head": "A"}, {"source_head": "B"}])
    monkeypatch.setattr(host_perf, "local_identity", lambda *_args: next(identities))
    module = host_run if mode == "full" else generator_run
    monkeypatch.setattr(module, "build_runner", lambda *_args: (binary, [], ["mock build"]))

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
