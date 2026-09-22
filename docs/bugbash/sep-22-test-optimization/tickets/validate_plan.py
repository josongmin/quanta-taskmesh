#!/usr/bin/env python3
"""Fail-closed structural validator for the SEP-22 optimization tickets."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path
from typing import Optional

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
PLAN_PATH = HERE / "plan.json"
README_PATH = HERE / "README.md"
REQUIRED_HEADINGS = {
    "## 목적",
    "## RCA",
    "## 확정 근거",
    "## 목표 불변식",
    "## 구현 플랜",
    "## 테스트와 intentional negative",
    "## DoD",
    "## 금지되는 임시방편",
    "## 검증 명령",
    "## Stop/reopen 조건",
}
ALLOWED_STATUSES = {"PLANNED", "IN_PROGRESS", "IMPLEMENTED_UNQUALIFIED", "LOCALLY_VERIFIED"}
SOURCE_CITATION = re.compile(
    r"`(?P<path>[A-Za-z0-9_./-]+):(?P<ranges>\d+(?:-\d+)?(?:,\d+(?:-\d+)?)*)`"
)
LOCAL_LINK = re.compile(r"\[[^]]+\]\((?P<target>[^)]+)\)")


def fail(message: str) -> None:
    raise SystemExit(f"SEP-22 plan invalid: {message}")


def git_bytes(*args: str) -> bytes:
    process = subprocess.run(["git", *args], cwd=REPO, check=False, capture_output=True)
    if process.returncode != 0:
        fail(f"git {' '.join(args)} failed: {process.stderr.decode().strip()}")
    return process.stdout


def git_value(*args: str) -> str:
    return git_bytes(*args).decode().strip()


def sha256(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def validate_links(document: Path, body: str) -> int:
    count = 0
    for match in LOCAL_LINK.finditer(body):
        target = match.group("target").split("#", 1)[0]
        if not target or "://" in target or target.startswith("mailto:"):
            continue
        count += 1
        if not (document.parent / target).resolve().is_file():
            fail(f"{document.name}: missing local link {target}")
    return count


def validate_citations(document: Path, body: str) -> int:
    count = 0
    for match in SOURCE_CITATION.finditer(body):
        count += 1
        path = REPO / match.group("path")
        if not path.is_file():
            fail(f"{document.name}: missing cited source {match.group('path')}")
        lines = len(path.read_text(encoding="utf-8").splitlines())
        for value in match.group("ranges").split(","):
            bounds = [int(part) for part in value.split("-")]
            if bounds[0] < 1 or bounds[-1] < bounds[0] or bounds[-1] > lines:
                fail(f"{document.name}: invalid citation {match.group(0)} for {lines} lines")
    return count


def find_cycle(graph: dict[str, list[str]]) -> Optional[list[str]]:  # noqa: UP045
    visiting: list[str] = []
    visited: set[str] = set()

    def walk(node: str) -> Optional[list[str]]:  # noqa: UP045
        if node in visiting:
            start = visiting.index(node)
            return [*visiting[start:], node]
        if node in visited:
            return None
        visiting.append(node)
        for dependency in graph[node]:
            cycle = walk(dependency)
            if cycle:
                return cycle
        visiting.pop()
        visited.add(node)
        return None

    for node in graph:
        cycle = walk(node)
        if cycle:
            return cycle
    return None


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--structure-only", action="store_true")
    args = parser.parse_args()

    plan = json.loads(PLAN_PATH.read_text(encoding="utf-8"))
    if plan.get("schema_version") != 1:
        fail("schema_version must be 1")

    source = plan.get("source", {})
    if not args.structure_only:
        if git_value("branch", "--show-current") != source.get("branch"):
            fail("plan branch is stale")
        if git_value("rev-parse", "HEAD") != source.get("head"):
            fail("plan HEAD is stale")
        if git_value("rev-parse", "HEAD^{tree}") != source.get("tree"):
            fail("plan tree is stale")
        prefix = source.get("audit_output_prefix")
        if not isinstance(prefix, str) or not prefix:
            fail("audit_output_prefix must be a non-empty string")
        tracked_status = git_bytes(
            "status", "--porcelain=v1", "--untracked-files=no", "--", ".", f":(exclude){prefix}**"
        ).decode()
        dirty_lines = tracked_status.splitlines()
        dirty_count = len(dirty_lines)
        if dirty_count != source.get("tracked_dirty_count"):
            fail(f"tracked dirty count changed: {dirty_count}")
        dirty_paths = sorted(line[3:] for line in dirty_lines)
        if dirty_paths != sorted(source.get("tracked_dirty_paths", [])):
            fail(f"tracked dirty paths changed: {dirty_paths}")
        tracked_diff = git_bytes("diff", "--binary", "HEAD", "--", ".", f":(exclude){prefix}**")
        if sha256(tracked_diff) != source.get("tracked_diff_sha256"):
            fail("tracked source diff changed")

    findings_path = (HERE / plan.get("findings_source", "")).resolve()
    if not findings_path.is_file():
        fail("findings source is missing")
    findings_body = findings_path.read_text(encoding="utf-8")
    finding_ids = re.findall(r"^\| (TO-\d{2}) \| P[012] \|", findings_body, re.MULTILINE)
    if len(finding_ids) != len(set(finding_ids)) or not finding_ids:
        fail("findings source must contain unique TO rows")

    tickets = plan.get("tickets")
    if not isinstance(tickets, list) or not tickets:
        fail("tickets must be a non-empty list")
    ids = [ticket.get("id") for ticket in tickets]
    files = [ticket.get("file") for ticket in tickets]
    mapped_findings = [ticket.get("finding") for ticket in tickets]
    if None in ids or len(ids) != len(set(ids)):
        fail("ticket IDs must be present and unique")
    if None in files or len(files) != len(set(files)):
        fail("ticket files must be present and unique")
    if sorted(mapped_findings) != sorted(finding_ids):
        fail("ticket finding mapping must cover every TO row exactly once")

    id_set = set(ids)
    graph: dict[str, list[str]] = {}
    acceptance_ids: list[str] = []
    citation_count = 0
    link_count = validate_links(README_PATH, README_PATH.read_text(encoding="utf-8"))
    link_count += validate_links(findings_path, findings_body)
    for ticket in tickets:
        ticket_id = ticket["id"]
        if ticket.get("status") not in ALLOWED_STATUSES:
            fail(f"{ticket_id}: invalid status")
        if ticket.get("priority") not in {"P0", "P1", "P2"}:
            fail(f"{ticket_id}: invalid priority")
        if not isinstance(ticket.get("lane"), str) or not ticket["lane"]:
            fail(f"{ticket_id}: lane is required")
        if not isinstance(ticket.get("owned_paths"), list) or not ticket["owned_paths"]:
            fail(f"{ticket_id}: owned_paths must be non-empty")
        for owned in ticket["owned_paths"]:
            if not (REPO / owned).is_file():
                fail(f"{ticket_id}: owned path does not exist: {owned}")
        dependencies = ticket.get("depends_on")
        if not isinstance(dependencies, list):
            fail(f"{ticket_id}: depends_on must be a list")
        unknown = sorted(set(dependencies) - id_set)
        if ticket_id in dependencies or unknown:
            fail(f"{ticket_id}: invalid dependencies {dependencies}")
        graph[ticket_id] = dependencies

        document = HERE / ticket["file"]
        if not document.is_file():
            fail(f"{ticket_id}: missing ticket file")
        body = document.read_text(encoding="utf-8")
        if not body.startswith(f"# {ticket_id} "):
            fail(f"{ticket_id}: title does not match")
        for heading in REQUIRED_HEADINGS:
            if heading not in body:
                fail(f"{ticket_id}: missing heading {heading}")
        if f"- 상태: {ticket['status']}" not in body:
            fail(f"{ticket_id}: status metadata differs from plan")
        if f"- finding: {ticket['finding']}" not in body:
            fail(f"{ticket_id}: finding metadata differs from plan")
        if f"- write lane: `{ticket['lane']}`" not in body:
            fail(f"{ticket_id}: lane metadata differs from plan")
        ids_in_body = re.findall(rf"\b{re.escape(ticket_id)}-A\d{{2}}\b", body)
        if len(set(ids_in_body)) < 3:
            fail(f"{ticket_id}: requires at least three unique acceptance IDs")
        acceptance_ids.extend(ids_in_body)
        citation_count += validate_citations(document, body)
        link_count += validate_links(document, body)

    cycle = find_cycle(graph)
    if cycle:
        fail(f"dependency cycle: {' -> '.join(cycle)}")
    if len(acceptance_ids) != len(set(acceptance_ids)):
        fail("acceptance IDs must be globally unique and appear once")

    serialized = plan.get("lane_serialization", {})
    for lane, order in serialized.items():
        actual = [ticket["id"] for ticket in tickets if ticket["lane"] == lane]
        if order != actual:
            fail(f"lane serialization for {lane} differs from ticket order")
        for previous, current in zip(order, order[1:]):
            if previous not in graph[current]:
                fail(f"{current} must depend on prior same-lane ticket {previous}")

    waves = plan.get("waves")
    flattened = [ticket_id for wave in waves or [] for ticket_id in wave]
    if len(flattened) != len(set(flattened)) or set(flattened) != id_set:
        fail("waves must schedule every ticket exactly once")
    wave_index = {ticket_id: index for index, wave in enumerate(waves) for ticket_id in wave}
    for ticket_id, dependencies in graph.items():
        if any(wave_index[dependency] >= wave_index[ticket_id] for dependency in dependencies):
            fail(f"{ticket_id}: dependency is not in an earlier wave")

    print(
        "SEP-22 plan valid: "
        f"tickets={len(tickets)} findings={len(finding_ids)} "
        f"acceptance={len(acceptance_ids)} citations={citation_count} links={link_count}"
    )


if __name__ == "__main__":
    main()
