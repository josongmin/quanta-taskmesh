"""Acquisition deadlines terminate actual hung children and retain failure evidence."""

from __future__ import annotations

import json
import os
import shlex
import shutil
import sys
import time
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import acquisition_process  # noqa: E402
import generator_run  # noqa: E402
import host_perf  # noqa: E402
import host_run  # noqa: E402
import host_special_run  # noqa: E402
from acquisition_process import SupervisedProcess  # noqa: E402

from tools.bench.tests.test_host_special import receipt_directory  # noqa: E402


def hanging_program(path: Path) -> Path:
    ready = path.with_name(path.name + ".ready")
    path.write_text(
        "#!/bin/sh\n# fixture-ready: "
        + json.dumps(str(ready))
        + "\nprintf 'retained partial stdout\\n'\n"
        + f"printf 'ready' > {shlex.quote(str(ready))}\nexec /bin/sleep 60\n"
    )
    path.chmod(0o755)
    return path


@pytest.fixture(autouse=True)
def fixture_output_ready_before_execution_deadline(monkeypatch: pytest.MonkeyPatch) -> None:
    run_process = acquisition_process.run_process

    def after_partial_output(command, **kwargs):
        environment = kwargs.get("env", os.environ)
        program = shutil.which(command[0], path=environment.get("PATH"))
        try:
            with Path(program or command[0]).open("rb") as stream:
                header = stream.read(4096)
        except OSError:
            return run_process(command, **kwargs)
        prefix = b"# fixture-ready: "
        markers = [line[len(prefix) :] for line in header.splitlines() if line.startswith(prefix)]
        if not markers:
            return run_process(command, **kwargs)
        ready = Path(json.loads(markers[0]))
        ready.unlink(missing_ok=True)
        original_started = kwargs.get("on_started")

        def started(process):
            if original_started is not None:
                original_started(process)
            # Establish emitted bytes before testing their retention. Production
            # may legitimately time out a child before it emits any output.
            startup_deadline = time.monotonic() + 5
            while not ready.exists() and time.monotonic() < startup_deadline:
                time.sleep(0.01)
            assert ready.exists(), "fixture did not reach its partial-output stage"

        kwargs["on_started"] = started
        return run_process(command, **kwargs)

    monkeypatch.setattr(acquisition_process, "run_process", after_partial_output)


def test_standalone_build_hang_has_a_terminal_record(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    hanging_program(tmp_path / "cargo")
    monkeypatch.setenv("PATH", str(tmp_path) + os.pathsep + os.environ["PATH"])
    prefix = tmp_path / "build"
    started = time.monotonic()
    with pytest.raises(host_perf.ReceiptError, match="build failed"):
        host_run.build_runner([], timeout_seconds=2.0, execution_path=prefix)
    assert time.monotonic() - started < 10
    terminal = json.loads((tmp_path / "build.execution.json").read_bytes())
    assert terminal["status"] == "failed"
    assert terminal["timed_out"] is True
    assert terminal["timeout_seconds"] == 2.0
    assert "retained partial stdout" in (tmp_path / "build.stdout").read_text()


@pytest.mark.parametrize("failure", ["timed_out", "interrupted", "aborted"])
def test_zero_exit_cannot_hide_incomplete_build(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, failure: str
) -> None:
    result = SupervisedProcess(
        0,
        "partial",
        "",
        failure == "timed_out",
        2 if failure == "interrupted" else None,
        failure == "aborted",
    )
    monkeypatch.setattr(host_run, "run_acquisition", lambda *_a, **_kw: result)
    with pytest.raises(host_perf.ReceiptError, match="build failed"):
        host_run.build_runner([], execution_path=tmp_path / "build")
    assert json.loads((tmp_path / "build.execution.json").read_bytes())["status"] == "failed"


def test_retained_special_validator_hang_rejects_with_partial_logs(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    directory = receipt_directory(tmp_path)
    hanging_program(directory / "validator")
    receipt = json.loads((directory / "receipt.json").read_bytes())
    receipt["validator_sha256"] = host_perf.sha256((directory / "validator").read_bytes())
    (directory / "receipt.json").write_text(json.dumps(receipt))
    monkeypatch.setattr(host_perf, "validate_identity", lambda _identity: None)
    monkeypatch.setattr(host_perf, "validate_resource_artifact", lambda _data, _pid: None)
    with pytest.raises(host_perf.ReceiptError, match="typed special raw/topology rejected"):
        host_special_run.verify_receipt(directory, timeout_seconds=2.0)
    terminal = next(directory.glob("validator-failure-*/validator.execution.json"))
    assert json.loads(terminal.read_bytes())["timed_out"] is True
    assert "retained partial stdout" in terminal.with_name("validator.stdout").read_text()


@pytest.mark.parametrize("mode", ["full", "generator", "minimal"])
def test_probe_hang_rejects_and_retains_resource_terminal_and_partial_logs(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, mode: str
) -> None:
    scenario, calibration, raw = (tmp_path / name for name in ("scenario", "calibration", "raw"))
    scenario.write_text("{}")
    calibration.write_text("{}")
    binary = hanging_program(tmp_path / "binary")
    identity = {"source_head": "fixture", "source_dirty": False}
    monkeypatch.setattr(host_perf, "local_identity", lambda *_a: identity)
    module = host_run if mode == "full" else generator_run
    monkeypatch.setattr(module, "build_runner", lambda *_a, **_kw: (binary, [], ["fake build"]))
    argv = ["runner", str(scenario), str(raw)]
    if mode == "full":
        argv += [str(tmp_path / "summary"), str(calibration), "--no-resource-sampling"]
    argv += ["--probe-timeout-seconds", "2.0"]
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
    assert not (tmp_path / "summary").exists()
    resources = json.loads((tmp_path / "raw.resources.json").read_bytes())
    assert resources["execution"]["timed_out"] is True
    assert resources["execution"]["timeout_seconds"] == 2.0
    assert "retained partial stdout" in (tmp_path / "raw.probe.stderr").read_text()
    assert json.loads((tmp_path / "raw.provenance.json").read_bytes())["status"] == "invalid"
    assert json.loads((tmp_path / "raw.rejection.json").read_bytes())["phase"] == "run"


@pytest.mark.parametrize("mode", ["full", "generator", "minimal"])
def test_declared_validator_deadline_reaches_actual_child_and_retains_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, mode: str
) -> None:
    hanging_program(tmp_path / "cargo")
    monkeypatch.setenv("PATH", str(tmp_path) + os.pathsep + os.environ["PATH"])
    scenario, calibration, raw = (tmp_path / name for name in ("scenario", "calibration", "raw"))
    scenario.write_text("{}")
    calibration.write_text("{}")
    binary = hanging_program(tmp_path / "binary")
    identity = {"source_head": "fixture", "source_dirty": False, "features": []}
    monkeypatch.setattr(host_perf, "local_identity", lambda *_a: identity)
    monkeypatch.setattr(host_perf, "validate_identity", lambda _identity: None)
    module = host_run if mode == "full" else generator_run
    monkeypatch.setattr(module, "build_runner", lambda *_a, **_kw: (binary, [], ["fake build"]))

    def completed(command: list[str], **_kwargs: object) -> tuple:
        Path(command[2]).write_text('{"status":{"kind":"complete"}}')
        Path(command[4]).write_text("{}")
        return (
            0,
            123,
            "",
            {
                "execution": {
                    "timeout_seconds": 1800,
                    "returncode": 0,
                    "timed_out": False,
                    "interrupted_by_signal": None,
                    "aborted_early": False,
                }
            },
        )

    monkeypatch.setattr(module, "sample_subprocess", completed)
    argv = ["runner", str(scenario), str(raw)]
    if mode == "full":
        argv += [str(tmp_path / "summary"), str(calibration)]
    monkeypatch.setattr(sys, "argv", [*argv, "--validator-timeout-seconds", "2"])
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
    terminal = json.loads((tmp_path / "raw.validation.execution.json").read_bytes())
    assert terminal["timed_out"] is True
    assert terminal["timeout_seconds"] == 2
    assert "retained partial stdout" in (tmp_path / "raw.validation.stdout").read_text()
    assert json.loads((tmp_path / "raw.provenance.json").read_bytes())["status"] == "invalid"
    assert json.loads((tmp_path / "raw.rejection.json").read_bytes())["phase"] == "validation"
    assert not (tmp_path / "summary").exists()


@pytest.mark.parametrize("phase", ["probe", "validator"])
def test_special_acquisition_hangs_keep_invalid_receipt_and_failure_logs(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, phase: str
) -> None:
    scenario = tmp_path / "scenario"
    scenario.write_text('{"topology":{}}')
    hanging = hanging_program(tmp_path / "hanging")
    runner = tmp_path / "runner"
    runner.write_text('#!/bin/sh\nprintf "{}" > "$2"\nprintf "{}" > "$3"\n')
    runner.chmod(0o755)
    monkeypatch.setattr(host_perf, "local_identity", lambda *_a: {"source_head": "fixture"})
    monkeypatch.setattr(host_special_run, "source_content_sha256", lambda: "a" * 64)
    monkeypatch.setattr(
        host_run,
        "build_runner",
        lambda _features, example, **_kw: (
            hanging if phase == "probe" or example == "host_special_validate" else runner,
            ["default"],
            ["fake build"],
        ),
    )
    directory = tmp_path / "output"
    with pytest.raises(
        host_perf.ReceiptError,
        match=f"special {phase if phase == 'validator' else 'runner'} failed",
    ):
        host_special_run.acquire(
            "local",
            scenario,
            directory,
            [],
            probe_timeout_seconds=2.0,
            validator_timeout_seconds=2.0,
        )
    assert json.loads((directory / "receipt.json").read_bytes())["status"] == "invalid"
    if phase == "probe":
        assert json.loads((directory / "resources").read_bytes())["execution"]["timed_out"] is True
        assert "retained partial stdout" in (directory / "probe.stderr").read_text()
    else:
        assert (
            json.loads((directory / "validation.execution.json").read_bytes())["timed_out"] is True
        )
        assert "retained partial stdout" in (directory / "validation.stdout").read_text()


@pytest.mark.parametrize("module", [host_run, generator_run, host_special_run])
@pytest.mark.parametrize("value", ["0", "-1", "nan", "inf"])
@pytest.mark.parametrize("phase", ["build", "validator", "probe"])
def test_cli_deadline_must_be_positive_finite(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, module: object, value: str, phase: str
) -> None:
    if module is host_special_run:
        argv = ["special", "local", "scenario", str(tmp_path / "output")]
    else:
        argv = ["runner", "scenario", "raw"]
        if module is host_run:
            argv += ["summary", "calibration"]
    monkeypatch.setattr(sys, "argv", [*argv, f"--{phase}-timeout-seconds", value])
    with pytest.raises(SystemExit) as error:
        module.main()
    assert error.value.code == 2
