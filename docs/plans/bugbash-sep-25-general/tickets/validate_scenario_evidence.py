#!/usr/bin/env python3
"""Fail-closed validation for the BG25 104-scenario evidence manifest."""

from __future__ import annotations

import json
import re
import shlex
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[3]
MANIFEST = ROOT / "scenario-evidence.json"
PLAN = ROOT / "plan.json"
CHECKLIST = REPO / "docs/misc/tmp-engine-checklist-sep-25.md"
VALID_STATUS = {"MAPPED", "OPEN", "NOT_RUN", "OUT_OF_SCOPE"}
VALID_ORIGIN = {"K", "P", "G"}
NIGHTLY_GATES = {
    "modelcheck",
    "tsan",
    "fuzz",
    "coverage-report",
    "bench-iai",
    "mutants-critical",
    "mutants-generated",
}


def fail(message: str) -> None:
    raise ValueError(message)


def checklist_ids() -> set[str]:
    return set(re.findall(r"^\| ([ABHD]\d{2}) \|", CHECKLIST.read_text(), re.M))


def repository_file(relative: str, context: str) -> Path:
    path = Path(relative)
    if path.is_absolute() or ".." in path.parts:
        fail(f"{context}: path must be repository-relative without traversal: {relative!r}")
    candidate = REPO / path
    try:
        repository = REPO.resolve(strict=True)
        resolved = candidate.resolve(strict=True)
    except FileNotFoundError:
        fail(f"{context}: missing file {relative!r}")
    if repository != resolved and repository not in resolved.parents:
        fail(f"{context}: path resolves outside the repository: {relative!r}")
    if not candidate.is_file():
        fail(f"{context}: not a file: {relative!r}")
    return candidate


def test_case_exists(path: Path, case: str) -> bool:
    lines = path.read_text(encoding="utf-8").splitlines()
    declaration = re.compile(rf"^\s*(?:async\s+)?fn\s+{re.escape(case)}\s*\(")
    attribute = re.compile(r"^\s*#\[(?P<body>[^]]+)]\s*$")
    for index, line in enumerate(lines):
        if declaration.search(line) is None:
            continue
        cursor = index - 1
        is_test = False
        while cursor >= 0:
            match = attribute.fullmatch(lines[cursor])
            if match is None:
                break
            body = match.group("body")
            is_test |= (
                body == "test"
                or body == "tokio::test"
                or body.startswith("tokio::test(")
            )
            cursor -= 1
        if is_test:
            return True
    return False


def recipe_body(justfile: str, recipe: str) -> str:
    match = re.search(
        rf"^{re.escape(recipe)}:\s*\n(?P<body>(?:^[ \t]+.*\n?)*)",
        justfile,
        re.M,
    )
    if match is None:
        fail(f"missing Just recipe {recipe!r}")
    return match.group("body")


def recipe_executes_test(body: str, target: str, case: str) -> bool:
    selector = ["--test", target, case, "--", "--exact"]
    for line in body.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        try:
            tokens = shlex.split(stripped)
        except ValueError:
            continue
        while tokens and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*=.*", tokens[0]):
            tokens.pop(0)
        if tokens[:2] != ["cargo", "test"]:
            continue
        if any(token in {"&&", "||", ";", "|", "&"} for token in tokens):
            continue
        if any(tokens[index : index + len(selector)] == selector for index in range(len(tokens))):
            return True
    return False


def evidence_cases(row: dict[str, object], scenario: str) -> list[tuple[str, str]]:
    target = row.get("target")
    case = row.get("case")
    if not isinstance(target, str) or not isinstance(case, str) or not case.strip():
        fail(f"{scenario}: target and case must be non-empty strings")
    entries = [(target, case)]
    supporting = row.get("supporting_cases", [])
    if not isinstance(supporting, list):
        fail(f"{scenario}: supporting_cases must be a list")
    for entry in supporting:
        if not isinstance(entry, dict) or set(entry) != {"target", "case"}:
            fail(f"{scenario}: each supporting case needs exactly target and case")
        support_target = entry["target"]
        support_case = entry["case"]
        if (
            not isinstance(support_target, str)
            or not isinstance(support_case, str)
            or not support_case.strip()
        ):
            fail(f"{scenario}: supporting target and case must be non-empty strings")
        entries.append((support_target, support_case))
    if len(set(entries)) != len(entries):
        fail(f"{scenario}: duplicate primary/supporting test case")
    return entries


def validate_nightly(nightly: object) -> None:
    if not isinstance(nightly, list) or not all(isinstance(row, dict) for row in nightly):
        fail("nightly qualification inventory must be a list of objects")
    required = {"gate", "status", "reason"}
    for row in nightly:
        if set(row) != required:
            drift = sorted(set(row) ^ required)
            fail(f"nightly qualification row has unknown or missing fields: {drift}")
    gates = [row.get("gate") for row in nightly]
    if len(gates) != len(NIGHTLY_GATES) or len(set(gates)) != len(gates):
        fail("nightly qualification inventory contains missing or duplicate gates")
    if set(gates) != NIGHTLY_GATES:
        fail("nightly qualification inventory drift")
    if any(
        row.get("status") != "NOT_RUN"
        or not isinstance(row.get("reason"), str)
        or not row["reason"].strip()
        for row in nightly
    ):
        fail("unexecuted nightly gates must remain explicit NOT_RUN entries with reasons")


def main() -> None:
    data = json.loads(MANIFEST.read_text(encoding="utf-8"))
    plan = json.loads(PLAN.read_text(encoding="utf-8"))
    if data.get("schema_version") != 3:
        fail("scenario evidence schema_version must be 3")
    expected_semantics = {
        "MAPPED": (
            "static candidate mapping only: every named target and test case exists; "
            "semantic sufficiency and execution are not asserted"
        ),
        "OPEN": "the scenario has no accepted implementation or oracle yet",
        "NOT_RUN": "the selected execution proof was not run",
        "OUT_OF_SCOPE": "the scenario is outside the accepted product boundary",
    }
    if data.get("status_semantics") != expected_semantics:
        fail("status semantics must keep static coverage separate from execution")
    rows = data.get("scenarios")
    if not isinstance(rows, list):
        fail("scenarios must be a list")

    expected = checklist_ids()
    ids = [row.get("id") for row in rows if isinstance(row, dict)]
    if len(rows) != 104 or len(ids) != 104 or set(ids) != expected:
        fail(
            f"scenario set drift: rows={len(rows)} unique={len(set(ids))} "
            f"missing={sorted(expected - set(ids))} extra={sorted(set(ids) - expected)}"
        )
    duplicates = sorted(key for key, count in Counter(ids).items() if count != 1)
    if duplicates:
        fail(f"duplicate scenario ids: {duplicates}")

    owners = {
        scenario: ticket["id"]
        for ticket in plan["tickets"]
        for scenario in ticket["scenarios"]
    }
    origins = Counter()
    statuses = Counter()
    inventory = json.loads((REPO / "tools/gates/inventory.json").read_text())
    known_gates = {gate["id"] for gate in inventory["gates"]}
    justfile = (REPO / "Justfile").read_text(encoding="utf-8")
    rayon_recipe = recipe_body(justfile, "test-rayon")

    required = {
        "id",
        "origin",
        "status",
        "owner_ticket",
        "contract",
        "target",
        "case",
        "oracle",
        "feature",
        "platform",
        "gate",
        "source",
    }
    optional = {"supporting_cases"}
    for row in rows:
        scenario = row["id"]
        fields = set(row)
        if not required.issubset(fields) or fields - required - optional:
            drift = (required - fields) | (fields - required - optional)
            fail(f"{scenario}: unknown or missing fields: {sorted(drift)}")
        if row["origin"] not in VALID_ORIGIN:
            fail(f"{scenario}: invalid origin {row['origin']!r}")
        if row["status"] not in VALID_STATUS:
            fail(f"{scenario}: invalid status {row['status']!r}")
        origins[row["origin"]] += 1
        statuses[row["status"]] += 1
        expected_owner = owners.get(scenario)
        if row["origin"] in {"P", "G"} and row["owner_ticket"] != expected_owner:
            fail(f"{scenario}: owner {row['owner_ticket']!r} != {expected_owner!r}")
        if row["origin"] == "K" and row["owner_ticket"] is not None:
            fail(f"{scenario}: baseline K row must not claim an implementation owner")
        if row["contract"] != f"docs/misc/tmp-engine-checklist-sep-25.md::{scenario}":
            fail(f"{scenario}: contract anchor drift")
        if not isinstance(row["oracle"], str) or not row["oracle"].strip():
            fail(f"{scenario}: empty independent oracle")
        if row["feature"] not in {"default", "rayon"} or row["platform"] != "any":
            fail(f"{scenario}: unsupported feature/platform selector")
        if row["gate"] not in known_gates:
            fail(f"{scenario}: selecting gate {row['gate']!r} is not inventoried")
        cases = evidence_cases(row, scenario)
        sources = row["source"]
        evidence_targets = {target for target, _case in cases}
        if (
            not isinstance(sources, list)
            or not sources
            or not evidence_targets.issubset(set(sources))
        ):
            fail(f"{scenario}: source must include every evidence target")
        for source in sources:
            if not isinstance(source, str):
                fail(f"{scenario}: source paths must be strings")
            repository_file(source, f"{scenario}: source")
        if row["status"] == "MAPPED":
            for case_target, case_name in cases:
                target = repository_file(case_target, f"{scenario}: target")
                if not test_case_exists(target, case_name):
                    fail(
                        f"{scenario}: MAPPED target/case does not exist: "
                        f"{case_target}::{case_name}"
                    )
                if row["feature"] == "rayon" and (
                    row["gate"] != "test-rayon"
                    or not recipe_executes_test(rayon_recipe, Path(case_target).stem, case_name)
                ):
                    fail(f"{scenario}: rayon selector is not executed by test-rayon")

    expected_origins = {"K": 51, "P": 34, "G": 19}
    if dict(origins) != expected_origins or data.get("origin_counts") != expected_origins:
        fail(f"origin counts drift: {dict(origins)}")
    expected_statuses = {status: statuses.get(status, 0) for status in sorted(VALID_STATUS)}
    if data.get("status_summary") != expected_statuses:
        fail(f"status summary drift: {expected_statuses}")
    if data.get("scenario_count") != 104:
        fail("scenario_count must be 104")

    validate_nightly(data.get("qualification", {}).get("nightly", []))

    print(
        "scenario mapping PASS: 104 unique MAPPED rows; "
        "origins K=51 P=34 G=19; targets/cases/sources/gates valid; execution not asserted"
    )


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"scenario evidence FAIL: {error}", file=sys.stderr)
        raise SystemExit(1) from error
