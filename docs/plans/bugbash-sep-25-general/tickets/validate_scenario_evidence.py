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
VALID_STATUS = {"PASS", "OPEN", "NOT_RUN", "OUT_OF_SCOPE"}
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


def main() -> None:
    data = json.loads(MANIFEST.read_text(encoding="utf-8"))
    plan = json.loads(PLAN.read_text(encoding="utf-8"))
    if data.get("schema_version") != 1:
        fail("scenario evidence schema_version must be 1")
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
        sources = row["source"]
        if not isinstance(sources, list) or not sources or row["target"] not in sources:
            fail(f"{scenario}: source must include its target")
        for source in sources:
            if not isinstance(source, str) or not (REPO / source).is_file():
                fail(f"{scenario}: missing source {source!r}")
        if row["status"] == "PASS":
            target = REPO / row["target"]
            if not target.is_file() or not test_case_exists(target, row["case"]):
                fail(f"{scenario}: PASS target/case does not exist: {row['target']}::{row['case']}")

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
        "scenario evidence PASS: 104 unique rows; "
        "origins K=51 P=34 G=19; targets/cases/sources/gates valid"
    )


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"scenario evidence FAIL: {error}", file=sys.stderr)
        raise SystemExit(1) from error
