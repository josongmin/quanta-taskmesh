from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]
CHECK_PATH = REPO / "tools" / "pm" / "check.py"
SPEC = importlib.util.spec_from_file_location("taskmesh_prompt_check", CHECK_PATH)
assert SPEC is not None and SPEC.loader is not None
CHECK = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = CHECK
SPEC.loader.exec_module(CHECK)


def test_current_prompt_surfaces_pass() -> None:
    reports = CHECK.check()
    assert reports[-1] == "scoped Cursor rules: 2"


@pytest.mark.parametrize("surface", ["CLAUDE.md", "AGENTS.override.md", ".cursorrules"])
def test_unregistered_root_startup_is_rejected(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, surface: str
) -> None:
    (tmp_path / "AGENTS.md").write_text("project rules\n", encoding="utf-8")
    (tmp_path / surface).write_text("competing rules\n", encoding="utf-8")
    inventory = tmp_path / "inventory.json"
    inventory.write_text(
        json.dumps(
            {
                "schema_version": 2,
                "startup": [{"path": "AGENTS.md", "max_bytes": 64, "max_lines": 4, "imports": []}],
                "cursor_rules": [],
                "generated": [],
            }
        ),
        encoding="utf-8",
    )
    monkeypatch.setattr(CHECK, "REPO", tmp_path)
    monkeypatch.setattr(CHECK, "INVENTORY", inventory)
    with pytest.raises(CHECK.PolicyError, match="unregistered startup"):
        CHECK.check()


def test_empty_startup_inventory_is_rejected(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    inventory = tmp_path / "inventory.json"
    inventory.write_text(
        json.dumps(
            {
                "schema_version": 2,
                "startup": [],
                "cursor_rules": [],
                "generated": [],
            }
        ),
        encoding="utf-8",
    )
    monkeypatch.setattr(CHECK, "REPO", tmp_path)
    monkeypatch.setattr(CHECK, "INVENTORY", inventory)
    with pytest.raises(CHECK.PolicyError, match="must include AGENTS.md"):
        CHECK.check()


@pytest.mark.parametrize("budget", [True, 0, -1])
def test_invalid_startup_budget_is_rejected(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, budget: object
) -> None:
    (tmp_path / "AGENTS.md").write_text("", encoding="utf-8")
    monkeypatch.setattr(CHECK, "REPO", tmp_path)
    with pytest.raises(CHECK.PolicyError, match="startup budgets must be"):
        CHECK._check_startup(
            [
                {"path": "AGENTS.md", "max_bytes": budget, "max_lines": 4, "imports": []},
            ]
        )


@pytest.mark.parametrize("cycle", [False, True])
def test_eager_import_must_be_inventoried_and_acyclic(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, cycle: bool
) -> None:
    (tmp_path / "AGENTS.md").write_text("@extra.md\n", encoding="utf-8")
    (tmp_path / "extra.md").write_text("@AGENTS.md\n" if cycle else "rules\n", encoding="utf-8")
    entries = [{"path": "AGENTS.md", "max_bytes": 64, "max_lines": 4, "imports": ["extra.md"]}]
    if cycle:
        entries.append(
            {
                "path": "extra.md",
                "max_bytes": 64,
                "max_lines": 4,
                "imports": ["AGENTS.md"],
            }
        )
    monkeypatch.setattr(CHECK, "REPO", tmp_path)
    with pytest.raises(CHECK.PolicyError, match="cycle" if cycle else "not inventoried"):
        CHECK._check_startup(entries)


def test_inventory_must_be_repository_owned_file(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repo = tmp_path / "repo"
    repo.mkdir()
    external = tmp_path / "inventory.json"
    external.write_text(
        json.dumps({"schema_version": 2, "startup": [], "cursor_rules": [], "generated": []}),
        encoding="utf-8",
    )
    inventory = repo / "inventory.json"
    inventory.symlink_to(external)
    monkeypatch.setattr(CHECK, "REPO", repo)
    monkeypatch.setattr(CHECK, "INVENTORY", inventory)

    with pytest.raises(CHECK.PolicyError, match="symlink"):
        CHECK._load_inventory()


def test_duplicate_frontmatter_key_is_rejected(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    rule = tmp_path / "rule.mdc"
    rule.write_text("---\nglobs: a\nglobs: b\n---\nbody\n", encoding="utf-8")
    monkeypatch.setattr(CHECK, "REPO", tmp_path)

    with pytest.raises(CHECK.PolicyError, match="duplicate YAML key 'globs'"):
        CHECK._frontmatter("rule.mdc")


def test_startup_imports_must_match_inventory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    (tmp_path / "AGENTS.md").write_text("rules\n", encoding="utf-8")
    (tmp_path / ".claude").mkdir()
    (tmp_path / ".claude/CLAUDE.md").write_text(
        "@../AGENTS.md\n@../docs/all.md\n", encoding="utf-8"
    )
    monkeypatch.setattr(CHECK, "REPO", tmp_path)
    entries = [
        {
            "path": ".claude/CLAUDE.md",
            "max_bytes": 256,
            "max_lines": 5,
            "imports": ["../AGENTS.md"],
        }
    ]

    with pytest.raises(CHECK.PolicyError, match="imports differ from inventory"):
        CHECK._check_startup(entries)


@pytest.mark.parametrize("parent_link", [False, True])
def test_startup_policy_cannot_read_through_symlink(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, parent_link: bool
) -> None:
    repo = tmp_path / "repo"
    repo.mkdir()
    external = tmp_path / "external"
    external.mkdir()
    (external / "policy.md").write_text("rules\n", encoding="utf-8")
    if parent_link:
        (repo / "policy").symlink_to(external, target_is_directory=True)
        path = "policy/policy.md"
    else:
        (repo / "policy.md").symlink_to(external / "policy.md")
        path = "policy.md"
    monkeypatch.setattr(CHECK, "REPO", repo)

    with pytest.raises(CHECK.PolicyError, match="symlink"):
        CHECK._check_startup([{"path": path, "max_bytes": 64, "max_lines": 4, "imports": []}])


def test_startup_import_cannot_read_through_symlink(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    (tmp_path / "AGENTS.md").write_text("rules\n", encoding="utf-8")
    (tmp_path / "alias.md").symlink_to(tmp_path / "AGENTS.md")
    (tmp_path / ".claude").mkdir()
    (tmp_path / ".claude/CLAUDE.md").write_text("@../alias.md\n", encoding="utf-8")
    monkeypatch.setattr(CHECK, "REPO", tmp_path)

    with pytest.raises(CHECK.PolicyError, match="symlink"):
        CHECK._check_startup(
            [
                {
                    "path": ".claude/CLAUDE.md",
                    "max_bytes": 64,
                    "max_lines": 4,
                    "imports": ["../alias.md"],
                }
            ]
        )


def test_import_symlink_before_parent_component_is_rejected(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repo = tmp_path / "repo"
    repo.mkdir()
    (repo / ".claude").mkdir()
    (repo / ".claude/AGENTS.md").write_text("local rules\n", encoding="utf-8")
    external = tmp_path / "external"
    external.mkdir()
    (repo / ".claude/link").symlink_to(external, target_is_directory=True)
    (repo / ".claude/CLAUDE.md").write_text("@link/../AGENTS.md\n", encoding="utf-8")
    monkeypatch.setattr(CHECK, "REPO", repo)

    with pytest.raises(CHECK.PolicyError, match="symlink"):
        CHECK._check_startup(
            [
                {
                    "path": ".claude/CLAUDE.md",
                    "max_bytes": 64,
                    "max_lines": 4,
                    "imports": ["link/../AGENTS.md"],
                }
            ]
        )


def test_unregistered_cursor_rule_is_rejected(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    rules = tmp_path / ".cursor/rules"
    rules.mkdir(parents=True)
    (rules / "a.mdc").write_text(
        "---\ndescription: a\nglobs: [src/**]\nalwaysApply: false\n---\n\nbody a\n",
        encoding="utf-8",
    )
    (rules / "extra.md").write_text("ignored by Cursor but unowned\n", encoding="utf-8")
    monkeypatch.setattr(CHECK, "REPO", tmp_path)

    with pytest.raises(CHECK.PolicyError, match="Cursor rule inventory mismatch"):
        CHECK._check_cursor_rules(
            [
                {
                    "path": ".cursor/rules/a.mdc",
                    "patterns": ["src/**"],
                }
            ]
        )
