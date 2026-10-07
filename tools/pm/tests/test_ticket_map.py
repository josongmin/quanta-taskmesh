"""Retired packet inputs keep exact ownership and fail-closed ADR evidence."""

from __future__ import annotations

import json
from copy import deepcopy
from pathlib import Path

import pytest

from tools.release import receipt, ticket_map

REPO = Path(__file__).resolve().parents[3]


def current_map() -> dict:
    return json.loads((REPO / ticket_map.MAP_PATH).read_text(encoding="utf-8"))


def test_compact_map_preserves_original_denominators_and_open_states() -> None:
    rows = ticket_map.ticket_rows(current_map())
    assert sum(len(row["acceptance_ids"]) for row in rows) == 73
    states = {row["id"]: ticket_map.adr_evidence(REPO, row) for row in rows}
    assert all(evidence for _, evidence in states.values())
    assert {
        name for name, (status, _) in states.items() if status == "IMPLEMENTED_UNQUALIFIED"
    } == {
        "SEP21-V01",
        "SEP21-V02",
        "SEP21-R01",
    }
    reasons: list[str] = []
    graph = receipt.closure_report(REPO, current_map(), reasons)
    assert len(graph) == 12
    assert reasons == []
    assert {row["ticket"]: row["status"] for row in graph}["SEP21-V02"] == (
        "IMPLEMENTED_UNQUALIFIED"
    )
    assert all(row["ticket_artifact"]["path"].startswith("docs/adr/") for row in graph)


def test_same_total_acceptance_reallocation_rejects_historical_drift() -> None:
    plan = deepcopy(current_map())
    first, second = plan["tickets"][:2]
    first["acceptance_ids"].pop()
    second["acceptance_ids"].append("SEP21-E01-A06")
    assert sum(len(row["acceptance_ids"]) for row in plan["tickets"]) == 73
    with pytest.raises(ValueError, match="historical acceptance inventory"):
        ticket_map.ticket_rows(plan)


def test_same_23_findings_swapped_between_tickets_rejects_historical_drift() -> None:
    plan = deepcopy(current_map())
    first, second = plan["tickets"][:2]
    first["findings"], second["findings"] = second["findings"], first["findings"]
    assert sorted(finding for row in plan["tickets"] for finding in row["findings"]) == [
        f"TM21-{number:03d}" for number in range(1, 24)
    ]
    with pytest.raises(ValueError, match="historical finding assignment"):
        ticket_map.ticket_rows(plan)


@pytest.mark.parametrize(
    "case",
    [
        "old_schema",
        "duplicate_id",
        "missing_finding",
        "path_escape",
        "acceptance_gap",
        "unknown_status",
        "malformed_status",
    ],
)
def test_invalid_map_rejects(case: str) -> None:
    plan = deepcopy(current_map())
    row = plan["tickets"][0]
    if case == "old_schema":
        plan["schema_version"] = 1
    elif case == "duplicate_id":
        plan["tickets"][1]["id"] = row["id"]
    elif case == "missing_finding":
        row["findings"] = []
    elif case == "path_escape":
        row["file"] = "docs/adr/../../../outside.md"
    elif case == "acceptance_gap":
        row["acceptance_ids"].pop(0)
    elif case == "unknown_status":
        row["closure_status"] = "APPROVED"
    else:
        row["closure_status"] = []
    with pytest.raises(ValueError):
        ticket_map.ticket_rows(plan)


def test_release_checkpoint_can_advance_without_rewriting_historical_status() -> None:
    plan = current_map()
    for row in plan["tickets"]:
        if row["id"] in {"SEP21-V01", "SEP21-V02"}:
            assert ticket_map.adr_evidence(REPO, row)[0] == "IMPLEMENTED_UNQUALIFIED"
            row["closure_status"] = "HOSTED_QUALIFIED"
    reasons: list[str] = []
    graph = receipt.closure_report(REPO, plan, reasons)
    assert not reasons
    assert {row["ticket"] for row in graph if row["status"] == "HOSTED_QUALIFIED"} == {
        "SEP21-V01",
        "SEP21-V02",
    }
    assert all(
        ticket_map.adr_evidence(REPO, row)[0] == "IMPLEMENTED_UNQUALIFIED"
        for row in plan["tickets"]
        if row["id"] in {"SEP21-V01", "SEP21-V02"}
    )


@pytest.mark.parametrize(
    "case",
    [
        "missing_section",
        "duplicate_section",
        "invalid_status",
        "wrong_findings",
        "missing_acceptance",
        "missing_file",
    ],
)
def test_adr_drift_cannot_borrow_evidence_from_another_ticket(tmp_path: Path, case: str) -> None:
    row = deepcopy(current_map()["tickets"][0])
    text = (REPO / row["file"]).read_text(encoding="utf-8")
    if case == "missing_section":
        text = text.replace("## SEP21-C01 —", "## retired-C01 —")
    elif case == "duplicate_section":
        text += "\n## SEP21-C01 — duplicate\n"
    elif case == "invalid_status":
        text = text.replace(
            "- Historical status: LOCALLY_VERIFIED", "- Historical status: APPROVED", 1
        )
    elif case == "wrong_findings":
        text = text.replace("- Findings: TM21-010", "- Findings: TM21-008", 1)
    elif case == "missing_acceptance":
        row["acceptance_ids"].pop()
    target = tmp_path / row["file"]
    target.parent.mkdir(parents=True)
    if case != "missing_file":
        target.write_text(text, encoding="utf-8")
    reasons: list[str] = []
    plan = current_map()
    plan["tickets"][0] = row
    graph = receipt.closure_report(tmp_path, plan, reasons)
    assert graph == [] or graph[0]["status"] == "MISSING"
    assert reasons


def test_evidence_heading_from_neighbor_cannot_close_ticket(tmp_path: Path) -> None:
    row = current_map()["tickets"][0]
    text = (REPO / row["file"]).read_text(encoding="utf-8")
    text = text.replace("### Closure evidence", "### Retired evidence", 1)
    target = tmp_path / row["file"]
    target.parent.mkdir(parents=True)
    target.write_text(text, encoding="utf-8")
    assert ticket_map.adr_evidence(tmp_path, row) == ("LOCALLY_VERIFIED", False)
    reasons: list[str] = []
    graph = receipt.closure_report(tmp_path, current_map(), reasons)
    assert graph[0]["evidence_section"] is False
    assert "upstream ticket SEP21-C01 has no closure/producer evidence section" in reasons


def test_adr_symlink_cannot_replace_historical_evidence(tmp_path: Path) -> None:
    row = current_map()["tickets"][0]
    target = tmp_path / row["file"]
    target.parent.mkdir(parents=True)
    target.symlink_to(REPO / row["file"])
    with pytest.raises(ValueError, match="symlinked"):
        ticket_map.adr_evidence(tmp_path, row)


def test_internal_parent_symlink_cannot_close_a_ticket_without_identity(tmp_path: Path) -> None:
    plan = current_map()
    relocated = tmp_path / "docs/relocated"
    relocated.mkdir(parents=True)
    for name in {row["file"] for row in plan["tickets"]}:
        (relocated / Path(name).name).write_bytes((REPO / name).read_bytes())
    (tmp_path / "docs/adr").symlink_to("relocated", target_is_directory=True)
    with pytest.raises(ValueError, match="parent directory is symlinked"):
        ticket_map.adr_evidence(tmp_path, plan["tickets"][0])
    reasons: list[str] = []
    graph = receipt.closure_report(tmp_path, plan, reasons)
    assert len(graph) == 12
    assert all(row["ticket_artifact"] is None for row in graph)
    assert all(row["status"] == "MISSING" for row in graph)
    assert all(row["evidence_section"] is False for row in graph)
    assert sum("identity unavailable" in reason for reason in reasons) == 12
