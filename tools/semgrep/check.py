#!/usr/bin/env python3
"""Run the real Semgrep gate once and verify its target-set enrollment."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
RULES = REPO / "tools" / "semgrep" / "rules"
ENROLLED_PATHS = (
    "crates/taskmesh/tests/e2e_chaos.rs",
    "crates/taskmesh/tests/hardening_deadline_custody.rs",
    "crates/taskmesh-engine/tests/hardening_fairness_reference.rs",
    "crates/taskmesh-engine/tests/prop_invariants.rs",
    "crates/taskmesh-bench/tests/inferno.rs",
    "crates/taskmesh-rayon/tests/rayon_smoke.rs",
)


def target_set_problems(scanned: set[str], root: Path = REPO) -> list[str]:
    """Reject a clean-looking scan that did not open the governed tests."""
    problems: list[str] = []
    if not scanned:
        return ["semgrep reported no scanned paths"]
    missing = sorted(path for path in ENROLLED_PATHS if path not in scanned)
    if missing:
        problems.append(f"enrolled integration tests were not scanned: {missing}")
    for crate_tests in sorted((root / "crates").glob("*/tests")):
        if not crate_tests.is_dir():
            continue
        expected = {
            str(path.relative_to(root)) for path in crate_tests.rglob("*.rs") if path.is_file()
        }
        if expected and not expected.intersection(scanned):
            problems.append(f"no Rust file under {crate_tests.relative_to(root)} was scanned")
    return problems


def main() -> int:
    try:
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
    scanned = {
        str(Path(path))
        for path in raw_scanned
        if isinstance(path, str)
    }
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
    print(f"semgrep gate: PASS ({len(scanned)} files, 0 findings)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
