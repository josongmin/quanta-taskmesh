#!/usr/bin/env python3
"""Run the real Semgrep gate once and verify its target-set enrollment."""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
RULES = REPO / "tools" / "semgrep" / "rules"
VERSION_FILE = REPO / "tools" / "semgrep" / "version.txt"
REQUIRED_SEMGREP_VERSION = VERSION_FILE.read_text(encoding="utf-8").strip()
if re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", REQUIRED_SEMGREP_VERSION) is None:
    raise ValueError("tools/semgrep/version.txt must contain a pinned Semgrep version")


def semgrep_identity_problems(proc: subprocess.CompletedProcess[str]) -> list[str]:
    """Require the reviewed parser version before trusting scan output."""
    if proc.returncode != 0:
        return [f"semgrep --version exited {proc.returncode}"]
    reported = proc.stdout.strip().splitlines()
    if not reported or reported[0] != REQUIRED_SEMGREP_VERSION:
        actual = reported[0] if reported else "<empty>"
        return [f"semgrep version mismatch: expected {REQUIRED_SEMGREP_VERSION}, got {actual}"]
    return []


def target_set_problems(scanned: set[str], root: Path = REPO) -> list[str]:
    """Require every present Rust source file under ``crates/`` to be scanned.

    The rule pack governs production code, tests, benches, and examples.  A
    target-set check limited to ``crates/*/tests`` would still accept a clean
    Semgrep result after ``.semgrepignore`` accidentally excluded a production
    module or benchmark.
    """
    if not scanned:
        return ["semgrep reported no scanned paths"]
    scanned_relative: set[str] = set()
    for path in scanned:
        candidate = Path(path)
        if candidate.is_absolute():
            try:
                candidate = candidate.relative_to(root)
            except ValueError:
                continue
        scanned_relative.add(candidate.as_posix())

    crates = root / "crates"
    expected = {
        path.relative_to(root).as_posix()
        for path in crates.rglob("*.rs")
        if path.is_file()
    }
    if not expected:
        return ["no Rust source files found under crates"]
    missing = sorted(expected - scanned_relative)
    return [f"Rust crate files were not scanned: {missing}"] if missing else []


def main() -> int:
    try:
        identity = subprocess.run(
            ["semgrep", "--version"],
            cwd=REPO,
            capture_output=True,
            text=True,
            check=False,
        )
        identity_problems = semgrep_identity_problems(identity)
        if identity_problems:
            for problem in identity_problems:
                print(f"semgrep gate: {problem}", file=sys.stderr)
            return 1
        proc = subprocess.run(
            [
                "semgrep",
                "--config",
                str(RULES),
                "--error",
                "--json",
                "--verbose",
                "crates",
            ],
            cwd=REPO,
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError as exc:
        print(f"semgrep gate: command could not start: {exc}", file=sys.stderr)
        return 1
    try:
        payload = json.loads(proc.stdout or "{}")
    except json.JSONDecodeError as exc:
        print(f"semgrep gate: invalid JSON output: {exc}", file=sys.stderr)
        if proc.stderr:
            print(proc.stderr[-2000:], file=sys.stderr)
        return 1

    problems: list[str] = []
    results = payload.get("results")
    if not isinstance(results, list):
        problems.append("semgrep JSON field 'results' is not a list")
        results = []
    paths = payload.get("paths")
    if not isinstance(paths, dict):
        problems.append("semgrep JSON field 'paths' is not an object")
        paths = {}
    raw_scanned = paths.get("scanned")
    if not isinstance(raw_scanned, list):
        problems.append("semgrep JSON field 'paths.scanned' is not a list")
        raw_scanned = []
    scanned = {str(Path(path)) for path in raw_scanned if isinstance(path, str)}
    problems.extend(target_set_problems(scanned))
    if proc.returncode != 0:
        problems.append(f"semgrep exited {proc.returncode} with {len(results)} finding(s)")
    if results:
        for result in results[:20]:
            if not isinstance(result, dict):
                problems.append("semgrep JSON contains a non-object finding")
                continue
            location = result.get("start", {})
            if not isinstance(location, dict):
                location = {}
            problems.append(
                f"{result.get('check_id', 'unknown')} at {result.get('path', 'unknown')}:"
                f"{location.get('line', '?')}"
            )
    if problems:
        print("semgrep gate: FAIL", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    print(
        f"semgrep gate: PASS (version {REQUIRED_SEMGREP_VERSION}, {len(scanned)} files, 0 findings)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
