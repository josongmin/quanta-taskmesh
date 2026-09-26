#!/usr/bin/env python3
"""Validate documentation structure only; this is not product verification."""

from __future__ import annotations

import json
import re
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SCENARIO = re.compile(r"\b([ABHD])(\d{2})(?:[–-]([ABHD])(\d{2}))?\b")
LINK = re.compile(r"\[[^]]+\]\(([^)]+)\)")
SECTIONS = (
    "## 목적", "## 근거", "## 변경 파일", "## 작업 계획",
    "## DoD", "## 검증", "## 인계 및 중단 조건",
)
MODES = {
    "decision",
    "conditional-implementation",
    "contract-proof",
    "authority-migration",
    "confirmed-defect",
    "proof-first",
    "evidence",
    "integration",
}
PROOF_STATUSES = {"STATIC_MAPPED", "REVIEW_REQUIRED", "RECEIPT_REQUIRED"}
EXTERNAL_STATUSES = {"OPEN", "NOT_APPLICABLE"}


def fail(message: str) -> None:
    raise ValueError(message)


def expand(value: str) -> list[str]:
    result: list[str] = []
    for match in SCENARIO.finditer(value):
        prefix, start, end_prefix, end = match.groups()
        if end is None:
            result.append(f"{prefix}{start}")
        elif prefix == end_prefix and int(start) <= int(end):
            result.extend(f"{prefix}{number:02d}" for number in range(int(start), int(end) + 1))
        else:
            fail(f"invalid scenario range: {match.group()}")
    return result


def incomplete_scenarios(path: Path) -> set[str]:
    text = path.read_text()
    section = text.split("## 테스트 대응 감사 (2026-09-25, 정적)", 1)[1].split("대표 근거:", 1)[0]
    all_ids: list[str] = []
    incomplete: list[str] = []
    for row in section.splitlines():
        if not re.match(r"^\| [ABHD] \(\d+\) \|", row):
            continue
        cells = row.split("|")
        complete = expand(cells[2])
        partial = expand(cells[3])
        gap = expand(cells[4])
        all_ids.extend(complete + partial + gap)
        incomplete.extend(partial + gap)
    if len(all_ids) != 104 or len(set(all_ids)) != 104:
        fail(f"audit cardinality drift: {len(all_ids)} total / {len(set(all_ids))} unique")
    if len(incomplete) != 53 or len(set(incomplete)) != 53:
        fail(
            f"incomplete cardinality drift: {len(incomplete)} total / "
            f"{len(set(incomplete))} unique"
        )
    return set(incomplete)


def check_links(path: Path) -> None:
    for target in LINK.findall(path.read_text()):
        if "://" in target or target.startswith("#"):
            continue
        resolved = path.parent / target.split("#", 1)[0]
        if not resolved.exists():
            fail(f"broken link in {path.name}: {target}")


def main() -> None:
    data = json.loads((ROOT / "plan.json").read_text())
    if data.get("schema_version") != 1:
        fail("unexpected schema")
    expected_plan_status = {
        "status": "PARTIAL",
        "implementation_status": "IMPLEMENTED",
        "qualification_status": "RECEIPT_REQUIRED",
        "external_status": "OPEN",
    }
    for field, expected_value in expected_plan_status.items():
        if data.get(field) != expected_value:
            fail(f"unexpected plan {field}: {data.get(field)!r}")
    tickets = data["tickets"]
    by_id = {ticket["id"]: ticket for ticket in tickets}
    if len(tickets) != 12 or len(by_id) != 12:
        fail("expected 12 unique tickets")
    if not re.fullmatch(r"[0-9a-f]{40}", data.get("audit_head", "")):
        fail("audit_head must be a full SHA-1")
    if not re.fullmatch(r"[0-9a-f]{40}", data.get("audit_tree", "")):
        fail("audit_tree must be a full SHA-1")
    expected = incomplete_scenarios(ROOT / data["checklist"])
    mapped = [scenario for ticket in tickets for scenario in ticket["scenarios"]]
    counts = Counter(mapped)
    if set(mapped) != expected or any(count != 1 for count in counts.values()):
        missing = sorted(expected - set(mapped))
        extra = sorted(set(mapped) - expected)
        duplicates = sorted(key for key, count in counts.items() if count != 1)
        fail(
            f"scenario mapping drift: missing={missing} extra={extra} "
            f"duplicates={duplicates}"
        )
    if by_id["BG25-001"]["scenarios"] or by_id["BG25-012"]["scenarios"]:
        fail("cross-cutting tickets cannot own scenario IDs")
    for ticket in tickets:
        if ticket.get("priority") not in {"P0", "P1"}:
            fail(f"invalid priority: {ticket['id']}")
        if ticket.get("mode") not in MODES:
            fail(f"invalid execution mode: {ticket['id']}")
        if ticket.get("implementation_status") != "IMPLEMENTED":
            fail(f"invalid implementation status: {ticket['id']}")
        if ticket.get("proof_status") not in PROOF_STATUSES:
            fail(f"invalid proof status: {ticket['id']}")
        if ticket.get("external_status") not in EXTERNAL_STATUSES:
            fail(f"invalid external status: {ticket['id']}")
        if not ticket.get("owner"):
            fail(f"missing owner: {ticket['id']}")
        path = ROOT / ticket["file"]
        if not path.is_file():
            fail(f"missing ticket: {path}")
        text = path.read_text()
        expected_status_lines = (
            f"- 구현 상태: {ticket['implementation_status']}",
            f"- 증명 상태: {ticket['proof_status']}",
            f"- 외부 상태: {ticket['external_status']}",
        )
        if not text.startswith(f"# {ticket['id']} ") or any(
            line not in text for line in expected_status_lines
        ):
            fail(f"heading/status mismatch: {ticket['id']}")
        for section in SECTIONS:
            if not re.search(rf"^{re.escape(section)}(?:\s|$)", text, re.M):
                fail(f"missing section: {ticket['id']} {section}")
        for scenario in ticket["scenarios"]:
            if f"`{ticket['id']}-{scenario}`" not in text:
                fail(f"missing acceptance: {ticket['id']}-{scenario}")
        for dependency in ticket["depends_on"]:
            if dependency not in by_id or dependency == ticket["id"]:
                fail(f"invalid dependency: {ticket['id']} -> {dependency}")
        check_links(path)
    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(ticket_id: str) -> None:
        if ticket_id in visiting:
            fail(f"dependency cycle at {ticket_id}")
        if ticket_id in visited:
            return
        visiting.add(ticket_id)
        for dependency in by_id[ticket_id]["depends_on"]:
            visit(dependency)
        visiting.remove(ticket_id)
        visited.add(ticket_id)

    for ticket_id in by_id:
        visit(ticket_id)
    expected_final = set(by_id) - {"BG25-001", "BG25-012"}
    if set(by_id["BG25-012"]["depends_on"]) != expected_final:
        fail("integration ticket must depend on every implementation ticket")
    if by_id["BG25-001"]["proof_status"] != "REVIEW_REQUIRED":
        fail("contract decision ticket must retain explicit review authority")
    if by_id["BG25-012"]["proof_status"] != "RECEIPT_REQUIRED":
        fail("integration ticket must not self-assert execution qualification")
    expected_external_open = {"BG25-001", "BG25-002", "BG25-003"}
    actual_external_open = {
        ticket["id"] for ticket in tickets if ticket["external_status"] == "OPEN"
    }
    if actual_external_open != expected_external_open:
        fail(f"external ownership boundary drift: {sorted(actual_external_open)}")
    coverage = (ROOT / "COVERAGE.md").read_text()
    commands = (ROOT / "COMMANDS.md").read_text()
    for ticket in tickets:
        if not ticket["scenarios"]:
            continue
        row = next(
            (line for line in coverage.splitlines() if line.startswith(f"| [{ticket['id']}]")),
            None,
        )
        if row is None or set(expand(row.split("|")[2])) != set(ticket["scenarios"]):
            fail(f"coverage table drift: {ticket['id']}")
        if not re.search(rf"^\| {re.escape(ticket['id'])} \|", commands, re.M):
            fail(f"verification command row missing: {ticket['id']}")
    predecessor = (
        ROOT.parents[3]
        / "docs/archive/2026-09-25/sep-25-engine-coverage/tickets/plan.json"
    )
    predecessor_data = json.loads(predecessor.read_text())
    if predecessor_data.get("status") != "SUPERSEDED":
        fail("predecessor plan remains an active duplicate authority")
    successor = (predecessor.parent / predecessor_data["superseded_by"]).resolve()
    if successor != (ROOT / "plan.json").resolve():
        fail("predecessor supersession target is not this plan")
    for markdown in ROOT.glob("*.md"):
        check_links(markdown)
        if any(line != line.rstrip() for line in markdown.read_text().splitlines()):
            fail(f"trailing whitespace: {markdown.name}")
    print(
        f"plan PASS: {len(tickets)} tickets; {len(expected)} scenarios mapped once; "
        "DAG and links valid"
    )


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, IndexError) as error:
        print(f"plan FAIL: {error}", file=sys.stderr)
        raise SystemExit(1) from error
