#!/usr/bin/env python3
"""Run V03 model checks and reject zero/partial/unreplayable exploration."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[2]
if str(REPO) not in sys.path:
    sys.path.insert(0, str(REPO))

from tools.qualification.evidence import (  # noqa: E402
    canonical_bytes,
    command_digest,
    envelope_problems,
    file_identity,
    runtime_action,
    tool_identity_digest,
)
from tools.qualification.receipt import source_identity  # noqa: E402

WITNESS = re.compile(r"taskmesh-model-witness (?P<fields>[^\r\n]+)")
REPLAY = re.compile(r"taskmesh-replay-witness (?P<fields>[^\r\n]+)")
REPLAY_GENERATION = re.compile(r"taskmesh-replay-generation (?P<fields>[^\r\n]+)")


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise ValueError(f"{path}: root must be an object")
    return value


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fields(text: str) -> dict[str, str]:
    parsed: dict[str, str] = {}
    for token in text.split():
        if "=" not in token:
            continue
        key, value = token.split("=", 1)
        parsed[key] = value
    return parsed


def parse_model_log(text: str, manifest: dict[str, Any]) -> tuple[list[dict[str, Any]], list[str]]:
    problems: list[str] = []
    records: list[dict[str, Any]] = []
    seen: set[tuple[str, str]] = set()
    for match in WITNESS.finditer(text):
        item = fields(match.group("fields"))
        checker, model_id = item.get("checker"), item.get("model_id")
        if checker not in {"loom", "shuttle"} or not model_id:
            problems.append("malformed model witness")
            continue
        identity = (checker, model_id)
        if identity in seen:
            problems.append(f"duplicate model witness {checker}/{model_id}")
            continue
        seen.add(identity)
        try:
            completed = int(item.get("completed", "0"))
        except ValueError:
            completed = 0
        if completed <= 0:
            problems.append(f"{model_id}: completed count must be positive")
        if checker == "loom":
            expected = manifest["checkers"]["loom"]
            if model_id not in expected["required_models"]:
                problems.append(f"loom: unknown model {model_id}")
            for key, expected_value in expected["bounds"].items():
                if item.get(key) != str(expected_value):
                    problems.append(f"{model_id}: {key} does not match manifest")
            records.append(
                {
                    "checker": checker,
                    "model_id": model_id,
                    **expected["bounds"],
                    "completed": completed,
                }
            )
        else:
            expected_models = manifest["checkers"]["shuttle"]["required_models"]
            expected = expected_models.get(model_id)
            if expected is None:
                problems.append(f"shuttle: unknown model {model_id}")
                continue
            try:
                seed = int(item.get("seed", "-1"))
                requested = int(item.get("requested", "0"))
            except ValueError:
                seed, requested = -1, 0
            if item.get("scheduler") != manifest["checkers"]["shuttle"]["scheduler"]:
                problems.append(f"{model_id}: scheduler does not match manifest")
            if seed != expected["seed"]:
                problems.append(f"{model_id}: seed does not match manifest")
            if requested != expected["requested"]:
                problems.append(f"{model_id}: requested count does not match manifest")
            if completed != requested:
                problems.append(f"{model_id}: completed {completed} of {requested} schedules")
            if item.get("max_steps") != str(manifest["checkers"]["shuttle"]["max_steps"]):
                problems.append(f"{model_id}: max_steps does not match manifest")
            records.append(
                {
                    "checker": checker,
                    "model_id": model_id,
                    "scheduler": item.get("scheduler"),
                    "seed": seed,
                    "requested": requested,
                    "completed": completed,
                    "max_steps": manifest["checkers"]["shuttle"]["max_steps"],
                }
            )

    actual_loom = {model for checker, model in seen if checker == "loom"}
    actual_shuttle = {model for checker, model in seen if checker == "shuttle"}
    required_loom = set(manifest["checkers"]["loom"]["required_models"])
    required_shuttle = set(manifest["checkers"]["shuttle"]["required_models"])
    if actual_loom != required_loom:
        actual_names = sorted(actual_loom)
        required_names = sorted(required_loom)
        problems.append(
            f"loom model set {actual_names} does not equal required set {required_names}"
        )
    if actual_shuttle != required_shuttle:
        actual_names = sorted(actual_shuttle)
        required_names = sorted(required_shuttle)
        problems.append(
            f"shuttle model set {actual_names} does not equal required set {required_names}"
        )
    return sorted(records, key=lambda record: (record["checker"], record["model_id"])), problems


def source_drift_problems(before: dict[str, Any], after: dict[str, Any]) -> list[str]:
    problems = []
    for key in ("head", "paths_digest", "dirty"):
        if before.get(key) != after.get(key):
            problems.append(f"source changed during model execution: {key}")
    return problems


def replay_artifact(
    replay_dir: Path, *, expected_sha256: str | None = None, relative_to: Path = REPO
) -> tuple[dict[str, str] | None, list[str]]:
    schedules = sorted(replay_dir.glob("schedule*.txt"))
    if len(schedules) != 1 or schedules[0].stat().st_size <= 0:
        return None, [f"expected one non-empty replay artifact, found {len(schedules)}"]
    artifact = {**file_identity(schedules[0], relative_to=relative_to), "role": "replay"}
    if expected_sha256 is not None and artifact["sha256"] != expected_sha256:
        return artifact, ["replay artifact changed between generation and replay"]
    return artifact, []


def parse_replay_logs(
    generation_text: str,
    replay_text: str,
    manifest: dict[str, Any],
    replay_dir: Path,
    generated_sha256: str | None,
) -> tuple[dict[str, Any] | None, list[str]]:
    problems: list[str] = []
    generation_matches = REPLAY_GENERATION.findall(generation_text)
    if len(generation_matches) != 1:
        problems.append(f"expected one replay generation witness, found {len(generation_matches)}")
    else:
        generated = fields(generation_matches[0])
        expected = manifest["replay"]
        if generated.get("model_id") != expected["model_id"]:
            problems.append("replay generation model_id does not match manifest")
        if generated.get("seed") != str(expected["seed"]):
            problems.append("replay generation seed does not match manifest")
        if generated.get("result") != "failure_persisted":
            problems.append("replay generation did not persist a failure")
    matches = REPLAY.findall(replay_text)
    if len(matches) != 1:
        problems.append(f"expected one replay witness, found {len(matches)}")
        return None, problems
    item = fields(matches[0])
    expected = manifest["replay"]
    if item.get("model_id") != expected["model_id"]:
        problems.append("replay model_id does not match manifest")
    if item.get("seed") != str(expected["seed"]):
        problems.append("replay seed does not match manifest")
    if item.get("result") != expected["result"]:
        problems.append("replay did not reproduce the same failure")
    artifact, artifact_problems = replay_artifact(replay_dir, expected_sha256=generated_sha256)
    problems.extend(artifact_problems)
    if artifact is None:
        return None, problems
    return {
        "model_id": expected["model_id"],
        "seed": expected["seed"],
        "result": item.get("result"),
        "artifact": artifact,
    }, problems


def exact_tool(name: str, command: list[str]) -> dict[str, str]:
    process = subprocess.run(command, cwd=REPO, capture_output=True, text=True, check=False)
    version = (process.stdout or process.stderr).strip()
    if process.returncode != 0 or not version:
        raise RuntimeError(f"cannot identify {name}: exit {process.returncode}")
    tool = {"name": name, "version": version}
    tool["identity_sha256"] = tool_identity_digest(tool)
    return tool


def run_command(
    argv: list[str],
    environment: dict[str, str],
    log_path: Path,
    *,
    timeout_seconds: float,
    termination_grace_seconds: float = 2.0,
) -> dict[str, Any]:
    if timeout_seconds <= 0:
        raise ValueError("model command timeout must be positive")
    process = subprocess.Popen(
        argv,
        cwd=REPO,
        env={**os.environ, **environment},
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        start_new_session=True,
    )
    timed_out = False
    try:
        stdout, stderr = process.communicate(timeout=timeout_seconds)
    except subprocess.TimeoutExpired:
        timed_out = True
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            stdout, stderr = process.communicate(timeout=termination_grace_seconds)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            stdout, stderr = process.communicate()
    text = stdout + stderr
    if timed_out:
        text += f"\ntaskmesh-model-process timed_out=true timeout_seconds={timeout_seconds}\n"
    log_path.write_text(text)
    sys.stderr.write(text)
    return {
        "exit_code": process.returncode,
        "signal": -process.returncode if process.returncode < 0 else None,
        "timed_out": timed_out,
        "timeout_seconds": timeout_seconds,
    }


def run_all(output_dir: Path) -> int:
    manifest_path = REPO / "tools/modelcheck/producer-manifest.json"
    manifest = load_json(manifest_path)
    timeout_seconds = manifest.get("command_timeout_seconds")
    if (
        not isinstance(timeout_seconds, int)
        or isinstance(timeout_seconds, bool)
        or timeout_seconds <= 0
    ):
        print("model command_timeout_seconds must be a positive integer", file=sys.stderr)
        return 1
    raw_dir = output_dir / "raw"
    replay_dir = output_dir / "replay"
    if output_dir.exists():
        shutil.rmtree(output_dir)
    raw_dir.mkdir(parents=True)
    replay_dir.mkdir(parents=True)
    started_at = utc_now()
    source_before = source_identity(REPO)

    commands = {
        "loom": (
            [
                "cargo",
                "test",
                "--locked",
                "-p",
                "taskmesh-engine",
                "--features",
                "loom",
                "--test",
                "loom_governance",
                "--release",
                "--",
                "--nocapture",
                "--test-threads=1",
            ],
            {"RUSTFLAGS": "--cfg loom"},
        ),
        "shuttle": (
            [
                "cargo",
                "test",
                "--locked",
                "-p",
                "taskmesh-engine",
                "--features",
                "shuttle",
                "--test",
                "shuttle_governance",
                "--release",
                "--",
                "--nocapture",
                "--test-threads=1",
            ],
            {"RUSTFLAGS": "--cfg shuttle", "TASKMESH_MODEL_FAILURE_DIR": str(replay_dir)},
        ),
        "replay_generate": (
            [
                "cargo",
                "test",
                "--locked",
                "-p",
                "taskmesh-engine",
                "--features",
                "shuttle",
                "--test",
                "model_replay_fixture",
                "--release",
                "--",
                "--nocapture",
                "--test-threads=1",
            ],
            {
                "RUSTFLAGS": "--cfg shuttle",
                "TASKMESH_MODEL_FAILURE_DIR": str(replay_dir),
                "TASKMESH_MODEL_REPLAY_MODE": "generate",
            },
        ),
    }
    processes: dict[str, dict[str, Any]] = {}
    for name, (argv, environment) in commands.items():
        processes[name] = run_command(
            argv,
            environment,
            raw_dir / f"{name}.log",
            timeout_seconds=timeout_seconds,
        )

    generated_artifact, generation_artifact_problems = replay_artifact(replay_dir)
    generated_sha256 = generated_artifact["sha256"] if generated_artifact is not None else None
    replay_argv = commands["replay_generate"][0]
    replay_environment = {
        "RUSTFLAGS": "--cfg shuttle",
        "TASKMESH_MODEL_FAILURE_DIR": str(replay_dir),
        "TASKMESH_MODEL_REPLAY_MODE": "replay",
    }
    processes["replay_verify"] = run_command(
        replay_argv,
        replay_environment,
        raw_dir / "replay_verify.log",
        timeout_seconds=timeout_seconds,
    )

    combined = (raw_dir / "loom.log").read_text() + (raw_dir / "shuttle.log").read_text()
    models, problems = parse_model_log(combined, manifest)
    replay, replay_problems = parse_replay_logs(
        (raw_dir / "replay_generate.log").read_text(),
        (raw_dir / "replay_verify.log").read_text(),
        manifest,
        replay_dir,
        generated_sha256,
    )
    problems.extend(generation_artifact_problems)
    problems.extend(replay_problems)
    for name, process in processes.items():
        if process["timed_out"]:
            problems.append(f"{name} process timed out after {timeout_seconds} seconds")
        elif process["exit_code"] != 0:
            problems.append(f"{name} process exited {process['exit_code']}")
    source_after = source_identity(REPO)
    source_problems = source_drift_problems(source_before, source_after)
    problems.extend(source_problems)
    finished_at = utc_now()
    status = "PASS" if not problems else "FAIL"
    receipt = {
        "schema_version": 1,
        "kind": "taskmesh-modelcheck-evidence",
        "producer_id": manifest["producer_id"],
        "models": models,
        "replay": replay,
        "processes": processes,
        "source_stable": not source_problems,
        "status": status,
        "problems": problems,
    }
    receipt_path = output_dir / "model-receipt.json"
    receipt_path.write_bytes(canonical_bytes(receipt) + b"\n")

    argv = ["python3", "tools/modelcheck/run.py", "all"]
    command: dict[str, Any] = {
        "argv": argv,
        "cwd": ".",
        "environment": {"RUSTFLAGS": "checker-specific; see receipt"},
    }
    command["sha256"] = command_digest(command)
    artifacts = [
        *(
            {**file_identity(raw_dir / f"{name}.log", relative_to=REPO), "role": "raw"}
            for name in processes
        ),
        {**file_identity(receipt_path, relative_to=REPO), "role": "summary"},
    ]
    if replay is not None:
        artifacts.append(replay["artifact"])
    envelope = {
        "schema_version": 1,
        "kind": "taskmesh-evidence-envelope",
        "producer": {"id": manifest["producer_id"], "version": "1"},
        "source": {
            "head": source_before["head"],
            "tree_digest": source_before["paths_digest"],
            "dirty": source_before["dirty"],
        },
        "command": command,
        "tools": [exact_tool("cargo", ["cargo", "-V"]), exact_tool("rustc", ["rustc", "-Vv"])],
        "configs": [
            {"path": manifest_path.relative_to(REPO).as_posix(), "sha256": sha256(manifest_path)},
            {"path": "Cargo.lock", "sha256": sha256(REPO / "Cargo.lock")},
            {
                "path": "crates/taskmesh-engine/tests/loom_governance.rs",
                "sha256": sha256(REPO / "crates/taskmesh-engine/tests/loom_governance.rs"),
            },
            {
                "path": "crates/taskmesh-engine/tests/shuttle_governance.rs",
                "sha256": sha256(REPO / "crates/taskmesh-engine/tests/shuttle_governance.rs"),
            },
            {
                "path": "crates/taskmesh-engine/tests/model_replay_fixture.rs",
                "sha256": sha256(REPO / "crates/taskmesh-engine/tests/model_replay_fixture.rs"),
            },
        ],
        "action": runtime_action(
            root=REPO,
            source_head=source_before["head"],
            local_workflow="tools/modelcheck/producer-manifest.json",
            local_job="local-modelcheck",
        ),
        "artifacts": artifacts,
        "result": {
            "status": status,
            "exit_code": 0 if status == "PASS" else 1,
            "started_at": started_at,
            "finished_at": finished_at,
            "selected_count": len(manifest["checkers"]["loom"]["required_models"])
            + len(manifest["checkers"]["shuttle"]["required_models"]),
            "executed_count": len(models),
        },
    }
    envelope_path = output_dir / "evidence-envelope.json"
    envelope_path.write_bytes(canonical_bytes(envelope) + b"\n")
    envelope_issues = envelope_problems(envelope, artifact_root=REPO)
    if envelope_issues:
        problems.extend(f"envelope: {problem}" for problem in envelope_issues)
    if problems:
        print("model evidence rejected: " + "; ".join(problems), file=sys.stderr)
        return 1
    relative_receipt = receipt_path.relative_to(REPO)
    print(f"taskmesh-modelcheck status=PASS models={len(models)} receipt={relative_receipt}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=["all", "validate-manifest"])
    parser.add_argument("--output-dir", type=Path, default=REPO / "target/modelcheck")
    args = parser.parse_args()
    manifest = load_json(REPO / "tools/modelcheck/producer-manifest.json")
    if args.command == "validate-manifest":
        timeout_seconds = manifest.get("command_timeout_seconds")
        if (
            not manifest["checkers"]["loom"]["required_models"]
            or not manifest["checkers"]["shuttle"]["required_models"]
            or not isinstance(timeout_seconds, int)
            or isinstance(timeout_seconds, bool)
            or timeout_seconds <= 0
        ):
            print("model manifest has empty models or invalid command timeout", file=sys.stderr)
            return 1
        return 0
    output_dir = args.output_dir.resolve()
    expected_root = (REPO / "target").resolve()
    if expected_root not in output_dir.parents:
        print("model output directory must be below repository target/", file=sys.stderr)
        return 1
    return run_all(output_dir)


if __name__ == "__main__":
    raise SystemExit(main())
