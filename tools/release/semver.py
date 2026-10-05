#!/usr/bin/env python3
"""Run each public crate's semver audit against one immutable Git baseline.

This is an audit producer, not a release decision. In particular, a clean
semver-checks result does not cover facade re-exports, Rust return types, wire
formats, or behavior. The release receipt requires separate adjudication.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
POLICY = Path(__file__).with_name("release-policy.json")
SHA = re.compile(r"[0-9a-f]{40}\Z")

if str(REPO) not in sys.path:
    sys.path.insert(0, str(REPO))

from tools.inspection import metadata_output  # noqa: E402
from tools.qualification.receipt import source_identity  # noqa: E402
from tools.release.execution import run_release_command, settled_exit_code  # noqa: E402


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def git(*args: str) -> str:
    return metadata_output(["git", *args], cwd=REPO).decode("utf-8").strip()


def workspace_version(text: str) -> str | None:
    section = re.search(r"^\[workspace\.package\]\s*$([\s\S]*?)(?=^\[|\Z)", text, re.MULTILINE)
    if section is None:
        return None
    version = re.search(r'^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"\s*$', section[1], re.MULTILINE)
    return version[1] if version else None


def policy_problems(policy: object) -> list[str]:
    if not isinstance(policy, dict):
        return ["release policy is not an object"]
    problems = []
    if policy.get("schema_version") != 1:
        problems.append("release policy schema_version != 1")
    baseline = policy.get("baseline_sha")
    if not isinstance(baseline, str) or not SHA.fullmatch(baseline):
        problems.append("baseline must be a full immutable 40-character SHA")
    if policy.get("public_crates") != [
        "taskmesh-contract",
        "taskmesh-engine",
        "taskmesh",
        "taskmesh-rayon",
    ]:
        problems.append("public crate set/order differs from the four shipped libraries")
    if policy.get("features") != ["--default-features"]:
        problems.append("semver feature selection is not the consumer default surface")
    if policy.get("release_type") != "minor":
        problems.append("semver audit must expose major-level breaks with --release-type minor")
    if policy.get("semver_tool") != "cargo-semver-checks 0.50.0":
        problems.append("semver tool identity is not pinned to 0.50.0")
    return problems


def command(crate: str, baseline: str) -> list[str]:
    return [
        "cargo",
        "semver-checks",
        "check-release",
        "-p",
        crate,
        "--baseline-rev",
        baseline,
        "--default-features",
        "--release-type",
        "minor",
        "--color",
        "never",
    ]


def produce(out: Path) -> int:
    policy = json.loads(POLICY.read_text(encoding="utf-8"))
    problems = policy_problems(policy)
    if problems:
        raise ValueError("; ".join(problems))
    baseline = policy["baseline_sha"]
    if git("rev-parse", "--verify", f"{baseline}^{{commit}}") != baseline:
        raise ValueError("baseline SHA does not resolve to the exact commit")
    baseline_version = workspace_version(git("show", f"{baseline}:Cargo.toml"))
    if baseline_version != policy["baseline_version"]:
        raise ValueError("baseline version does not match the immutable commit")
    before = source_identity()
    if before["dirty"]:
        raise ValueError("semver audit requires a clean candidate source")
    if out.is_symlink() or out.exists() and not out.is_dir():
        raise ValueError("output must be a directory, not a symlink or file")
    out = out.resolve()
    if not out.is_relative_to((REPO / "target").resolve()):
        raise ValueError("output must be under repository target/")
    out.mkdir(parents=True, exist_ok=True)
    tool = run_release_command(
        ["cargo", "semver-checks", "--version"],
        cwd=REPO,
        timeout_seconds=30,
    )
    tool_version = tool.stdout.decode("utf-8", errors="replace").strip()
    if settled_exit_code(tool) != 0 or tool_version != policy["semver_tool"]:
        raise ValueError("installed cargo-semver-checks does not match release policy")
    results = []
    for crate in policy["public_crates"]:
        argv = command(crate, baseline)
        started_at = datetime.now(timezone.utc).isoformat()
        try:
            process = run_release_command(argv, cwd=REPO, timeout_seconds=3600)
            exit_code = settled_exit_code(process)
            stdout, stderr = process.stdout, process.stderr
            timed_out = process.timed_out
            interrupted = process.interrupted_by_signal
            aborted_early = process.aborted_early
        except OSError as exc:
            exit_code = None
            stdout, stderr = b"", str(exc).encode()
            timed_out = False
            interrupted = None
            aborted_early = False
        stdout_path = out / f"{crate}.stdout.log"
        stderr_path = out / f"{crate}.stderr.log"
        stdout_path.write_bytes(stdout)
        stderr_path.write_bytes(stderr)
        audit_status = {0: "CLEAN", 100: "FINDINGS"}.get(exit_code, "TOOL_FAILURE")
        results.append(
            {
                "crate": crate,
                "command": argv,
                "started_at": started_at,
                "finished_at": datetime.now(timezone.utc).isoformat(),
                "exit_code": exit_code,
                "timed_out": timed_out,
                "interrupted_by_signal": interrupted,
                "aborted_early": aborted_early,
                "status": audit_status,
                "stdout": {
                    "path": str(stdout_path.relative_to(REPO)),
                    "sha256": sha256(stdout_path),
                    "size": stdout_path.stat().st_size,
                },
                "stderr": {
                    "path": str(stderr_path.relative_to(REPO)),
                    "sha256": sha256(stderr_path),
                    "size": stderr_path.stat().st_size,
                },
            }
        )
        if interrupted is not None:
            # The supervisor consumes the signal to settle child custody.
            # Preserve the user's stop request across subsequent commands.
            break
    after = source_identity()
    stable = before["paths_digest"] == after["paths_digest"] and not after["dirty"]
    manifest = {
        "schema_version": 1,
        "producer": "taskmesh-semver-release-v1",
        "status": (
            "REPORTED"
            if stable
            and len(results) == len(policy["public_crates"])
            and all(r["status"] in ("CLEAN", "FINDINGS") for r in results)
            else "FAIL"
        ),
        "source": before,
        "source_after": {
            field: after[field] for field in ("head", "tree", "paths_digest", "dirty")
        },
        "baseline_sha": baseline,
        "baseline_version": policy["baseline_version"],
        "policy_sha256": sha256(POLICY),
        "tool": tool_version,
        "features": policy["features"],
        "release_type": policy["release_type"],
        "candidate_version": policy["candidate_version"],
        "results": results,
    }
    (out / "semver-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"semver manifest: {out / 'semver-manifest.json'} ({manifest['status']})")
    return 0 if manifest["status"] == "REPORTED" else 1


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=REPO / "target/release/semver")
    args = parser.parse_args(argv)
    try:
        return produce(args.out)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"semver release NOT_RUN: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
