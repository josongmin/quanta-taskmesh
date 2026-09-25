#!/usr/bin/env python3
"""Run the CI Rust test surface with a checked selection/execution denominator."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
if str(REPO) not in sys.path:
    sys.path.insert(0, str(REPO))

from tools.gates.target_catalog import (  # noqa: E402
    RECIPE_FRAGMENTS,
    ZERO_CASE_LIBS,
    catalog_digest,
    source_catalog,
)
from tools.gates.validate_inventory import recipe_body  # noqa: E402
from tools.qualification.receipt import source_identity  # noqa: E402

REPORT = REPO / "target/verification/rust-test-execution.json"
SELECTORS = (
    ("--workspace", "--exclude", "taskmesh-doc-examples", "--lib", "--tests"),
    ("-p", "taskmesh-doc-examples", "--lib", "--tests"),
)
RUNNER_VERSION = "0.9.104"


def digest(value: object) -> str:
    payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(payload).hexdigest()


def run(argv: list[str], *, env: dict[str, str] | None = None) -> str:
    result = subprocess.run(argv, cwd=REPO, env=env, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(
            f"{argv!r} exited {result.returncode}\n"
            + (result.stdout + result.stderr)[-4000:]
        )
    return result.stdout


def selected_cases(value: object) -> tuple[set[tuple[str, str, str]], set[str]]:
    if not isinstance(value, dict) or not isinstance(value.get("rust-suites"), dict):
        raise ValueError("nextest list has no rust-suites")
    targets: set[tuple[str, str, str]] = set()
    selected: set[str] = set()
    for suite in value["rust-suites"].values():
        if not isinstance(suite, dict) or suite.get("status") != "listed":
            raise ValueError("nextest suite was not listed")
        package, kind, target, binary_id = (
            suite.get("package-name"), suite.get("kind"),
            suite.get("binary-name"), suite.get("binary-id"),
        )
        if not all(isinstance(item, str) and item for item in (package, kind, target, binary_id)):
            raise ValueError("nextest suite identity is malformed")
        identity = (package, kind, target)
        if identity in targets:
            raise ValueError(f"duplicate selected target {identity}")
        targets.add(identity)
        cases = suite.get("testcases")
        if not isinstance(cases, dict):
            raise ValueError(f"{identity}: missing testcases")
        case_prefix = f"{package}::{target}" if kind == "lib" else binary_id
        selected_in_suite = 0
        for name, case in cases.items():
            if not isinstance(case, dict) or not isinstance(name, str):
                raise ValueError(f"{identity}: malformed testcase")
            match = case.get("filter-match")
            if not isinstance(match, dict):
                raise ValueError(f"{identity}: missing filter status")
            if case.get("ignored") is False and match.get("status") == "matches":
                selected.add(f"{case_prefix}${name}")
                selected_in_suite += 1
        if selected_in_suite == 0 and f"{package}/{kind}/{target}" not in ZERO_CASE_LIBS:
            raise ValueError(f"{identity}: no runnable cases")
    if value.get("test-count") != len(selected):
        raise ValueError("nextest test-count differs from selected nonignored cases")
    if not selected:
        raise ValueError("nextest selector matched no runnable cases")
    return targets, selected


def executed_cases(lines: str) -> tuple[set[str], set[str]]:
    started: set[str] = set()
    passed: set[str] = set()
    suites = 0
    for line in lines.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError as exc:
            raise ValueError("nextest emitted non-JSON result") from exc
        if not isinstance(event, dict):
            raise ValueError("nextest event is not an object")
        if event.get("type") == "test":
            name = event.get("name")
            if not isinstance(name, str) or not name:
                raise ValueError("nextest test event has no name")
            if event.get("event") == "started":
                if name in started:
                    raise ValueError(f"duplicate test start {name}")
                started.add(name)
            elif event.get("event") == "ok":
                if name not in started or name in passed:
                    raise ValueError(f"unmatched test pass {name}")
                passed.add(name)
            else:
                raise ValueError(f"test {name} did not pass: {event.get('event')}")
        elif event.get("type") == "suite" and event.get("event") == "ok":
            if event.get("failed") != 0 or event.get("ignored") != 0:
                raise ValueError("nextest suite reports failed or ignored cases")
            suites += 1
        elif event.get("type") == "suite" and event.get("event") == "started":
            pass
        else:
            raise ValueError(f"unrecognized nextest event: {event}")
    if not suites:
        raise ValueError("nextest ran no suites")
    return started, passed


def main() -> None:
    REPORT.unlink(missing_ok=True)
    before = source_identity(REPO)
    if before["dirty"]:
        raise ValueError("qualified Rust tests require clean source")
    identity = run(["cargo", "nextest", "--version"]).splitlines()[0].split()
    if len(identity) < 2 or identity[:2] != ["cargo-nextest", RUNNER_VERSION]:
        raise ValueError(f"nextest must be pinned to {RUNNER_VERSION}")
    recipes = {name: recipe_body(name) for name in RECIPE_FRAGMENTS}
    records, problems = source_catalog(REPO, recipes)
    if problems:
        raise ValueError(f"target catalog failed: {problems}")
    expected_targets = {
        (record["package"], record["kind"], record["target"])
        for record in records if record["executing_gate"] == "test"
    }
    selected_targets: set[tuple[str, str, str]] = set()
    selected: set[str] = set()
    passed: set[str] = set()
    commands = []
    for selector in SELECTORS:
        common = ["--locked", *selector]
        listed = json.loads(run(["cargo", "nextest", "list", *common, "--message-format", "json"]))
        targets, cases = selected_cases(listed)
        if selected_targets & targets or selected & cases:
            raise ValueError("test selections overlap")
        selected_targets.update(targets)
        selected.update(cases)
        argv = [
            "cargo", "nextest", "run", *common,
            "--message-format", "libtest-json-plus", "--message-format-version", "0.1",
            "--test-threads", os.environ.get("TASKMESH_TEST_JOBS", "4"),
        ]
        env = {**os.environ, "NEXTEST_EXPERIMENTAL_LIBTEST_JSON": "1"}
        started, completed = executed_cases(run(argv, env=env))
        if started != cases or completed != cases:
            raise ValueError(
                f"Nextest execution differs from selection: selected={len(cases)} "
                f"started={len(started)} passed={len(completed)} "
                f"missing_start={sorted(cases - started)[:5]} "
                f"extra_start={sorted(started - cases)[:5]} "
                f"missing_pass={sorted(cases - completed)[:5]}"
            )
        passed.update(completed)
        commands.append({
            "list": ["cargo", "nextest", "list", *common, "--message-format", "json"],
            "run": argv,
        })
    if selected_targets != expected_targets:
        raise ValueError(
            f"target denominator mismatch: missing={sorted(expected_targets - selected_targets)} "
            f"extra={sorted(selected_targets - expected_targets)}"
        )
    if not selected or selected != passed:
        raise ValueError("empty or incomplete Rust case denominator")
    after = source_identity(REPO)
    if any(before[key] != after[key] for key in ("head", "tree", "paths_digest", "dirty")):
        raise ValueError("Rust test source changed during execution")
    report = {
        "schema_version": 1,
        "source": before,
        "runner": "nextest",
        "runner_version": RUNNER_VERSION,
        "experimental_env": {"NEXTEST_EXPERIMENTAL_LIBTEST_JSON": "1"},
        "commands": commands,
        "catalog_digest": catalog_digest(records),
        "targets": sorted(map(list, selected_targets)),
        "selected": sorted(selected),
        "passed": sorted(passed),
    }
    report["selection_digest"] = digest(report["selected"])
    report["execution_digest"] = digest(report["passed"])
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(
        f"taskmesh-test status=PASS runner=nextest runner_version={RUNNER_VERSION} "
        f"targets={len(selected_targets)} selected={len(selected)} passed={len(passed)} "
        f"catalog_digest={report['catalog_digest']} "
        f"selection_digest={report['selection_digest']} "
        f"execution_digest={report['execution_digest']}"
    )


if __name__ == "__main__":
    try:
        main()
    except (OSError, subprocess.SubprocessError, ValueError, RuntimeError) as exc:
        print(f"Rust test denominator FAIL: {exc}", file=sys.stderr)
        raise SystemExit(1) from exc
