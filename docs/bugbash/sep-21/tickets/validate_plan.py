#!/usr/bin/env python3
"""Validate the SEP-21 remediation plan's internal authority boundaries."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
PLAN_PATH = HERE / "plan.json"
README_PATH = HERE / "README.md"
REQUIRED_HEADINGS = {
    "## 목적",
    "## RCA",
    "## 확정 근거",
    "## 작업 플랜",
    "## DoD",
    "## 금지되는 임시방편",
}
SOURCE_CITATION = re.compile(
    r"`(?P<path>[A-Za-z0-9_./-]+):"
    r"(?P<ranges>\d+(?:-\d+)?(?:,\d+(?:-\d+)?)*)`"
)
LOCAL_LINK = re.compile(r"\[[^]]+\]\((?P<target>[^)]+)\)")
ALLOWED_STATUSES = {
    "PLANNED",
    "IN_PROGRESS",
    "IMPLEMENTED_UNQUALIFIED",
    "LOCALLY_VERIFIED",
    "HOSTED_QUALIFIED",
}


def fail(message: str) -> None:
    raise SystemExit(f"SEP-21 plan invalid: {message}")


def git_bytes(*args: str) -> bytes:
    process = subprocess.run(
        ["git", *args],
        cwd=REPO,
        check=False,
        capture_output=True,
    )
    if process.returncode != 0:
        fail(f"git {' '.join(args)} failed: {process.stderr.decode().strip()}")
    return process.stdout


def git_value(*args: str) -> str:
    return git_bytes(*args).decode().strip()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def validate_citations(document: Path, body: str) -> int:
    count = 0
    for match in SOURCE_CITATION.finditer(body):
        count += 1
        cited_path = REPO / match.group("path")
        if not cited_path.is_file():
            fail(f"{document.name}: cited source does not exist: {match.group('path')}")
        line_count = len(cited_path.read_text(encoding="utf-8").splitlines())
        for line_range in match.group("ranges").split(","):
            bounds = [int(value) for value in line_range.split("-")]
            if bounds[0] < 1 or bounds[-1] < bounds[0] or bounds[-1] > line_count:
                fail(
                    f"{document.name}: invalid citation {match.group(0)} "
                    f"for {line_count}-line source"
                )
    return count


def validate_local_links(document: Path, body: str) -> int:
    count = 0
    for match in LOCAL_LINK.finditer(body):
        target = match.group("target").split("#", 1)[0]
        if not target or "://" in target or target.startswith("mailto:"):
            continue
        count += 1
        if not (document.parent / target).resolve().is_file():
            fail(f"{document.name}: local link does not exist: {target}")
    return count


def find_cycle(graph: dict[str, list[str]]) -> list[str] | None:
    visiting: list[str] = []
    visited: set[str] = set()

    def walk(node: str) -> list[str] | None:
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


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--structure-only",
        action="store_true",
        help="validate plan structure after implementation has intentionally changed the baseline",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
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

        output_prefixes = source.get("audit_output_prefixes")
        if not isinstance(output_prefixes, list) or not output_prefixes:
            fail("audit_output_prefixes must be a non-empty list")
        untracked = git_value("ls-files", "--others", "--exclude-standard").splitlines()
        source_untracked = sorted(
            path
            for path in untracked
            if not any(path.startswith(prefix) for prefix in output_prefixes)
        )
        if source_untracked != source.get("untracked_source_files"):
            fail(f"untracked audit inputs changed: {source_untracked}")
        tracked_dirty_count = len(
            git_value(
                "status",
                "--porcelain=v1",
                "--untracked-files=no",
                "--",
                ".",
                ":(exclude)docs/bugbash/sep-21/**",
            ).splitlines()
        )
        if tracked_dirty_count != source.get("tracked_dirty_count"):
            fail(f"tracked dirty count changed: {tracked_dirty_count}")
        tracked_diff = git_bytes(
            "diff",
            "--binary",
            "HEAD",
            "--",
            ".",
            ":(exclude)docs/bugbash/sep-21/**",
        )
        if sha256_bytes(tracked_diff) != source.get("tracked_diff_sha256"):
            fail("audited tracked working-tree diff changed")

    checklist_path = REPO / source.get("checklist", "")
    if not checklist_path.is_file():
        fail(f"checklist does not exist: {checklist_path}")
    if sha256_file(checklist_path) != source.get("checklist_sha256"):
        fail("checklist digest changed")

    findings_path = REPO / source.get("findings", "")
    if not findings_path.is_file():
        fail(f"findings source does not exist: {findings_path}")
    if sha256_file(findings_path) != source.get("findings_sha256"):
        fail("findings digest changed")
    findings_body = findings_path.read_text(encoding="utf-8")
    citation_count = 0
    if not args.structure_only:
        citation_count = validate_citations(findings_path, findings_body)
    link_count = validate_local_links(findings_path, findings_body)
    finding_ids = re.findall(
        r"^### (TM21-\d{3})\b",
        findings_body,
        re.MULTILINE,
    )
    if len(finding_ids) != len(set(finding_ids)):
        fail("findings source has duplicate IDs")
    summary_priorities = dict(
        re.findall(r"^\| (TM21-\d{3}) \| (P[012]) \|", findings_body, re.MULTILINE)
    )
    if set(summary_priorities) != set(finding_ids):
        fail("findings summary inventory differs from detail sections")
    section_priorities: dict[str, str] = {}
    current_priority: str | None = None
    for line in findings_body.splitlines():
        priority_heading = re.match(r"## (P[012])\b", line)
        if priority_heading:
            current_priority = priority_heading.group(1)
            continue
        finding_heading = re.match(r"### (TM21-\d{3})\b", line)
        if finding_heading:
            if current_priority is None:
                fail(f"{finding_heading.group(1)}: no priority section")
            section_priorities[finding_heading.group(1)] = current_priority
    if summary_priorities != section_priorities:
        fail("finding summary priorities differ from detail sections")

    tickets = plan.get("tickets")
    if not isinstance(tickets, list) or not tickets:
        fail("tickets must be a non-empty list")
    ids = [ticket.get("id") for ticket in tickets]
    files = [ticket.get("file") for ticket in tickets]
    if len(ids) != len(set(ids)) or None in ids:
        fail("ticket IDs must be present and unique")
    if len(files) != len(set(files)) or None in files:
        fail("ticket files must be present and unique")

    id_set = set(ids)
    graph: dict[str, list[str]] = {}
    mapped_findings: list[str] = []
    acceptance_ids: list[str] = []
    readme = README_PATH.read_text(encoding="utf-8")
    link_count += validate_local_links(README_PATH, readme)
    for ticket in tickets:
        ticket_id = ticket["id"]
        dependencies = ticket.get("depends_on")
        if not isinstance(dependencies, list):
            fail(f"{ticket_id}: depends_on must be a list")
        if ticket_id in dependencies:
            fail(f"{ticket_id}: self dependency")
        unknown = sorted(set(dependencies) - id_set)
        if unknown:
            fail(f"{ticket_id}: unknown dependencies {unknown}")
        graph[ticket_id] = dependencies

        ticket_findings = ticket.get("findings")
        if not isinstance(ticket_findings, list) or not ticket_findings:
            fail(f"{ticket_id}: findings must be non-empty")
        mapped_findings.extend(ticket_findings)

        ticket_file = HERE / ticket["file"]
        if not ticket_file.is_file():
            fail(f"{ticket_id}: missing {ticket['file']}")
        body = ticket_file.read_text(encoding="utf-8")
        if not args.structure_only:
            citation_count += validate_citations(ticket_file, body)
        link_count += validate_local_links(ticket_file, body)
        if not body.startswith(f"# {ticket_id} —"):
            fail(f"{ticket_id}: title does not match ID")
        status_match = re.search(r"^- 상태: ([A-Z_]+)$", body, re.MULTILINE)
        if status_match is None or status_match.group(1) not in ALLOWED_STATUSES:
            fail(f"{ticket_id}: invalid lifecycle status")
        if status_match.group(1) == "HOSTED_QUALIFIED" and ticket_id != "SEP21-R01":
            fail(f"{ticket_id}: only SEP21-R01 may be HOSTED_QUALIFIED")
        if f"- write lane: `{ticket['lane']}`" not in body:
            fail(f"{ticket_id}: write lane differs from plan.json")
        body_findings = re.findall(r"TM21-\d{3}", body.split("## 목적", 1)[0])
        if sorted(body_findings) != sorted(ticket_findings):
            fail(f"{ticket_id}: header finding mapping differs from plan.json")
        missing_headings = sorted(REQUIRED_HEADINGS - set(body.splitlines()))
        if missing_headings:
            fail(f"{ticket_id}: missing headings {missing_headings}")
        ticket_acceptance = re.findall(rf"`({re.escape(ticket_id)}-A\d{{2}})`", body)
        if not ticket_acceptance:
            fail(f"{ticket_id}: no acceptance IDs")
        if len(ticket_acceptance) != len(set(ticket_acceptance)):
            fail(f"{ticket_id}: duplicate acceptance IDs")
        acceptance_ids.extend(ticket_acceptance)
        if f"({ticket['file']})" not in readme:
            fail(f"{ticket_id}: README link missing")

    if sorted(mapped_findings) != sorted(finding_ids):
        missing = sorted(set(finding_ids) - set(mapped_findings))
        extra = sorted(set(mapped_findings) - set(finding_ids))
        duplicates = sorted(
            finding for finding in set(mapped_findings) if mapped_findings.count(finding) > 1
        )
        fail(f"finding mapping mismatch missing={missing} extra={extra} duplicates={duplicates}")
    if len(acceptance_ids) != len(set(acceptance_ids)):
        fail("acceptance IDs are not globally unique")
    lane_tickets: dict[str, list[str]] = {}
    for ticket in tickets:
        lane_tickets.setdefault(ticket["lane"], []).append(ticket["id"])
    expected_serialized_lanes = {
        lane for lane, ticket_ids in lane_tickets.items() if len(ticket_ids) > 1
    }
    lane_serialization = plan.get("lane_serialization")
    if not isinstance(lane_serialization, dict):
        fail("lane_serialization must be an object")
    if set(lane_serialization) != expected_serialized_lanes:
        fail("lane_serialization must cover every multi-ticket lane exactly once")
    for lane, sequence in lane_serialization.items():
        if not isinstance(sequence, list) or len(sequence) != len(set(sequence)):
            fail(f"{lane}: lane serialization must be a unique list")
        if set(sequence) != set(lane_tickets[lane]):
            fail(f"{lane}: lane serialization inventory mismatch")
        for previous, current in zip(sequence[:-1], sequence[1:], strict=True):
            graph[current].append(previous)

    cycle = find_cycle(graph)
    if cycle:
        fail(f"dependency cycle: {' -> '.join(cycle)}")

    planned_markdown = {HERE / ticket_file for ticket_file in files}
    actual_markdown = set(HERE.glob("SEP21-*.md"))
    if actual_markdown != planned_markdown:
        missing = sorted(str(path.name) for path in planned_markdown - actual_markdown)
        extra = sorted(str(path.name) for path in actual_markdown - planned_markdown)
        fail(f"ticket file inventory mismatch missing={missing} extra={extra}")

    prompt_root = HERE.parent / "prompts"
    prompt_readme = prompt_root / "README.md"
    if not prompt_readme.is_file():
        fail("prompt pack README is missing")
    link_count += validate_local_links(
        prompt_readme, prompt_readme.read_text(encoding="utf-8")
    )
    orchestrator_prompt = (HERE / plan.get("orchestrator_prompt", "")).resolve()
    if not orchestrator_prompt.is_file():
        fail("orchestrator prompt is missing")
    link_count += validate_local_links(
        orchestrator_prompt, orchestrator_prompt.read_text(encoding="utf-8")
    )

    packets = plan.get("execution_packets")
    if not isinstance(packets, list) or not packets:
        fail("execution_packets must be a non-empty list")
    packet_ids = [packet.get("id") for packet in packets]
    packet_files = [packet.get("file") for packet in packets]
    if None in packet_ids or len(packet_ids) != len(set(packet_ids)):
        fail("execution packet IDs must be present and unique")
    if None in packet_files or len(packet_files) != len(set(packet_files)):
        fail("execution packet files must be present and unique")
    packet_tickets: list[str] = []
    planned_prompts = {prompt_readme.resolve(), orchestrator_prompt}
    for packet in packets:
        packet_file = (HERE / packet["file"]).resolve()
        planned_prompts.add(packet_file)
        if not packet_file.is_file():
            fail(f"{packet['id']}: execution prompt is missing")
        packet_body = packet_file.read_text(encoding="utf-8")
        link_count += validate_local_links(packet_file, packet_body)
        assigned = packet.get("tickets")
        if not isinstance(assigned, list) or not assigned:
            fail(f"{packet['id']}: tickets must be a non-empty list")
        for ticket_id in assigned:
            if ticket_id not in id_set:
                fail(f"{packet['id']}: unknown ticket {ticket_id}")
            if ticket_id not in packet_body:
                fail(f"{packet['id']}: prompt does not name {ticket_id}")
        packet_tickets.extend(assigned)
    if sorted(packet_tickets) != sorted(ids):
        missing = sorted(id_set - set(packet_tickets))
        duplicates = sorted(
            ticket_id
            for ticket_id in set(packet_tickets)
            if packet_tickets.count(ticket_id) > 1
        )
        fail(f"execution packet coverage mismatch missing={missing} duplicates={duplicates}")
    actual_prompts = {path.resolve() for path in prompt_root.glob("*.md")}
    if actual_prompts != planned_prompts:
        missing = sorted(path.name for path in planned_prompts - actual_prompts)
        extra = sorted(path.name for path in actual_prompts - planned_prompts)
        fail(f"prompt inventory mismatch missing={missing} extra={extra}")

    source_status = "not_checked" if args.structure_only else "current"
    citation_status = "not_checked" if args.structure_only else str(citation_count)
    print(
        "SEP-21 plan PASS: "
        f"tickets={len(tickets)} packets={len(packets)} findings={len(finding_ids)} "
        f"acceptance={len(acceptance_ids)} citations={citation_status} links={link_count} "
        f"acyclic=true source={source_status}"
    )


if __name__ == "__main__":
    main()
