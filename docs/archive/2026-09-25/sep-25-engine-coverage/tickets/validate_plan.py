#!/usr/bin/env python3
"""Check ticket coverage and links against the current audit document.

This validates documentation structure only; it does not run production tests.
"""

from __future__ import annotations

from collections import Counter
import json
from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parent
SCENARIO = re.compile(r"\b([ABHD])(\d{2})(?:[–-]([ABHD])(\d{2}))?\b")
LOCAL_LINK = re.compile(r"\[[^]]+\]\(([^)]+)\)")
REQUIRED_TICKET_SECTIONS = (
    "## 목적",
    "## 변경 파일",
    "## 구현 순서",
    "## DoD",
    "## 계획된 검증",
    "## 인계·중단 조건",
)


def fail(message: str) -> None:
    raise ValueError(message)


def expand_ids(value: str) -> list[str]:
    result: list[str] = []
    for match in SCENARIO.finditer(value):
        prefix, start, end_prefix, end = match.groups()
        if end is None:
            result.append(f"{prefix}{start}")
            continue
        if end_prefix != prefix or int(end) < int(start):
            fail(f"invalid scenario range: {match.group()}")
        result.extend(f"{prefix}{number:02d}" for number in range(int(start), int(end) + 1))
    return result


def audit_open_ids(checklist: Path) -> set[str]:
    text = checklist.read_text()
    try:
        audit = text.split("## 테스트 대응 감사 (2026-09-25, 정적)", 1)[1]
        audit = audit.split("대표 근거:", 1)[0]
    except IndexError as error:
        fail(f"audit table missing: {checklist}")
        raise AssertionError from error
    all_ids: list[str] = []
    open_ids: list[str] = []
    for row in audit.splitlines():
        if not re.match(r"^\| [ABHD] \(\d+\) \|", row):
            continue
        cells = row.split("|")
        all_ids.extend(expand_ids(cells[2]))
        open_ids.extend(expand_ids(cells[3]))
        open_ids.extend(expand_ids(cells[4]))
    all_ids.extend(open_ids)
    if len(all_ids) != 104 or len(set(all_ids)) != 104:
        fail(f"audit all-ID cardinality drift: {len(all_ids)} total, {len(set(all_ids))} unique")
    for prefix, maximum in (("A", 16), ("B", 28), ("H", 35), ("D", 25)):
        expected = {f"{prefix}{number:02d}" for number in range(1, maximum + 1)}
        actual = {item for item in all_ids if item.startswith(prefix)}
        if actual != expected:
            fail(f"audit IDs drift for {prefix}: missing={sorted(expected - actual)} extra={sorted(actual - expected)}")
    if len(open_ids) != 53 or len(set(open_ids)) != 53:
        fail(f"audit P/G cardinality drift: {len(open_ids)} total, {len(set(open_ids))} unique")
    return set(open_ids)


def check_links(markdown: Path) -> None:
    for target in LOCAL_LINK.findall(markdown.read_text()):
        if "://" in target or target.startswith("#"):
            continue
        target = target.split("#", 1)[0]
        if not (markdown.parent / target).exists():
            fail(f"broken link in {markdown.name}: {target}")


def main() -> None:
    data = json.loads((ROOT / "plan.json").read_text())
    if data.get("schema_version") != 1:
        fail("unexpected plan schema/status")
    if data.get("status") == "SUPERSEDED":
        target = ROOT / data["superseded_by"]
        if not target.is_file():
            fail(f"superseding plan missing: {target}")
        print(f"plan SUPERSEDED: use {target.resolve()}")
        return
    if data.get("status") != "PLANNED":
        fail("unexpected plan schema/status")
    tickets = data["tickets"]
    by_id = {ticket["id"]: ticket for ticket in tickets}
    if len(by_id) != len(tickets) or len(tickets) != 12:
        fail("duplicate or missing ticket ID")
    expected = audit_open_ids(ROOT / data["checklist"])
    mapped = [item for ticket in tickets for item in ticket["scenarios"]]
    counts = Counter(mapped)
    if set(mapped) != expected or any(value != 1 for value in counts.values()):
        fail(f"scenario mapping drift: missing={sorted(expected - set(mapped))} extra={sorted(set(mapped) - expected)} duplicates={sorted(item for item, value in counts.items() if value != 1)}")
    if by_id["S25-001"]["scenarios"] or by_id["S25-012"]["scenarios"]:
        fail("cross-cutting tickets cannot own scenario IDs")
    for ticket in tickets:
        ticket_id = ticket["id"]
        path = ROOT / ticket["file"]
        if not path.is_file():
            fail(f"missing ticket file: {path}")
        content = path.read_text()
        if not content.startswith(f"# {ticket_id} ") or "상태: PLANNED" not in content:
            fail(f"ticket heading/status mismatch: {ticket_id}")
        for heading in REQUIRED_TICKET_SECTIONS:
            if not re.search(rf"^{re.escape(heading)}(?:\s|$)", content, re.M):
                fail(f"ticket section missing: {ticket_id} {heading}")
        for item in ticket["scenarios"]:
            if f"`{ticket_id}-{item}`" not in content:
                fail(f"acceptance oracle missing: {ticket_id}-{item}")
        for dependency in ticket["depends_on"]:
            if dependency not in by_id or dependency == ticket_id:
                fail(f"invalid dependency {ticket_id} -> {dependency}")
        check_links(path)
    visited: set[str] = set()
    active: set[str] = set()

    def visit(ticket_id: str) -> None:
        if ticket_id in active:
            fail(f"dependency cycle at {ticket_id}")
        if ticket_id in visited:
            return
        active.add(ticket_id)
        for dependency in by_id[ticket_id]["depends_on"]:
            visit(dependency)
        active.remove(ticket_id)
        visited.add(ticket_id)

    for ticket_id in by_id:
        visit(ticket_id)
    if set(by_id["S25-012"]["depends_on"]) != set(by_id) - {"S25-001", "S25-012"}:
        fail("integration ticket does not depend on every implementation ticket")
    coverage = (ROOT / "COVERAGE.md").read_text()
    for ticket in tickets:
        if not ticket["scenarios"]:
            continue
        row = next((line for line in coverage.splitlines() if line.startswith(f"| [{ticket['id']}]")), None)
        if row is None or set(expand_ids(row.split("|")[2])) != set(ticket["scenarios"]):
            fail(f"coverage table drift: {ticket['id']}")
    for markdown in ROOT.glob("*.md"):
        check_links(markdown)
        if any(line != line.rstrip() for line in markdown.read_text().splitlines()):
            fail(f"trailing whitespace: {markdown.name}")
    print(f"plan PASS: {len(tickets)} tickets; {len(expected)} P/G scenarios mapped once; acyclic dependencies; links intact")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, IndexError) as error:
        print(f"plan FAIL: {error}", file=sys.stderr)
        raise SystemExit(1) from error
