#!/usr/bin/env python3
"""Fail-closed validation for the BG25 104-scenario evidence manifest."""

from __future__ import annotations

from collections import Counter
import json
from pathlib import Path
import re
import sys

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


def test_case_exists(path: Path, case: str) -> bool:
    text = path.read_text(encoding="utf-8")
    declaration = re.compile(rf"^\s*(?:async\s+)?fn\s+{re.escape(case)}\s*\(", re.M)
    return declaration.search(text) is not None


def recipe_body(justfile: str, recipe: str) -> str:
    match = re.search(
        rf"^{re.escape(recipe)}:\s*\n(?P<body>(?:^[ \t]+.*\n?)*)",
        justfile,
        re.M,
    )
    if match is None:
        fail(f"missing Just recipe {recipe!r}")
    return match.group("body")


def main() -> None:
    data = json.loads(MANIFEST.read_text(encoding="utf-8"))
    plan = json.loads(PLAN.read_text(encoding="utf-8"))
    if data.get("schema_version") != 2:
        fail("scenario evidence schema_version must be 2")
    expected_semantics = {
        "MAPPED": "static candidate mapping only: the named target and test case exist; semantic sufficiency and execution are not asserted",
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
    for row in rows:
        scenario = row["id"]
        if set(row) != required:
            fail(f"{scenario}: unknown or missing fields: {sorted(set(row) ^ required)}")
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
        if row["feature"] == "rayon":
            selector = (
                f"--test {Path(row['target']).stem} {row['case']} -- --exact"
            )
            if row["gate"] != "test-rayon" or selector not in rayon_recipe:
                fail(f"{scenario}: rayon selector is not executed by test-rayon")
        sources = row["source"]
        if not isinstance(sources, list) or not sources or row["target"] not in sources:
            fail(f"{scenario}: source must include its target")
        for source in sources:
            if not isinstance(source, str) or not (REPO / source).is_file():
                fail(f"{scenario}: missing source {source!r}")
        if row["status"] == "MAPPED":
            target = REPO / row["target"]
            if not target.is_file() or not test_case_exists(target, row["case"]):
                fail(
                    f"{scenario}: MAPPED target/case does not exist: "
                    f"{row['target']}::{row['case']}"
                )

    expected_origins = {"K": 51, "P": 34, "G": 19}
    if dict(origins) != expected_origins or data.get("origin_counts") != expected_origins:
        fail(f"origin counts drift: {dict(origins)}")
    expected_statuses = {status: statuses.get(status, 0) for status in sorted(VALID_STATUS)}
    if data.get("status_summary") != expected_statuses:
        fail(f"status summary drift: {expected_statuses}")
    if data.get("scenario_count") != 104:
        fail("scenario_count must be 104")

    nightly = data.get("qualification", {}).get("nightly", [])
    nightly_by_gate = {row.get("gate"): row for row in nightly if isinstance(row, dict)}
    if set(nightly_by_gate) != NIGHTLY_GATES:
        fail("nightly qualification inventory drift")
    if any(row.get("status") != "NOT_RUN" or not row.get("reason") for row in nightly):
        fail("unexecuted nightly gates must remain explicit NOT_RUN entries with reasons")

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
