#!/usr/bin/env python3
"""Run the CI Rust test denominator and verify every selected case terminated."""

from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO))

from tools.gates.target_catalog import (  # noqa: E402
    RAYON_SCOPES,
    RECIPE_FRAGMENTS,
    ZERO_CASE_LIBS,
    catalog_digest,
    recipe_body,
    source_catalog,
)

NEXTEST_VERSION = "0.9.104"
BASE = ("--locked", "--lib", "--tests")
SCOPES = (
    ("workspace", ("--workspace", "--exclude", "taskmesh-doc-examples")),
    ("doc-examples", ("-p", "taskmesh-doc-examples")),
)


def digest(value: object) -> str:
    payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(payload).hexdigest()


def run_command(args: list[str], *, env: dict[str, str] | None = None) -> str:
    process = subprocess.run(args, cwd=REPO, env=env, capture_output=True, text=True)
    if process.stderr:
        print(process.stderr, file=sys.stderr, end="")
    if process.returncode:
        raise ValueError(f"{args[0:3]} exited {process.returncode}")
    return process.stdout


def listed_cases(
    value: object, *, allow_filtered: bool = False
) -> tuple[set[tuple[str, str, str]], set[str]]:
    if not isinstance(value, dict) or not isinstance(value.get("rust-suites"), dict):
        raise ValueError("nextest list lacks rust-suites")
    cases: set[tuple[str, str, str]] = set()
    targets: set[str] = set()
    for suite in value["rust-suites"].values():
        if not isinstance(suite, dict) or suite.get("status") != "listed":
            raise ValueError("nextest list contains an unlisted suite")
        package, binary, kind = (suite.get(key) for key in ("package-name", "binary-name", "kind"))
        tests = suite.get("testcases")
        if not all(isinstance(field, str) for field in (package, binary, kind)) or not isinstance(
            tests, dict
        ):
            raise ValueError("nextest list contains a malformed suite")
        target = f"{package}/{kind}/{binary}"
        if target in targets:
            raise ValueError(f"duplicate selected binary {target}")
        targets.add(target)
        selected_in_suite = 0
        for case, metadata in tests.items():
            if not isinstance(case, str) or not isinstance(metadata, dict):
                raise ValueError(f"malformed test case in {target}")
            if metadata.get("ignored") is True:
                continue
            filter_match = metadata.get("filter-match")
            if (
                allow_filtered
                and isinstance(filter_match, dict)
                and filter_match.get("status") == "mismatch"
            ):
                continue
            if metadata.get("ignored") is not False or filter_match != {"status": "matches"}:
                raise ValueError(f"filtered or malformed test case {target}${case}")
            cases.add((package, binary, case))
            selected_in_suite += 1
        if selected_in_suite == 0 and (allow_filtered or target not in ZERO_CASE_LIBS):
            raise ValueError(f"nextest selected no runnable cases in {target}")
    total = value.get("test-count")
    if not targets or not cases or type(total) is not int or total < len(cases):
        raise ValueError("nextest list has an empty or inconsistent test denominator")
    if not allow_filtered and total != len(cases):
        raise ValueError("nextest list has an empty or inconsistent test denominator")
    return cases, targets


def executed_cases(
    lines: str, expected: set[tuple[str, str, str]], *, allow_filtered: bool = False
) -> set[tuple[str, str, str]]:
    started: set[tuple[str, str, str]] = set()
    passed: set[tuple[str, str, str]] = set()
    suite_started: set[tuple[str, str]] = set()
    suite_finished: set[tuple[str, str]] = set()
    expected_by_suite: dict[tuple[str, str], int] = {}
    for package, binary, _case in expected:
        key = (package, binary)
        expected_by_suite[key] = expected_by_suite.get(key, 0) + 1
    for line in lines.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError as exc:
            raise ValueError("nextest run emitted non-JSON output") from exc
        if not isinstance(event, dict):
            raise ValueError("nextest run emitted non-object output")
        if event.get("type") == "suite":
            info = event.get("nextest")
            if not isinstance(info, dict):
                raise ValueError("nextest suite lacks identity")
            key = (info.get("crate"), info.get("test_binary"))
            if not all(isinstance(x, str) for x in key):
                raise ValueError("nextest suite identity is malformed")
            if event.get("event") == "started":
                if key in suite_started:
                    raise ValueError(f"duplicate suite start {key}")
                if event.get("test_count") != expected_by_suite.get(key, 0):
                    raise ValueError(f"suite start denominator differs from selected cases {key}")
                suite_started.add(key)
            elif event.get("event") == "ok":
                if key in suite_finished or key not in suite_started:
                    raise ValueError(f"unmatched suite finish {key}")
                filtered_out = event.get("filtered_out")
                if (
                    event.get("passed") != expected_by_suite.get(key, 0)
                    or event.get("failed") != 0
                    or event.get("ignored") != 0
                    or type(filtered_out) is not int
                    or filtered_out < 0
                    or (not allow_filtered and filtered_out != 0)
                ):
                    raise ValueError(f"non-pass suite denominator {key}")
                suite_finished.add(key)
            else:
                raise ValueError(f"non-pass suite event {event.get('event')}")
        elif event.get("type") == "test":
            name = event.get("name")
            match = re.fullmatch(r"(.+?)::([^$]+)\$(.+)", name) if isinstance(name, str) else None
            if match is None:
                raise ValueError("nextest test event lacks identity")
            key = match.groups()
            if key not in expected:
                raise ValueError(f"unexpected test event {name}")
            if event.get("event") == "started":
                if key in started:
                    raise ValueError(f"duplicate test start {name}")
                started.add(key)
            elif event.get("event") == "ok":
                if key not in started or key in passed:
                    raise ValueError(f"unmatched test finish {name}")
                passed.add(key)
            else:
                raise ValueError(f"non-pass test event {event.get('event')} for {name}")
        else:
            raise ValueError(f"unknown nextest event type {event.get('type')}")
    if passed != expected or started != expected or suite_started != suite_finished:
        raise ValueError(
            f"nextest execution denominator mismatch: selected={len(expected)} "
            f"started={len(started)} passed={len(passed)} "
            f"suites={len(suite_finished)}/{len(suite_started)}"
        )
    return passed


def main() -> int:
    try:
        rayon = sys.argv[1:] == ["--rayon"]
        if sys.argv[1:] and not rayon:
            raise ValueError("only --rayon is a supported argument")
        version = run_command(["cargo", "nextest", "--version"]).splitlines()[0]
        if not version.startswith(f"cargo-nextest {NEXTEST_VERSION} "):
            raise ValueError(
                f"qualified runner must be cargo-nextest {NEXTEST_VERSION}; got {version!r}"
            )
        records, problems = source_catalog(
            REPO, {name: recipe_body(name) for name in RECIPE_FRAGMENTS}
        )
        if problems:
            raise ValueError(f"static target catalog failed: {problems}")
        expected_targets = {
            f"{record['package']}/{record['kind']}/{record['target']}"
            for record in records
            if record["executing_gate"] == "test"
        }
        if not expected_targets:
            raise ValueError("static test target denominator is empty")
        test_jobs = os.environ.get("TASKMESH_TEST_JOBS", "4")
        if not test_jobs.isdecimal() or int(test_jobs) < 1:
            raise ValueError("TASKMESH_TEST_JOBS must be a positive integer")
        env = dict(os.environ, NEXTEST_EXPERIMENTAL_LIBTEST_JSON="1")
        all_cases: set[tuple[str, str, str]] = set()
        all_targets: set[str] = set()
        commands: list[list[str]] = []
        scopes: list[tuple[list[str], tuple[str, str, str] | None]] = []
        if rayon:
            for package, kind, target, case in RAYON_SCOPES:
                args = ["--locked", "-p", package]
                if package == "taskmesh":
                    args.extend(("--features", "rayon"))
                args.extend(("--lib",) if kind == "lib" else ("--test", target))
                scopes.append((args, (package, target, case) if case else None))
            expected_targets = {
                f"{package}/{kind}/{target}" for package, kind, target, _ in RAYON_SCOPES
            }
        else:
            scopes = [([*BASE, *scope], None) for _, scope in SCOPES]
        for scope, exact_case in scopes:
            suffix = ["--", exact_case[2], "--exact"] if exact_case else []
            selection = ["cargo", "nextest", "list", *scope, "--message-format", "json", *suffix]
            execution = [
                "cargo",
                "nextest",
                "run",
                *scope,
                "--message-format",
                "libtest-json-plus",
                "--test-threads",
                test_jobs,
                *suffix,
            ]
            listed = json.loads(run_command(selection))
            cases, targets = listed_cases(listed, allow_filtered=exact_case is not None)
            if exact_case is not None and cases != {exact_case}:
                raise ValueError(
                    f"Rayon exact selector did not select {exact_case}: {sorted(cases)}"
                )
            if (not rayon and all_targets & targets) or all_cases & cases:
                raise ValueError("test scopes overlap")
            output = run_command(execution, env=env)
            executed_cases(output, cases, allow_filtered=exact_case is not None)
            all_cases.update(cases)
            all_targets.update(targets)
            commands.extend((selection, execution))
        if all_targets != expected_targets:
            raise ValueError(
                f"compiled target mismatch: missing={sorted(expected_targets - all_targets)} "
                f"extra={sorted(all_targets - expected_targets)}"
            )
        summary = {
            "schema_version": 1,
            "runner": "nextest",
            "runner_version": NEXTEST_VERSION,
            "catalog_digest": catalog_digest(records),
            "selection_digest": digest(sorted(all_cases)),
            "execution_digest": digest(sorted(all_cases)),
            "commands_digest": digest(commands),
            "targets": len(all_targets),
            "cases": len(all_cases),
        }
        summary["summary_digest"] = digest(summary)
        fields = " ".join(f"{key}={value}" for key, value in summary.items())
        if rayon:
            print("taskmesh-test-rayon status=PASS " + fields)
        else:
            print("taskmesh-test status=PASS " + fields)
        return 0
    except (OSError, subprocess.SubprocessError, ValueError, KeyError, TypeError) as exc:
        print(f"taskmesh-test status=FAIL reason={exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
