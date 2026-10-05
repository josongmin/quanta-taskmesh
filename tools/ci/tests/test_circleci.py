"""CircleCI must retain the exact required rail and opt-in heavy work."""

from __future__ import annotations

import copy
import json
import os
import subprocess
from pathlib import Path

import yaml

from tools.ci.check_circleci import circleci_contract_problems

ROOT = Path(__file__).resolve().parents[3]


def inputs() -> tuple[dict, dict]:
    config = yaml.safe_load((ROOT / ".circleci/config.yml").read_text(encoding="utf-8"))
    required = json.loads((ROOT / "tools/gates/required.json").read_text(encoding="utf-8"))
    return config, required


def test_circleci_contract_is_complete() -> None:
    config, required = inputs()
    assert circleci_contract_problems(config, required) == []


def test_heavy_default_and_gate_mutilation_fail_closed() -> None:
    config, required = inputs()
    invalid = copy.deepcopy(config)
    invalid["parameters"]["run_fuzz"]["default"] = True
    assert any("default-false" in item for item in circleci_contract_problems(invalid, required))

    invalid = copy.deepcopy(config)
    gate = next(
        step["run"]
        for step in invalid["jobs"]["required"]["steps"]
        if isinstance(step, dict)
        and "run" in step
        and "tools/gates/run.py" in step["run"].get("command", "")
    )
    gate["command"] = gate["command"].replace("--profile ci", "--profile nightly")
    assert any(
        "complete CI receipt" in item for item in circleci_contract_problems(invalid, required)
    )

    invalid = copy.deepcopy(config)
    invalid["workflows"]["manual-load"].pop("when")
    assert any(
        "explicit parameter" in item for item in circleci_contract_problems(invalid, required)
    )

    invalid = copy.deepcopy(config)
    invalid["jobs"]["required"]["steps"][1]["run"]["command"] = "echo bind source"
    assert any("source binding" in item for item in circleci_contract_problems(invalid, required))


def test_circleci_bash_env_preserves_child_path_shims(tmp_path: Path) -> None:
    config, _ = inputs()
    install = config["commands"]["install-tools"]["steps"][0]["run"]["command"]
    bootstrap = next(
        line for line in install.splitlines()
        if line.startswith("printf '%s\\n' ") and line.endswith(' >> "$BASH_ENV"')
    )
    bash_env = tmp_path / "bash.env"
    bash_env.touch()
    home = tmp_path / "home"
    cargo_bin = home / ".cargo" / "bin"
    cargo_bin.mkdir(parents=True)
    (home / ".local" / "bin").mkdir(parents=True)
    shim_bin = tmp_path / "shims"
    shim_bin.mkdir()
    for directory in (cargo_bin, shim_bin):
        tool = directory / "cargo"
        tool.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        tool.chmod(0o755)
    env = os.environ | {
        "HOME": str(home),
        "BASH_ENV": str(bash_env),
        "SHIM_BIN": str(shim_bin),
    }
    env.pop("TASKMESH_CI_TOOLS_ON_PATH", None)
    setup = subprocess.run(
        ["bash", "-c", bootstrap], env=env, capture_output=True, text=True, check=False
    )
    assert setup.returncode == 0, setup.stderr
    result = subprocess.run(
        [
            "bash",
            "-c",
            "command -v cargo; PATH=\"$SHIM_BIN:$PATH\" bash -c 'command -v cargo'",
        ],
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    assert result.stdout.splitlines() == [str(cargo_bin / "cargo"), str(shim_bin / "cargo")]
