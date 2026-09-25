#!/usr/bin/env python3
"""Read-only structural validation of the plan, not product qualification."""

import hashlib
import json
import re
import sys
from pathlib import Path
from urllib.parse import unquote, urlsplit

BASE = Path(__file__).resolve().parent
ROOT = BASE.parents[4]


def exception_problems(base, ticket_files):
    """Implemented state: an unchecked box in a ticket is an *exception* (a
    recorded decision, an environment block, or an external dependency), never
    an unfinished item. Each one must link to the register and, when it carries
    an acceptance id, appear there by id; and the register must not list more
    rows than there are boxes (a row for something that is checked is stale)."""
    problems = []
    exceptions = (base / "EXCEPTIONS.md").read_text()
    exception_rows = [
        line for line in exceptions.splitlines() if line.startswith("| H16-") and " | " in line
    ]
    unchecked = 0
    for file in ticket_files:
        lines = (base / file).read_text().splitlines()
        for line_no, line in enumerate(lines):
            if not line.strip().startswith("- [ ]"):
                continue
            unchecked += 1
            block = [line]
            for following in lines[line_no + 1 :]:
                if following.startswith("  ") and not following.strip().startswith("- ["):
                    block.append(following)
                else:
                    break
            text = "\n".join(block)
            if "EXCEPTIONS.md" not in text:
                problems.append(
                    f"{file}:{line_no + 1}: unchecked item without a link to EXCEPTIONS.md"
                )
            for aid in sorted(set(re.findall(r"\bH16-\d{3}-A\d{2}\b", text))):
                if aid not in exceptions:
                    problems.append(f"{aid}: unchecked but not in EXCEPTIONS.md")
    if len(exception_rows) != unchecked:
        problems.append(
            f"EXCEPTIONS.md lists {len(exception_rows)} rows "
            f"but the tickets have {unchecked} unchecked items"
        )
    return problems


def main():
    errors = []

    def require(condition, message):
        if not condition:
            errors.append(message)

    def safe_path(raw, origin=ROOT):
        path = (origin / raw).resolve()
        require(path.is_relative_to(ROOT), f"path outside repo: {raw}")
        return path

    try:
        manifest = json.loads((BASE / "plan.json").read_text())
        tickets = manifest["tickets"]
        bugs = manifest["source_bugs"]
        quality = manifest["quality_items"]
        decisions = manifest["decisions"]
        require(manifest["schema_version"] == 1, "unknown schema version")
        require(manifest["status"] == "IMPLEMENTED", "unexpected plan status")
        require(re.fullmatch(r"[0-9a-f]{40}", manifest["source_head"]), "invalid source HEAD")
        ids = [t["id"] for t in tickets]
        expected_ids = {f"H16-{i:03}" for i in range(1, 23)}
        require(len(ids) == len(set(ids)) == 22, "ticket count/uniqueness != 22")
        require(set(ids) == expected_ids, "ticket IDs do not match H16-001..022")
        by_id = {t["id"]: t for t in tickets}
        require(len({t["file"] for t in tickets}) == len(tickets), "duplicate ticket file")
        observed_files = {p.name for p in BASE.glob("H16-*.md")}
        require(observed_files == {t["file"] for t in tickets}, "unregistered/missing ticket files")

        primary = []
        required_sections = [
            "목적",
            "변경 범위",
            "구현 액션",
            "검증 / 완료 조건",
            "실행 명령",
            "호환성 / 실패 모드",
            "인계 / 완료 증거",
        ]
        for ticket in tickets:
            tid = ticket["id"]
            expected_status = "RETIRED" if tid == "H16-020" else "IMPLEMENTED"
            require(ticket["status"] == expected_status, f"{tid}: unexpected status")
            if expected_status == "RETIRED":
                require(
                    ticket.get("retired_on") == "2026-09-23"
                    and bool(ticket.get("retirement_reason")),
                    f"{tid}: retirement decision is incomplete",
                )
            require(ticket["priority"] in {"P0", "P1", "P2"}, f"{tid}: invalid priority")
            require(ticket["owner"] in set("IERFVBCDQ"), f"{tid}: invalid owner")
            require(bool(ticket["locks"]), f"{tid}: missing write leases")
            require(
                len(ticket["depends_on"]) == len(set(ticket["depends_on"])),
                f"{tid}: duplicate dependency",
            )
            for dependency in ticket["depends_on"]:
                require(
                    dependency in by_id and dependency != tid,
                    f"{tid}: invalid dependency {dependency}",
                )
            path = safe_path(ticket["file"], BASE)
            require(path.is_file(), f"missing ticket file: {path}")
            if not path.is_file():
                continue
            body = path.read_text()
            require(body.startswith(f"# {tid} — "), f"{tid}: title mismatch")
            for section in required_sections:
                require(f"## {section}\n" in body, f"{tid}: missing section {section}")
            acceptance = re.findall(r"`(H16-\d{3}-A\d{2})`", body)
            require(acceptance == ticket["acceptance_ids"], f"{tid}: acceptance inventory drift")
            require(
                bool(acceptance) and len(acceptance) == len(set(acceptance)),
                f"{tid}: missing/duplicate acceptance",
            )
            for dependency in ticket["depends_on"]:
                require(
                    f"[{dependency}]({by_id[dependency]['file']})" in body
                    if dependency in by_id
                    else False,
                    f"{tid}: missing dependency link {dependency}",
                )
            for raw in ticket["existing_paths"]:
                require(safe_path(raw).exists(), f"{tid}: missing existing path {raw}")
                linked_paths = [
                    safe_path(unquote(urlsplit(target).path), path.parent)
                    for target in re.findall(r"\[[^\]\n]+\]\(([^)]+)\)", body)
                    if not urlsplit(target).scheme and urlsplit(target).path
                ]
                require(safe_path(raw) in linked_paths, f"{tid}: existing path not linked {raw}")
            for raw in ticket["proposed_paths"]:
                safe_path(raw)
                require(f"제안 경로: `{raw}`" in body, f"{tid}: proposed path not labeled {raw}")
            for bug_id in ticket["source_bugs"]:
                require(f"[{bug_id}]" in body, f"{tid}: missing finding link {bug_id}")
            primary.extend(ticket["source_bugs"])

        bug_ids = [b["id"] for b in bugs]
        expected_bugs = {f"TM16-{i:03}" for i in range(1, 41)}
        require(len(bug_ids) == len(set(bug_ids)) == 40, "source bug count/uniqueness != 40")
        require(set(bug_ids) == expected_bugs, "source IDs mismatch")
        require(
            set(primary) == expected_bugs and len(primary) == len(set(primary)),
            "findings missing or multiply assigned",
        )
        audit_dir = ROOT / "docs/bugbash/sep-16-general/tickets"
        require(
            {x.name for x in audit_dir.glob("TM16-*.md")} == {Path(b["path"]).name for b in bugs},
            "audit inventory changed",
        )
        for bug in bugs:
            path = safe_path(bug["path"])
            require(path.is_file(), f"missing source bug {path}")
            if path.is_file():
                actual = hashlib.sha256(path.read_bytes()).hexdigest()
                require(actual == bug["sha256"], f"source audit changed: {bug['id']}")

        visiting, visited = set(), set()

        def visit(tid):
            if tid in visiting:
                errors.append(f"dependency cycle at {tid}")
                return
            if tid in visited or tid not in by_id:
                return
            visiting.add(tid)
            for dep in by_id[tid]["depends_on"]:
                visit(dep)
            visiting.remove(tid)
            visited.add(tid)

        visit("H16-022")
        require(visited == expected_ids, "final qualification closure misses tickets")
        for tid in ids:
            visit(tid)
        for items, prefix, count in [(quality, "Q", 33), (decisions, "D", 12)]:
            item_ids = [item["id"] for item in items]
            require(
                len(item_ids) == count
                and set(item_ids) == {f"{prefix}{i:02}" for i in range(1, count + 1)},
                f"{prefix}: inventory mismatch",
            )
            for item in items:
                require(bool(item["tickets"]), f"{item['id']}: no responsible ticket")
                require(all(t in by_id for t in item["tickets"]), f"{item['id']}: unknown ticket")

        markdown = list(BASE.glob("*.md"))
        require(len(markdown) == 30, "expected 22 tickets + 8 coordination Markdown documents")
        for path in markdown:
            body = path.read_text()
            for line_no, line in enumerate(body.splitlines(), 1):
                require(line.rstrip() == line, f"{path.name}:{line_no}: trailing whitespace")
            require(body.count("```") % 2 == 0, f"{path.name}: unbalanced code fences")
            for target in re.findall(r"\[[^\]\n]+\]\(([^)]+)\)", body):
                url = urlsplit(target)
                if url.scheme or not url.path:
                    continue
                require(
                    safe_path(unquote(url.path), path.parent).exists(),
                    f"{path.name}: broken local link {target}",
                )
            for tid in set(re.findall(r"\bH16-\d{3}\b", body)):
                require(tid in by_id, f"{path.name}: unknown ticket mention {tid}")
        for problem in exception_problems(BASE, [t["file"] for t in tickets]):
            require(False, problem)
        coverage = (BASE / "COVERAGE.md").read_text()
        for bug in bugs:
            owner = next((t for t in tickets if bug["id"] in t["source_bugs"]), None)
            if owner:
                row = next(
                    (line for line in coverage.splitlines() if line.startswith(f"| [{bug['id']}]")),
                    "",
                )
                require(f"[{owner['id']}]({owner['file']})" in row, f"coverage drift: {bug['id']}")
        for item in quality:
            require(f"| {item['id']} |" in coverage, f"missing quality row {item['id']}")
    except (KeyError, TypeError, ValueError, OSError) as exc:
        errors.append(f"invalid plan or unreadable evidence: {exc}")

    if errors:
        for error in errors:
            print(f"FAIL: {error}", file=sys.stderr)
        return 1
    print(
        "PASS: 22 tracked tickets (21 implemented, 1 retired); "
        "unchecked items all registered in EXCEPTIONS.md; "
        "40 findings uniquely mapped and source hashes preserved; "
        "33 quality items; 12 decisions; acyclic complete closure; "
        "30 Markdown files/links/acceptance IDs valid"
    )
    print("Scope: document integrity only; production implementation/tests/qualification NOT RUN")
    return 0


if __name__ == "__main__":
    sys.exit(main())
