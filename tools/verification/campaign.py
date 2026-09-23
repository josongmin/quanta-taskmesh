#!/usr/bin/env python3
"""Isolation and evidence primitives shared by mutation campaign producers."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
if str(REPO) not in sys.path:
    sys.path.insert(0, str(REPO))

from tools.process_supervisor import run_process  # noqa: E402
from tools.qualification.evidence import (  # noqa: E402
    canonical_bytes,  # noqa: F401 - compatibility re-export for callers
    canonical_digest,  # noqa: F401 - compatibility re-export for callers
    git_source_paths,
    runtime_action,
    source_tree_digest,
)

AMBIENT_BUILD_ENV_KEYS = {
    "CARGO_BUILD_TARGET",
    "CARGO_ENCODED_RUSTFLAGS",
    "RUSTC",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTDOCFLAGS",
    "RUSTFLAGS",
    "RUSTUP_TOOLCHAIN",
}
AMBIENT_BUILD_ENV_PREFIXES = ("CARGO_PROFILE_", "CARGO_TARGET_", "CARGO_MUTANTS_")
CAMPAIGN_ISOLATION_ENV_KEYS = {"CARGO_BUILD_TARGET_DIR", "CARGO_TARGET_DIR"}


def sanitized_campaign_environment(parent: dict[str, str]) -> dict[str, str]:
    """Return a stable inherited environment or reject undeclared build semantics.

    Campaign-owned overrides are applied by the caller after this check and are
    recorded in the evidence envelope. Inherited process settings that can
    change compiled code are rejected because they are otherwise absent from
    the command identity and make two different executions look identical.
    """
    forbidden = sorted(
        key
        for key in parent
        if key not in CAMPAIGN_ISOLATION_ENV_KEYS
        and (
            key in AMBIENT_BUILD_ENV_KEYS
            or any(key.startswith(prefix) for prefix in AMBIENT_BUILD_ENV_PREFIXES)
        )
    )
    if forbidden:
        raise ValueError(
            "mutation campaign refuses unrecorded build or mutation-affecting environment: "
            + ", ".join(forbidden)
        )
    environment = parent.copy()
    for key in CAMPAIGN_ISOLATION_ENV_KEYS:
        environment.pop(key, None)
    return environment


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


tree_digest = source_tree_digest


def git_head(repo: Path) -> str:
    proc = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=repo, capture_output=True, text=True, check=False
    )
    if proc.returncode != 0:
        raise RuntimeError(f"git rev-parse failed: {proc.stderr.strip()}")
    return proc.stdout.strip()


def git_dirty(repo: Path) -> bool:
    proc = subprocess.run(
        ["git", "status", "--porcelain", "--untracked-files=all"],
        cwd=repo,
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"git status failed: {proc.stderr.strip()}")
    return bool(proc.stdout.strip())


@dataclass
class IsolatedCampaign:
    campaign_id: str
    root: Path
    source: Path
    target: Path
    source_paths: list[str]
    source_head: str
    source_dirty: bool
    source_before: str
    snapshot_digest: str
    created_at: str

    def source_after(self, original_repo: Path) -> str:
        current_paths = git_source_paths(original_repo)
        if current_paths != self.source_paths:
            return "PATH_SET_CHANGED:" + canonical_digest(current_paths)
        return tree_digest(original_repo, current_paths)

    def original_is_unchanged(self, original_repo: Path) -> bool:
        return self.source_after(original_repo) == self.source_before

    def cleanup(self) -> None:
        shutil.rmtree(self.root, ignore_errors=True)

    def identity(self) -> dict[str, object]:
        return {
            "campaign_id": self.campaign_id,
            "isolation_root": str(self.root),
            "source_root": str(self.source),
            "target_dir": str(self.target),
            "source_head": self.source_head,
            "source_dirty": self.source_dirty,
            "source_before_sha256": self.source_before,
            "snapshot_sha256": self.snapshot_digest,
            "created_at": self.created_at,
        }


def create_isolated_campaign(repo: Path, *, parent: Path | None = None) -> IsolatedCampaign:
    """Copy a stable current-source snapshot without reading or writing normal target state."""
    repo = repo.resolve()
    head = git_head(repo)
    paths = git_source_paths(repo)
    tracked = subprocess.run(
        ["git", "ls-files", "--cached", "-z"], cwd=repo, capture_output=True, check=True
    ).stdout
    before = tree_digest(repo, paths)
    dirty = git_dirty(repo)
    campaign_id = uuid.uuid4().hex
    if parent is not None:
        parent.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix=f"taskmesh-mutation-{campaign_id[:12]}-", dir=parent))
    source = root / "source"
    target = root / "target"

    try:
        source.mkdir()
        for relative in paths:
            original = repo / relative
            snapshot_path = source / relative
            snapshot_path.parent.mkdir(parents=True, exist_ok=True)
            if original.is_symlink():
                os.symlink(os.readlink(original), snapshot_path)
            elif original.is_file():
                shutil.copy2(original, snapshot_path, follow_symlinks=False)
            else:
                raise RuntimeError(f"git-visible source path is missing or unsupported: {relative}")
        # The snapshot includes non-ignored untracked source too, but gates
        # must still distinguish it from committed files. Recreate only git's
        # tracked-file index locally; copying sources without an index makes
        # inventory tests fail before a mutation reaches its intended oracle.
        subprocess.run(["git", "init", "-q"], cwd=source, check=True)
        subprocess.run(
            ["git", "add", "-f", "--pathspec-from-file=-", "--pathspec-file-nul"],
            cwd=source,
            input=tracked,
            capture_output=True,
            check=True,
        )
        snapshot_tracked = subprocess.run(
            ["git", "ls-files", "--cached", "-z"], cwd=source, capture_output=True, check=True
        ).stdout
        if snapshot_tracked != tracked:
            raise RuntimeError("isolated snapshot tracked-file index differs from source")
        target.mkdir()
        current_paths = git_source_paths(repo)
        after_copy = tree_digest(repo, current_paths)
        snapshot = tree_digest(source, paths)
        if (
            paths != current_paths
            or head != git_head(repo)
            or dirty != git_dirty(repo)
            or before != after_copy
            or before != snapshot
        ):
            raise RuntimeError(
                "source changed while the isolated snapshot was created; "
                "refusing mixed-source evidence"
            )
        return IsolatedCampaign(
            campaign_id=campaign_id,
            root=root,
            source=source,
            target=target,
            source_paths=paths,
            source_head=head,
            source_dirty=dirty,
            source_before=before,
            snapshot_digest=snapshot,
            created_at=utc_now(),
        )
    except BaseException:
        shutil.rmtree(root, ignore_errors=True)
        raise


@dataclass(frozen=True)
class ProcessResult:
    argv: list[str]
    returncode: int | None
    exit_code: int | None
    signal: int | None
    timed_out: bool
    stdout: str
    stderr: str
    started_at: str
    finished_at: str
    duration_s: float
    interrupted_by_signal: int | None = None


def execute(
    argv: list[str], *, cwd: Path, env: dict[str, str], timeout_seconds: int
) -> ProcessResult:
    """Run one process group and reap it on timeout or parent cancellation."""
    started_at = utc_now()
    started = time.monotonic()
    process = run_process(argv, cwd=cwd, env=env, timeout_seconds=timeout_seconds)
    returncode = process.returncode
    return ProcessResult(
        argv=list(argv),
        returncode=returncode,
        exit_code=returncode if returncode is not None and returncode >= 0 else None,
        signal=-returncode if returncode is not None and returncode < 0 else None,
        timed_out=process.timed_out,
        stdout=process.stdout,
        stderr=process.stderr,
        started_at=started_at,
        finished_at=utc_now(),
        duration_s=time.monotonic() - started,
        interrupted_by_signal=process.interrupted_by_signal,
    )


def exact_tool_version(argv: list[str], *, cwd: Path) -> str:
    proc = subprocess.run(argv, cwd=cwd, capture_output=True, text=True, check=False)
    output = (proc.stdout or proc.stderr).strip()
    if proc.returncode != 0 or not output:
        raise RuntimeError(f"could not identify tool with {argv!r}: {output}")
    return output


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def prepare_output_dir(repo: Path, output_dir: Path) -> Path:
    """Reset only a named V02 artifact directory, never an arbitrary caller path."""
    allowed_root = (repo / "target/sep21/v02").resolve()
    resolved = output_dir.resolve()
    try:
        relative = resolved.relative_to(allowed_root)
    except ValueError as error:
        raise ValueError(f"output directory must be below {allowed_root}") from error
    if not relative.parts:
        raise ValueError("output directory must name a campaign below the V02 artifact root")
    if resolved.exists():
        shutil.rmtree(resolved)
    resolved.mkdir(parents=True)
    return resolved


def evidence_envelope(
    *,
    repo: Path,
    campaign: IsolatedCampaign,
    producer_id: str,
    command: list[str],
    environment: dict[str, str],
    tools: list[dict[str, str]],
    config_paths: list[Path],
    artifact_paths: list[tuple[Path, str]],
    job: str,
    status: str,
    exit_code: int | None,
    started_at: str,
    finished_at: str,
    selected_count: int,
    executed_count: int,
) -> dict[str, object]:
    """Build an EvidenceEnvelopeV1 without weakening producer-specific semantics."""

    repo = repo.resolve()

    def relative(path: Path) -> str:
        try:
            return path.resolve().relative_to(repo).as_posix()
        except ValueError as error:
            raise ValueError(f"evidence path must be inside the repository: {path}") from error

    command_record = {
        "argv": command,
        "cwd": str(campaign.source),
        "environment": environment,
    }
    command_record["sha256"] = canonical_digest(command_record)
    artifacts = []
    for path, role in artifact_paths:
        size = path.stat().st_size
        if size <= 0:
            continue
        artifacts.append(
            {"path": relative(path), "sha256": sha256_file(path), "size": size, "role": role}
        )
    if not any(artifact["role"] == "raw" for artifact in artifacts):
        raise ValueError("evidence envelope requires a non-empty raw artifact")
    return {
        "schema_version": 1,
        "kind": "taskmesh-evidence-envelope",
        "producer": {"id": producer_id, "version": "1"},
        "source": {
            "head": campaign.source_head,
            "tree_digest": campaign.source_before,
            "dirty": campaign.source_dirty,
        },
        "command": command_record,
        "tools": tools,
        "configs": [{"path": relative(path), "sha256": sha256_file(path)} for path in config_paths],
        "action": runtime_action(
            root=repo,
            source_head=campaign.source_head,
            local_workflow="tools/verification/mutation-gate.json",
            local_job=job,
        ),
        "artifacts": artifacts,
        "result": {
            "status": status,
            "exit_code": exit_code,
            "started_at": started_at,
            "finished_at": finished_at,
            "selected_count": selected_count,
            "executed_count": executed_count,
        },
    }
