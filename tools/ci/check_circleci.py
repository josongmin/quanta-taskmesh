"""Fail-closed checks for the bounded CircleCI migration contract."""

from __future__ import annotations

from typing import Any


def _run_commands(steps: object) -> list[str]:
    if not isinstance(steps, list):
        return []
    return [
        str(step["run"].get("command", ""))
        for step in steps
        if isinstance(step, dict) and isinstance(step.get("run"), dict)
    ]


def circleci_contract_problems(config: object, required: dict[str, Any]) -> list[str]:
    """Ensure CircleCI cannot silently narrow or bypass the 16-gate rail."""
    if not isinstance(config, dict):
        return ["CircleCI config is not an object"]
    problems: list[str] = []
    parameters = config.get("parameters")
    expected_parameters = {"run_fuzz", "run_load", "run_benchmark"}
    if (
        not isinstance(parameters, dict)
        or set(parameters) != expected_parameters
        or any(
            parameters.get(name) != {"type": "boolean", "default": False}
            for name in expected_parameters
        )
    ):
        problems.append("CircleCI heavy parameters must be three default-false booleans")
    ci_ids = set(required.get("required", [])) - set(required.get("nightly_required", []))
    if len(ci_ids) != 16:
        problems.append("CircleCI required profile must contain 16 gates")

    executors = config.get("executors", {})
    executor = executors.get("taskmesh-linux", {}) if isinstance(executors, dict) else {}
    env = executor.get("environment", {}) if isinstance(executor, dict) else {}
    if not isinstance(executor, dict) or not isinstance(executor.get("machine"), dict):
        problems.append("CircleCI requires a pinned Linux machine executor")
    if (
        executor.get("resource_class") != "medium"
        or not isinstance(env, dict)
        or any(
            env.get(name) != "2"
            for name in ("CARGO_BUILD_JOBS", "TASKMESH_BUILD_JOBS", "TASKMESH_TEST_JOBS")
        )
    ):
        problems.append("CircleCI medium executor must cap build and test workers at two")
    if env.get("UV_LOCKED") != "1" or env.get("UV_PYTHON") != "3.12.12":
        problems.append("CircleCI Python and lock policy drift")

    jobs = config.get("jobs", {})
    if not isinstance(jobs, dict) or set(jobs) != {"required", "fuzz", "load", "benchmark"}:
        return [*problems, "CircleCI job inventory drift"]
    fuzz_steps = jobs["fuzz"].get("steps", []) if isinstance(jobs["fuzz"], dict) else []
    if not any(
        "TASKMESH_FUZZ_TOOLCHAIN=nightly-2026-09-21 FUZZ_SECONDS=60 just fuzz" in command
        for command in _run_commands(fuzz_steps)
    ):
        problems.append("CircleCI fuzz campaign must use the pinned nightly")
    steps = jobs["required"].get("steps", []) if isinstance(jobs["required"], dict) else []
    if not isinstance(steps, list) or not steps or steps[0] != "checkout":
        return [*problems, "CircleCI required job must check out source first"]
    source = _run_commands(steps[:2])
    if source != ["set -euo pipefail\nbash tools/ci/checkout_verified_source.sh\n"]:
        problems.append("CircleCI source binding must run before dependencies")
    if [step for step in steps if step == "restore-dependencies"] != ["restore-dependencies"]:
        problems.append("CircleCI required job must restore dependency cache once")
    if [step for step in steps if step == "install-tools"] != ["install-tools"]:
        problems.append("CircleCI required job must install pinned tools once")
    if [step for step in steps if step == "save-dependencies"] != ["save-dependencies"]:
        problems.append("CircleCI required job must save dependency cache once")
    expected_gate = (
        "set -euo pipefail\n"
        "source target/ci/source.env\n"
        'test "$(git rev-parse HEAD)" = "$TASKMESH_EXPECTED_SHA"\n'
        "uv run python tools/gates/run.py --profile ci --require-clean-source "
        "--receipt target/verification/circleci-gates.json\n"
        "uv run python tools/gates/run.py --validate-receipt "
        'target/verification/circleci-gates.json --expected-head "$TASKMESH_EXPECTED_SHA"\n'
        "test -s target/verification/circleci-gates.json\n"
    )
    gate_commands = [command for command in _run_commands(steps) if "tools/gates/run.py" in command]
    if gate_commands != [expected_gate]:
        problems.append("CircleCI required job must enforce the exact complete CI receipt")
    if not any(
        isinstance(step, dict)
        and step.get("store_artifacts", {}).get("path") == "target/verification/circleci-gates.json"
        for step in steps
    ):
        problems.append("CircleCI required receipt artifact missing")

    commands = config.get("commands", {})
    if not isinstance(commands, dict):
        return [*problems, "CircleCI reusable commands missing"]
    install = commands.get("install-tools", {})
    install_steps = install.get("steps", []) if isinstance(install, dict) else []
    install_commands = _run_commands(install_steps)
    install_text = "\n".join(install_commands)
    for marker in (
        "rustup toolchain install 1.99.0",
        "rustup toolchain install 1.81.0",
        "0.9.104/linux",
        "--tag 1.58.0",
        "cargo-deny-0.19.7",
        "uv/0.9.11",
        'uv tool install "semgrep==$(cat tools/semgrep/version.txt)"',
        "uv sync --locked",
        "cargo fetch --locked",
        "cargo metadata --locked --manifest-path fuzz/Cargo.toml",
    ):
        if marker not in install_text:
            problems.append(f"CircleCI pinned tool or lock check missing: {marker}")
    for command in install_commands + _run_commands(steps):
        if (
            not command.startswith("set -euo pipefail\n")
            or "|| true" in command
            or "set +e" in command
        ):
            problems.append("CircleCI required shell may mask command failure")

    for name in ("restore-dependencies", "save-dependencies"):
        declaration = commands.get(name, {})
        entries = declaration.get("steps", []) if isinstance(declaration, dict) else []
        keys = [
            str(step.get("restore_cache", {}).get("keys", ""))
            + str(step.get("save_cache", {}).get("key", ""))
            for step in entries
            if isinstance(step, dict)
        ]
        if len(keys) != 2 or any("taskmesh-v1-linux-amd64-" not in key for key in keys):
            problems.append(f"CircleCI {name} must use isolated Taskmesh dependency keys")
        if any("target" in str(step) for step in entries):
            problems.append(f"CircleCI {name} must never cache build targets")

    workflows = config.get("workflows", {})
    if not isinstance(workflows, dict) or set(workflows) != {
        "regular",
        "manual-fuzz",
        "manual-load",
        "manual-benchmark",
    }:
        return [*problems, "CircleCI workflow inventory drift"]
    regular = workflows["regular"]
    if (
        not isinstance(regular, dict)
        or regular.get("unless")
        != {
            "or": [
                "<< pipeline.parameters.run_fuzz >>",
                "<< pipeline.parameters.run_load >>",
                "<< pipeline.parameters.run_benchmark >>",
            ]
        }
        or regular.get("jobs") != [{"required": {"filters": {"tags": {"ignore": "/.*/"}}}}]
    ):
        problems.append("CircleCI regular workflow must run only required job and ignore tags")
    for name, parameter, job in (
        ("manual-fuzz", "run_fuzz", "fuzz"),
        ("manual-load", "run_load", "load"),
        ("manual-benchmark", "run_benchmark", "benchmark"),
    ):
        if workflows[name] != {
            "when": f"<< pipeline.parameters.{parameter} >>",
            "jobs": [job],
        }:
            problems.append(f"CircleCI {name} must require its explicit parameter")
    return problems
