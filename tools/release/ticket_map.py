"""Historical SEP-21 ownership and ADR evidence; never a current proof verdict."""

from __future__ import annotations

import re
from pathlib import Path

MAP_PATH = "docs/evidence/sep21/ticket-map.json"
TICKETS = {
    "SEP21-C01",
    "SEP21-E01",
    "SEP21-E02",
    "SEP21-E03",
    "SEP21-E04",
    "SEP21-H01",
    "SEP21-H02",
    "SEP21-H03",
    "SEP21-V01",
    "SEP21-V02",
    "SEP21-V03",
    "SEP21-R01",
}
STATUSES = {"LOCALLY_VERIFIED", "IMPLEMENTED_UNQUALIFIED"}
# Original 2123a462 finding ownership is historical data, not a mutable
# interpretation of the new map. A 23-ID aggregate check alone misses swaps.
HISTORICAL_FINDINGS = {
    "SEP21-C01": ("TM21-010",),
    "SEP21-E01": ("TM21-008",),
    "SEP21-E02": ("TM21-004",),
    "SEP21-E03": ("TM21-016",),
    "SEP21-E04": ("TM21-001", "TM21-002", "TM21-009", "TM21-023"),
    "SEP21-H01": ("TM21-007",),
    "SEP21-H02": ("TM21-005", "TM21-006", "TM21-019"),
    "SEP21-H03": ("TM21-003", "TM21-015", "TM21-020"),
    "SEP21-V01": ("TM21-011", "TM21-014", "TM21-022"),
    "SEP21-V02": ("TM21-012", "TM21-017"),
    "SEP21-V03": ("TM21-018", "TM21-021"),
    "SEP21-R01": ("TM21-013",),
}
# Immutable per-ticket acceptance inventory from the retained 2123a462 ticket
# documents. A total-only check would allow IDs to move between tickets.
ACCEPTANCE_COUNTS = {
    "SEP21-C01": 7,
    "SEP21-E01": 5,
    "SEP21-E02": 5,
    "SEP21-E03": 6,
    "SEP21-E04": 8,
    "SEP21-H01": 5,
    "SEP21-H02": 6,
    "SEP21-H03": 6,
    "SEP21-V01": 7,
    "SEP21-V02": 6,
    "SEP21-V03": 6,
    "SEP21-R01": 6,
}


def ticket_rows(plan: object) -> list[dict]:
    if not isinstance(plan, dict) or plan.get("schema_version") != 2:
        raise ValueError("ticket map schema must be 2")
    rows = plan.get("tickets")
    if not isinstance(rows, list) or len(rows) != len(TICKETS):
        raise ValueError("ticket map must contain exactly 12 tickets")
    ids: list[str] = []
    findings: list[str] = []
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError("ticket map has malformed ticket")
        ticket_id, document = row.get("id"), row.get("file")
        if not isinstance(ticket_id, str) or ticket_id not in TICKETS:
            raise ValueError("ticket map has unknown ticket ID")
        if not isinstance(document, str):
            raise ValueError(f"{ticket_id}: ADR path missing")
        path = Path(document)
        if (
            path.is_absolute()
            or ".." in path.parts
            or path.parts[:2] != ("docs", "adr")
            or path.suffix != ".md"
        ):
            raise ValueError(f"{ticket_id}: invalid ADR path")
        mapped = row.get("findings")
        if (
            not isinstance(mapped, list)
            or not mapped
            or not all(isinstance(finding, str) for finding in mapped)
        ):
            raise ValueError(f"{ticket_id}: malformed findings")
        historical_findings = HISTORICAL_FINDINGS.get(ticket_id)
        if historical_findings is None or mapped != list(historical_findings):
            raise ValueError(f"{ticket_id}: historical finding assignment differs")
        acceptance = row.get("acceptance_ids")
        if (
            not isinstance(acceptance, list)
            or not acceptance
            or not all(isinstance(item, str) for item in acceptance)
        ):
            raise ValueError(f"{ticket_id}: acceptance inventory missing")
        historical_count = ACCEPTANCE_COUNTS.get(ticket_id)
        if historical_count is None:
            raise ValueError(f"{ticket_id}: historical acceptance inventory missing")
        expected_acceptance = [
            f"{ticket_id}-A{number:02d}" for number in range(1, historical_count + 1)
        ]
        if acceptance != expected_acceptance:
            raise ValueError(f"{ticket_id}: historical acceptance inventory differs")
        ids.append(ticket_id)
        findings.extend(mapped)
    if len(set(ids)) != len(TICKETS):
        raise ValueError("ticket map has duplicate or missing tickets")
    if sorted(findings) != [f"TM21-{number:03d}" for number in range(1, 24)]:
        raise ValueError("ticket map must map exactly all 23 findings once")
    return rows


def adr_evidence(root: Path, row: dict) -> tuple[str, bool]:
    path = root / row["file"]
    if path.is_symlink() or not path.resolve().is_relative_to(root.resolve()):
        raise ValueError(f"{row['id']}: ADR escapes repository or is symlinked")
    text = path.read_text(encoding="utf-8")
    sections = re.findall(
        rf"^## {re.escape(row['id'])} — [^\n]+\n(.*?)(?=^## |\Z)",
        text,
        re.MULTILINE | re.DOTALL,
    )
    if len(sections) != 1:
        raise ValueError(f"{row['id']}: missing or duplicate ADR section")
    section = sections[0]
    statuses = re.findall(r"^- Historical status: ([A-Z_]+)$", section, re.MULTILINE)
    if len(statuses) != 1 or statuses[0] not in STATUSES:
        raise ValueError(f"{row['id']}: invalid historical status")
    findings = re.findall(r"^- Findings: (.+)$", section, re.MULTILINE)
    if findings != [", ".join(row["findings"])]:
        raise ValueError(f"{row['id']}: ADR finding mapping differs")
    counts = re.findall(r"records ([0-9]+) acceptance IDs\.", section)
    if counts != [str(len(row["acceptance_ids"]))]:
        raise ValueError(f"{row['id']}: ADR acceptance inventory differs")
    evidence = (
        re.search(r"^### (?:Closure|Producer) evidence\n\s*\S", section, re.MULTILINE) is not None
    )
    return statuses[0], evidence
