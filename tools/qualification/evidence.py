#!/usr/bin/env python3
"""Canonical, product-neutral EvidenceEnvelopeV1 validation."""

from __future__ import annotations

import hashlib
import json
import os
import re
import stat
import subprocess
from collections.abc import Iterable
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

SCHEMA_VERSION = 1
KIND = "taskmesh-evidence-envelope"
SCHEMA = Path(__file__).with_name("evidence-envelope-v1.schema.json")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
GIT_SHA = re.compile(r"^[0-9a-f]{40}$")
PINNED_ACTION = re.compile(r"^[^\s@]+@[0-9a-f]{40}$")
STATUSES = {"PASS", "FAIL", "NOT_RUN", "SKIPPED", "TIMEOUT", "BASELINE_CREATED"}
MUTABLE_TOOL_VERSION = re.compile(r"^(?:stable|latest|nightly|master|main|head)$", re.IGNORECASE)
RFC3339_UTC = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|\+00:00)$")
WORKFLOW_REF = re.compile(r"^[^/\s]+/[^/\s]+/(?P<path>\.github/workflows/[^@\s]+)@(?P<ref>\S+)$")


def canonical_bytes(value: object) -> bytes:
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False
    ).encode()


def canonical_digest(value: object) -> str:
    return hashlib.sha256(canonical_bytes(value)).hexdigest()


def file_identity(path: Path, *, relative_to: Path) -> dict[str, object]:
    data = path.read_bytes()
    return {
        "path": path.relative_to(relative_to).as_posix(),
        "sha256": hashlib.sha256(data).hexdigest(),
        "size": len(data),
    }


def command_digest(command: dict[str, object]) -> str:
    return canonical_digest(
        {key: command[key] for key in ("argv", "cwd", "environment") if key in command}
    )


def tool_identity_digest(tool: dict[str, object]) -> str:
    return canonical_digest({key: tool[key] for key in ("name", "version") if key in tool})


def git_source_paths(root: Path) -> list[str]:
    """Return tracked and non-ignored untracked source paths in stable order."""
    process = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=root,
        capture_output=True,
        check=False,
    )
    if process.returncode != 0:
        detail = process.stderr.decode(errors="replace").strip()
        raise RuntimeError(f"git ls-files failed: {detail}")
    return sorted(path.decode("utf-8") for path in process.stdout.split(b"\0") if path)


def source_tree_digest(root: Path, paths: Iterable[str]) -> str:
    """Digest path, file kind, executable bit, and bytes for an explicit source set."""
    digest = hashlib.sha256()
    for relative in sorted(paths):
        path = root / relative
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        if path.is_symlink():
            digest.update(b"symlink\0")
            digest.update(os.readlink(path).encode("utf-8"))
        elif path.is_file():
            digest.update(b"file\0")
            executable = path.stat().st_mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
            digest.update(b"x" if executable else b"-")
            digest.update(path.read_bytes())
        else:
            digest.update(b"missing\0")
    return digest.hexdigest()


def runtime_action(
    *, root: Path, source_head: str, local_workflow: str, local_job: str
) -> dict[str, object]:
    """Bind evidence to the actual hosted job, or to an explicit local context."""
    root = root.resolve()
    if os.environ.get("GITHUB_ACTIONS") == "true":
        values = {
            name: os.environ.get(name, "")
            for name in ("GITHUB_WORKFLOW_REF", "GITHUB_JOB", "GITHUB_EVENT_NAME", "GITHUB_SHA")
        }
        missing = [name for name, value in values.items() if not value]
        if missing:
            raise RuntimeError(f"hosted action identity is missing {missing}")
        match = WORKFLOW_REF.fullmatch(values["GITHUB_WORKFLOW_REF"])
        if match is None:
            raise RuntimeError("GITHUB_WORKFLOW_REF is malformed")
        if values["GITHUB_SHA"] != source_head:
            raise RuntimeError("GITHUB_SHA does not match producer source HEAD")
        workflow_path = match.group("path")
        workflow = root / workflow_path
        if not workflow.is_file():
            raise RuntimeError(f"hosted workflow {workflow_path!r} is missing")
        return {
            "context": "github-actions",
            "workflow": workflow_path,
            "workflow_ref": values["GITHUB_WORKFLOW_REF"],
            "workflow_sha256": hashlib.sha256(workflow.read_bytes()).hexdigest(),
            "source_sha": values["GITHUB_SHA"],
            "job": values["GITHUB_JOB"],
            "event": values["GITHUB_EVENT_NAME"],
            "actions": [],
        }

    workflow = Path(local_workflow)
    if workflow.is_absolute() or ".." in workflow.parts or not local_job:
        raise ValueError("local action workflow/job must be explicit and repo-relative")
    disk = root / workflow
    if not disk.is_file():
        raise ValueError(f"local action workflow {local_workflow!r} is missing")
    return {
        "context": "local",
        "workflow": workflow.as_posix(),
        "workflow_ref": f"local:{workflow.as_posix()}",
        "workflow_sha256": hashlib.sha256(disk.read_bytes()).hexdigest(),
        "source_sha": source_head,
        "job": local_job,
        "event": "local",
        "actions": [],
    }


def _object(value: object, name: str, problems: list[str]) -> dict[str, Any]:
    if not isinstance(value, dict):
        problems.append(f"{name} is missing or is not an object")
        return {}
    return value


def _required(obj: dict[str, Any], keys: tuple[str, ...], name: str, problems: list[str]) -> None:
    for key in keys:
        if key not in obj:
            problems.append(f"{name}.{key} is required")


def _sha(value: object, name: str, problems: list[str]) -> None:
    if not isinstance(value, str) or not SHA256.fullmatch(value):
        problems.append(f"{name} must be a lowercase sha256 digest")


def _safe_relative(value: object) -> bool:
    return (
        isinstance(value, str)
        and bool(value)
        and not Path(value).is_absolute()
        and ".." not in Path(value).parts
    )


def _utc_timestamp(value: object, name: str, problems: list[str]) -> datetime | None:
    if not isinstance(value, str) or not RFC3339_UTC.fullmatch(value):
        problems.append(f"{name} must be an RFC3339 UTC timestamp")
        return None
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        problems.append(f"{name} must be an RFC3339 UTC timestamp")
        return None
    if parsed.tzinfo is None or parsed.utcoffset() != timezone.utc.utcoffset(parsed):
        problems.append(f"{name} must use UTC")
        return None
    return parsed


def _verify_file_identity(
    *, root: Path, path: str, expected_sha256: object, name: str, problems: list[str]
) -> None:
    disk = _confined_regular_file(root=root, path=path, name=name, problems=problems)
    if disk is not None and hashlib.sha256(disk.read_bytes()).hexdigest() != expected_sha256:
        problems.append(f"{name} {path!r} digest mismatch")


def _confined_regular_file(*, root: Path, path: str, name: str, problems: list[str]) -> Path | None:
    """Resolve an evidence path without following links outside its artifact root."""
    if not _safe_relative(path):
        return None
    disk = root / path
    if disk.is_symlink():
        problems.append(f"{name} {path!r} must not be a symlink")
        return None
    try:
        resolved_root = root.resolve(strict=True)
        resolved = disk.resolve(strict=True)
        resolved.relative_to(resolved_root)
    except FileNotFoundError:
        problems.append(f"{name} {path!r} is missing")
        return None
    except ValueError:
        problems.append(f"{name} {path!r} escapes artifact root")
        return None
    if not resolved.is_file():
        problems.append(f"{name} {path!r} is not a regular file")
        return None
    return resolved


def envelope_problems(
    envelope: object,
    *,
    artifact_root: Path | None = None,
    expected_source: dict[str, object] | None = None,
) -> list[str]:
    """Validate declarations and, when given a root, re-hash every artifact."""
    problems: list[str] = []
    root = _object(envelope, "envelope", problems)
    if not root:
        return problems
    top_keys = (
        "schema_version",
        "kind",
        "producer",
        "source",
        "command",
        "tools",
        "configs",
        "action",
        "artifacts",
        "result",
    )
    _required(root, top_keys, "envelope", problems)
    if root.get("schema_version") != SCHEMA_VERSION:
        problems.append(f"schema_version must be {SCHEMA_VERSION}")
    if root.get("kind") != KIND:
        problems.append(f"kind must be {KIND!r}")

    producer = _object(root.get("producer"), "producer", problems)
    _required(producer, ("id", "version"), "producer", problems)
    for key in ("id", "version"):
        if key in producer and (not isinstance(producer[key], str) or not producer[key].strip()):
            problems.append(f"producer.{key} must be a non-empty string")

    source = _object(root.get("source"), "source", problems)
    _required(source, ("head", "tree_digest", "dirty"), "source", problems)
    if "head" in source and not (
        isinstance(source["head"], str) and GIT_SHA.fullmatch(source["head"])
    ):
        problems.append("source.head must be a lowercase 40-character git commit")
    if "tree_digest" in source:
        _sha(source["tree_digest"], "source.tree_digest", problems)
    if "dirty" in source and not isinstance(source["dirty"], bool):
        problems.append("source.dirty must be boolean")
    if expected_source is not None:
        for key in ("head", "tree_digest", "dirty"):
            if source.get(key) != expected_source.get(key):
                problems.append(f"source.{key} does not match the expected source")

    command = _object(root.get("command"), "command", problems)
    _required(command, ("argv", "cwd", "environment", "sha256"), "command", problems)
    argv, environment = command.get("argv"), command.get("environment")
    if not isinstance(argv, list) or not argv or not all(isinstance(x, str) and x for x in argv):
        problems.append("command.argv must be a non-empty string array")
    if not isinstance(command.get("cwd"), str) or not command.get("cwd"):
        problems.append("command.cwd must be a non-empty string")
    if not isinstance(environment, dict) or not all(
        isinstance(k, str) and isinstance(v, str) for k, v in environment.items()
    ):
        problems.append("command.environment must map strings to strings")
    if "sha256" in command:
        _sha(command["sha256"], "command.sha256", problems)
        if (
            isinstance(argv, list)
            and isinstance(environment, dict)
            and isinstance(command.get("cwd"), str)
        ):
            if command["sha256"] != command_digest(command):
                problems.append("command.sha256 does not match argv/cwd/environment")

    tools = root.get("tools")
    if not isinstance(tools, list) or not tools:
        problems.append("tools must be a non-empty array")
    else:
        names: set[str] = set()
        for index, value in enumerate(tools):
            tool = _object(value, f"tools[{index}]", problems)
            _required(tool, ("name", "version", "identity_sha256"), f"tools[{index}]", problems)
            name, version = tool.get("name"), tool.get("version")
            if not isinstance(name, str) or not name:
                problems.append(f"tools[{index}].name must be non-empty")
            elif name in names:
                problems.append(f"tools contains duplicate name {name!r}")
            else:
                names.add(name)
            if not isinstance(version, str) or not version.strip():
                problems.append(f"tools[{index}].version must be exact and non-empty")
            elif MUTABLE_TOOL_VERSION.fullmatch(version.strip()) or not any(
                character.isdigit() for character in version
            ):
                problems.append(f"tools[{index}].version is a mutable or ambiguous alias")
            if "identity_sha256" in tool:
                _sha(tool["identity_sha256"], f"tools[{index}].identity_sha256", problems)
                if isinstance(name, str) and isinstance(version, str):
                    if tool["identity_sha256"] != tool_identity_digest(tool):
                        problems.append(
                            f"tools[{index}].identity_sha256 does not match name/version"
                        )

    configs = root.get("configs")
    if not isinstance(configs, list) or not configs:
        problems.append("configs must be a non-empty array")
    else:
        paths: set[str] = set()
        for index, value in enumerate(configs):
            config = _object(value, f"configs[{index}]", problems)
            _required(config, ("path", "sha256"), f"configs[{index}]", problems)
            path = config.get("path")
            if not _safe_relative(path):
                problems.append(f"configs[{index}].path must be a safe relative path")
            elif path in paths:
                problems.append(f"configs contains duplicate path {path!r}")
            else:
                paths.add(path)
            if "sha256" in config:
                _sha(config["sha256"], f"configs[{index}].sha256", problems)
            if artifact_root is not None and _safe_relative(path):
                _verify_file_identity(
                    root=artifact_root,
                    path=path,
                    expected_sha256=config.get("sha256"),
                    name="config",
                    problems=problems,
                )

    action = _object(root.get("action"), "action", problems)
    _required(
        action,
        (
            "context",
            "workflow",
            "workflow_ref",
            "workflow_sha256",
            "source_sha",
            "job",
            "event",
            "actions",
        ),
        "action",
        problems,
    )
    if "workflow" in action and not _safe_relative(action["workflow"]):
        problems.append("action.workflow must be a safe relative path")
    for key in ("job", "event"):
        if key in action and (not isinstance(action[key], str) or not action[key]):
            problems.append(f"action.{key} must be a non-empty string")
    context = action.get("context")
    workflow_ref = action.get("workflow_ref")
    source_sha = action.get("source_sha")
    if context not in {"local", "github-actions"}:
        problems.append("action.context must be 'local' or 'github-actions'")
    if not isinstance(source_sha, str) or not GIT_SHA.fullmatch(source_sha):
        problems.append("action.source_sha must be a lowercase 40-character git commit")
    elif source_sha != source.get("head"):
        problems.append("action.source_sha does not match source.head")
    if context == "local":
        if workflow_ref != f"local:{action.get('workflow')}":
            problems.append("local action.workflow_ref does not match action.workflow")
        if action.get("event") != "local":
            problems.append("local action.event must be 'local'")
    elif context == "github-actions":
        match = WORKFLOW_REF.fullmatch(workflow_ref) if isinstance(workflow_ref, str) else None
        if match is None:
            problems.append("hosted action.workflow_ref is malformed")
        elif match.group("path") != action.get("workflow"):
            problems.append("hosted action.workflow_ref does not match action.workflow")
    if "workflow_sha256" in action:
        _sha(action["workflow_sha256"], "action.workflow_sha256", problems)
    if artifact_root is not None and _safe_relative(action.get("workflow")):
        _verify_file_identity(
            root=artifact_root,
            path=action["workflow"],
            expected_sha256=action.get("workflow_sha256"),
            name="workflow",
            problems=problems,
        )
    actions = action.get("actions")
    if not isinstance(actions, list):
        problems.append("action.actions must be an array")
    else:
        for index, ref in enumerate(actions):
            if not isinstance(ref, str) or not PINNED_ACTION.fullmatch(ref):
                problems.append(f"action.actions[{index}] is not pinned to a full commit SHA")

    artifacts = root.get("artifacts")
    raw_count = 0
    if not isinstance(artifacts, list) or not artifacts:
        problems.append("artifacts must be a non-empty array")
    else:
        paths: set[str] = set()
        for index, value in enumerate(artifacts):
            item = _object(value, f"artifacts[{index}]", problems)
            _required(item, ("path", "sha256", "size", "role"), f"artifacts[{index}]", problems)
            path = item.get("path")
            if not _safe_relative(path):
                problems.append(f"artifacts[{index}].path must be a safe relative path")
            elif path in paths:
                problems.append(f"artifacts contains duplicate path {path!r}")
            else:
                paths.add(path)
            if "sha256" in item:
                _sha(item["sha256"], f"artifacts[{index}].sha256", problems)
            if not isinstance(item.get("size"), int) or item.get("size", 0) <= 0:
                problems.append(f"artifacts[{index}].size must be a positive integer")
            if item.get("role") == "raw":
                raw_count += 1
            elif item.get("role") not in {"summary", "replay", "diagnostic"}:
                problems.append(f"artifacts[{index}].role is unknown")
            if artifact_root is not None and _safe_relative(path):
                disk = _confined_regular_file(
                    root=artifact_root, path=path, name="artifact", problems=problems
                )
                if disk is not None:
                    data = disk.read_bytes()
                    if len(data) != item.get("size"):
                        problems.append(f"artifact {path!r} size mismatch")
                    if hashlib.sha256(data).hexdigest() != item.get("sha256"):
                        problems.append(f"artifact {path!r} digest mismatch")
    if raw_count == 0:
        problems.append("artifacts must contain at least one raw artifact")

    result = _object(root.get("result"), "result", problems)
    result_keys = (
        "status",
        "exit_code",
        "started_at",
        "finished_at",
        "selected_count",
        "executed_count",
    )
    _required(result, result_keys, "result", problems)
    status, exit_code = result.get("status"), result.get("exit_code")
    if status not in STATUSES:
        problems.append(f"result.status must be one of {sorted(STATUSES)}")
    if exit_code is not None and not isinstance(exit_code, int):
        problems.append("result.exit_code must be an integer or null")
    started = _utc_timestamp(result.get("started_at"), "result.started_at", problems)
    finished = _utc_timestamp(result.get("finished_at"), "result.finished_at", problems)
    if started is not None and finished is not None and finished < started:
        problems.append("result.finished_at must not precede result.started_at")
    selected, executed = result.get("selected_count"), result.get("executed_count")
    if not isinstance(selected, int) or selected < 0:
        problems.append("result.selected_count must be a non-negative integer")
    if not isinstance(executed, int) or executed < 0:
        problems.append("result.executed_count must be a non-negative integer")
    if status == "PASS":
        if exit_code != 0:
            problems.append("PASS requires exit_code 0")
        if not isinstance(selected, int) or selected <= 0:
            problems.append("PASS requires selected_count > 0")
        if executed != selected:
            problems.append("PASS requires executed_count == selected_count")
    return problems


def qualification_problems(
    envelope: object, *, artifact_root: Path, expected_source: dict[str, object] | None = None
) -> list[str]:
    problems = envelope_problems(
        envelope, artifact_root=artifact_root, expected_source=expected_source
    )
    result = envelope.get("result") if isinstance(envelope, dict) else None
    status = result.get("status") if isinstance(result, dict) else None
    if status != "PASS":
        problems.append(f"required producer status is {status!r}, not 'PASS'")
    return problems
