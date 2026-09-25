"""Validate the selected and executed test denominator saved in a gate receipt."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

from tools.gates.execute_rust_tests import SELECTORS
from tools.gates.target_catalog import RECIPE_FRAGMENTS, catalog_digest, source_catalog
from tools.gates.validate_inventory import recipe_body

REPO = Path(__file__).resolve().parents[2]
REPORTS = {
    "test": REPO / "target/verification/rust-test-execution.json",
    "py-test": REPO / "target/verification/pytest-execution.json",
}


def digest(value: object) -> str:
    payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(payload).hexdigest()


def fields(line: str) -> dict[str, str]:
    parts = line.split()
    found = {}
    for token in parts[1:]:
        if "=" not in token:
            raise ValueError("test status line has malformed field")
        key, value = token.split("=", 1)
        if key in found or not value:
            raise ValueError("test status line has duplicate/empty field")
        found[key] = value
    return found


def report_problems(
    gate_id: str,
    report: object,
    line: str | None,
    source: dict,
    *,
    root: Path = REPO,
) -> list[str]:
    if gate_id not in REPORTS:
        return []
    if not isinstance(report, dict):
        return [f"{gate_id}: execution report is missing"]
    problems = []
    if report.get("schema_version") != 1 or report.get("source") != source:
        problems.append(f"{gate_id}: execution report source/schema mismatch")
    if not isinstance(line, str):
        return [*problems, f"{gate_id}: execution status line is missing"]
    try:
        stated = fields(line)
    except ValueError as exc:
        return [*problems, f"{gate_id}: {exc}"]
    recipes = {name: recipe_body(name) for name in RECIPE_FRAGMENTS}
    try:
        records, catalog_problems = source_catalog(root, recipes)
    except (OSError, ValueError, KeyError) as exc:
        return [*problems, f"{gate_id}: catalog unavailable: {exc}"]
    if catalog_problems:
        problems.append(f"{gate_id}: catalog invalid: {catalog_problems}")
    expected_catalog = catalog_digest(records)
    if (
        report.get("catalog_digest") != expected_catalog
        or stated.get("catalog_digest") != expected_catalog
    ):
        problems.append(f"{gate_id}: catalog digest mismatch")
    selected = report.get("selected")
    passed = report.get("passed")
    if (
        not isinstance(selected, list)
        or not isinstance(passed, list)
        or not selected
        or not all(isinstance(name, str) for name in selected + passed)
        or selected != sorted(set(selected))
        or passed != sorted(set(passed))
    ):
        return [*problems, f"{gate_id}: selected/passed case lists are malformed"]
    if stated.get("runner") != report.get("runner") or stated.get("runner_version") != report.get(
        "runner_version"
    ):
        problems.append(f"{gate_id}: runner identity mismatch")
    if report.get("selection_digest") != digest(selected) or stated.get(
        "selection_digest"
    ) != digest(selected):
        problems.append(f"{gate_id}: selection digest mismatch")
    if gate_id == "test":
        targets = report.get("targets")
        expected_targets = sorted(
            [
                [record["package"], record["kind"], record["target"]]
                for record in records
                if record["executing_gate"] == "test"
            ]
        )
        expected_commands = []
        for selector in SELECTORS:
            common = ["--locked", *selector]
            expected_commands.append({
                "list": ["cargo", "nextest", "list", *common, "--message-format", "json"],
                "run_prefix": [
                    "cargo", "nextest", "run", *common, "--message-format",
                    "libtest-json-plus", "--message-format-version", "0.1",
                    "--test-threads",
                ],
            })
        commands = report.get("commands")
        commands_valid = isinstance(commands, list) and len(commands) == len(SELECTORS)
        if commands_valid:
            for command, expected in zip(commands, expected_commands):
                if not isinstance(command, dict) or command.get("list") != expected["list"]:
                    commands_valid = False
                    break
                run = command.get("run")
                if (
                    not isinstance(run, list) or run[:-1] != expected["run_prefix"]
                    or not isinstance(run[-1], str) or not run[-1].isdigit()
                    or int(run[-1]) <= 0
                ):
                    commands_valid = False
                    break
        if (
            targets != expected_targets or not commands_valid
            or report.get("runner") != "nextest"
            or report.get("runner_version") != "0.9.104"
            or report.get("experimental_env") != {"NEXTEST_EXPERIMENTAL_LIBTEST_JSON": "1"}
        ):
            problems.append("test: selected target or command denominator mismatch")
        if selected != passed or report.get("execution_digest") != digest(passed):
            problems.append("test: execution differs from selected cases")
        if stated.get("execution_digest") != digest(passed):
            problems.append("test: execution status digest mismatch")
        expected_counts = {
            "targets": len(expected_targets),
            "selected": len(selected),
            "passed": len(passed),
        }
    else:
        modules = report.get("modules")
        expected_modules = sorted(
            record["path"] for record in records if record["executing_gate"] == "py-test"
        )
        if modules != expected_modules or report.get("command") != [
            "pytest",
            "tools",
            "-q",
            "--strict-markers",
        ]:
            problems.append("py-test: module or command denominator mismatch")
        excluded = report.get("excluded")
        if not isinstance(excluded, list) or not all(
            isinstance(item, dict)
            and isinstance(item.get("case"), str)
            and item.get("reason") == "skipped_or_xfail"
            for item in excluded
        ):
            return [*problems, "py-test: excluded case list is malformed"]
        excluded_names = [item["case"] for item in excluded]
        if sorted(passed + excluded_names) != selected or not passed:
            problems.append("py-test: execution/exclusion differs from selected cases")
        execution_digest = digest({"passed": passed, "excluded": excluded})
        if (
            report.get("execution_digest") != execution_digest
            or stated.get("execution_digest") != execution_digest
        ):
            problems.append("py-test: execution digest mismatch")
        slow = report.get("slow")
        qualification = report.get("qualification")
        if (
            not isinstance(slow, list)
            or not isinstance(qualification, list)
            or not set(slow + qualification) <= set(selected)
        ):
            problems.append("py-test: marked-case denominator mismatch")
        expected_counts = {
            "modules": len(expected_modules),
            "selected": len(selected),
            "passed": len(passed),
            "excluded": len(excluded),
            "slow": len(slow) if isinstance(slow, list) else -1,
            "qualification": len(qualification) if isinstance(qualification, list) else -1,
        }
    for key, count in expected_counts.items():
        if stated.get(key) != str(count):
            problems.append(f"{gate_id}: {key} count differs from execution report")
    return problems
