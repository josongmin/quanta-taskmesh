"""Reference lifecycle, router drift and build preflight regressions."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path

import pytest

SPEC = importlib.util.spec_from_file_location(
    "pm_registry_check", Path(__file__).parents[1] / "check.py"
)
assert SPEC is not None and SPEC.loader is not None
PM = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PM)


@pytest.fixture
def registry(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    (tmp_path / "tools/pm").mkdir(parents=True)
    (tmp_path / "docs").mkdir()
    (tmp_path / "docs/contract.md").write_text(
        "# Contract\n\n## Decision\nRule.\n", encoding="utf-8"
    )
    (tmp_path / "AGENTS.md").write_text(
        "Manual rules stay.\n\n" + PM.ROUTE_START + "\n" + PM.ROUTE_END + "\n\nManual tail.\n",
        encoding="utf-8",
    )
    limits = {"path": "AGENTS.md", "max_bytes": 2048, "max_lines": 40}
    inventory = {
        "schema_version": 2,
        "startup": [{**limits, "imports": []}],
        "cursor_rules": [],
        "generated": [limits.copy()],
    }
    references = {
        "schema_version": 1,
        "references": [
            {
                "id": "contract",
                "path": "docs/contract.md",
                "role": "Execution contract",
                "status": "active",
                "superseded_by": None,
            }
        ],
    }
    routes = {
        "schema_version": 1,
        "routes": [
            {
                "id": "execution",
                "when": "Execution changes",
                "targets": [{"ref": "contract", "section": "Decision"}],
                "surfaces": ["AGENTS.md"],
            }
        ],
    }
    files = {"inventory": inventory, "references": references, "routes": routes}

    def save():
        for name, value in files.items():
            (tmp_path / f"tools/pm/{name}.json").write_text(json.dumps(value), encoding="utf-8")

    monkeypatch.setattr(PM, "REPO", tmp_path)
    monkeypatch.setattr(PM, "INVENTORY", tmp_path / "tools/pm/inventory.json")
    save()
    return tmp_path, files, save


def test_build_is_idempotent_and_check_is_read_only(registry):
    root, _, _ = registry
    original = (root / "AGENTS.md").read_text()
    with pytest.raises(PM.PolicyError, match="router drift"):
        PM.check()
    assert (root / "AGENTS.md").read_text() == original
    assert PM.build() == ["AGENTS.md"]
    rendered = (root / "AGENTS.md").read_text()
    assert rendered.startswith("Manual rules stay.\n")
    assert rendered.endswith("\n\nManual tail.\n")
    assert "`docs/contract.md` (section: Decision)" in rendered
    assert PM.build() == []
    PM.check()
    (root / "AGENTS.md").write_text(rendered.replace("Execution changes", "Invented changes"))
    with pytest.raises(PM.PolicyError, match="router drift"):
        PM.check()


@pytest.mark.parametrize(
    "case, message",
    [
        ("missing_file", "missing prompt-policy file"),
        ("symlink", "symlinked"),
        ("missing_section", "missing or ambiguous section"),
        ("duplicate_heading", "missing or ambiguous section"),
        ("fenced_heading", "missing or ambiguous section"),
        ("archive", "active reference points to archive"),
        ("inactive", "inactive reference"),
        ("unknown_ref", "unknown document reference"),
        ("duplicate_ref", "duplicate id"),
        ("duplicate_path", "duplicate or noncanonical"),
        ("duplicate_route", "duplicate id"),
        ("duplicate_condition", "duplicate route condition"),
        ("duplicate_target", "duplicate route target"),
        ("unknown_surface", "registered surfaces"),
        ("missing_markers", "marker pair"),
        ("reversed_markers", "reversed"),
        ("budget", "exceeds budget"),
        ("successor_cycle", "superseded_by cycle"),
    ],
)
def test_invalid_management_input_rejects_before_writing(registry, case, message):
    root, files, save = registry
    ref = files["references"]["references"][0]
    route = files["routes"]["routes"][0]
    doc = root / "docs/contract.md"
    if case == "missing_file":
        doc.unlink()
    elif case == "symlink":
        doc.rename(root / "docs/real.md")
        doc.symlink_to(root / "docs/real.md")
    elif case == "missing_section":
        route["targets"][0]["section"] = "Removed"
    elif case == "duplicate_heading":
        doc.write_text("## Decision\n## Decision\n")
    elif case == "fenced_heading":
        doc.write_text("```md\n## Decision\n```\n")
    elif case == "archive":
        ref["path"] = "docs/archive/contract.md"
    elif case == "inactive":
        ref["status"] = "archive"
    elif case == "unknown_ref":
        route["targets"][0]["ref"] = "unknown"
    elif case == "duplicate_ref":
        files["references"]["references"].append(ref.copy())
    elif case == "duplicate_path":
        files["references"]["references"].append({**ref, "id": "second"})
    elif case == "duplicate_route":
        files["routes"]["routes"].append(route.copy())
    elif case == "duplicate_condition":
        files["routes"]["routes"].append({**route, "id": "second"})
    elif case == "duplicate_target":
        route["targets"].append(route["targets"][0].copy())
    elif case == "unknown_surface":
        route["surfaces"].append("CLAUDE.md")
    elif case == "missing_markers":
        (root / "AGENTS.md").write_text("manual only\n")
    elif case == "reversed_markers":
        (root / "AGENTS.md").write_text(PM.ROUTE_END + "\n" + PM.ROUTE_START + "\n")
    elif case == "budget":
        files["inventory"]["generated"][0]["max_bytes"] = 1
    elif case == "successor_cycle":
        ref.update(status="superseded", superseded_by="second")
        (root / "docs/second.md").write_text("# Second\n")
        files["references"]["references"].append(
            {
                **ref,
                "id": "second",
                "path": "docs/second.md",
                "superseded_by": "contract",
            }
        )
    save()
    original = (root / "AGENTS.md").read_bytes()
    with pytest.raises(PM.PolicyError, match=message):
        PM.build()
    assert (root / "AGENTS.md").read_bytes() == original


def test_live_document_edits_need_no_digest_refresh(registry):
    root, _, _ = registry
    PM.build()
    doc = root / "docs/contract.md"
    doc.write_text(doc.read_text() + "\nNew contract detail.\n")
    PM.check()


def test_paths_are_not_split_when_wrapping_routes(registry):
    root, files, save = registry
    name = "long-document-" * 5 + "contract.md"
    (root / "docs/contract.md").rename(root / "docs" / name)
    files["references"]["references"][0]["path"] = "docs/" + name
    save()
    PM.build()
    assert f"`docs/{name}`" in (root / "AGENTS.md").read_text()


def test_duplicate_json_key_rejects(registry):
    root, _, _ = registry
    (root / "tools/pm/references.json").write_text(
        '{"schema_version":1,"schema_version":1,"references":[]}'
    )
    with pytest.raises(PM.PolicyError, match="duplicate JSON key"):
        PM.build()


def test_later_invalid_output_does_not_write_earlier_candidate(registry):
    root, files, save = registry
    limits = {"path": "CLAUDE.md", "max_bytes": 2048, "max_lines": 40}
    files["inventory"]["startup"].append({**limits, "imports": []})
    files["inventory"]["generated"].append(limits)
    files["routes"]["routes"][0]["surfaces"].append("CLAUDE.md")
    (root / "CLAUDE.md").write_text("missing router markers\n")
    save()
    before = (root / "AGENTS.md").read_bytes()
    with pytest.raises(PM.PolicyError, match="marker pair"):
        PM.build()
    assert (root / "AGENTS.md").read_bytes() == before


def test_section_cannot_inject_eager_import(registry):
    root, files, save = registry
    (root / "docs/contract.md").write_text("## @docs/all.md\n")
    files["routes"]["routes"][0]["targets"][0]["section"] = "@docs/all.md"
    save()
    before = (root / "AGENTS.md").read_bytes()
    with pytest.raises(PM.PolicyError, match="invalid section"):
        PM.build()
    assert (root / "AGENTS.md").read_bytes() == before
