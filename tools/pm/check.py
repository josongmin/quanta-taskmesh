#!/usr/bin/env python3
"""Validate taskmesh prompt surfaces without rendering or rewriting them."""

from __future__ import annotations

import json
import os
import re
import sys
from pathlib import Path, PurePosixPath
from typing import Any

import yaml

PM_DIR = Path(__file__).resolve().parent
REPO = PM_DIR.parent.parent
INVENTORY = PM_DIR / "inventory.json"
IMPORT_RE = re.compile(r"(?<![\w`])@([A-Za-z0-9._/-]+)")


class PolicyError(Exception):
    """A prompt-policy surface is missing, malformed, or out of inventory."""


class _UniqueKeyLoader(yaml.SafeLoader):
    pass


def _construct_mapping(loader: _UniqueKeyLoader, node: yaml.MappingNode, deep: bool = False):
    seen: set[Any] = set()
    for key_node, _value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in seen:
            raise PolicyError(f"duplicate YAML key {key!r} at line {key_node.start_mark.line + 1}")
        seen.add(key)
    return yaml.SafeLoader.construct_mapping(loader, node, deep=deep)


_UniqueKeyLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG,
    _construct_mapping,
)


def _relative_path(raw: object, *, field: str) -> str:
    if not isinstance(raw, str) or not raw:
        raise PolicyError(f"{field} must be a non-empty string")
    path = PurePosixPath(raw)
    if path.is_absolute() or ".." in path.parts:
        raise PolicyError(f"{field} must stay inside the repository: {raw!r}")
    return path.as_posix()


def _load_inventory() -> dict[str, Any]:
    try:
        raw = json.loads(
            _source_file(INVENTORY, label="inventory.json").read_text(encoding="utf-8")
        )
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise PolicyError(f"cannot load {INVENTORY.relative_to(REPO)}: {exc}") from exc
    if not isinstance(raw, dict) or raw.get("schema_version") != 1:
        raise PolicyError("inventory.json must be an object with schema_version 1")
    if set(raw) != {"schema_version", "startup", "cursor_rules"}:
        raise PolicyError("inventory.json has unknown or missing top-level keys")
    if not isinstance(raw["startup"], list) or not isinstance(raw["cursor_rules"], list):
        raise PolicyError("inventory startup and cursor_rules must be lists")
    return raw


def _source_file(candidate: Path, *, label: str) -> Path:
    # Inspect the lexical traversal before collapsing `..`: a symlink/..
    # sequence may resolve somewhere different from its normalized spelling.
    try:
        relative = candidate.relative_to(REPO)
        normalized = Path(os.path.normpath(candidate))
        normalized.relative_to(REPO)
    except ValueError as exc:
        raise PolicyError(f"{label}: path escapes the repository") from exc
    component = REPO
    for part in relative.parts:
        component /= part
        if component.is_symlink():
            raise PolicyError(f"{label}: symlinked prompt-policy path is not source-bound")
    if not normalized.is_file():
        raise PolicyError(f"missing prompt-policy file: {label}")
    return normalized


def _read(path: str) -> str:
    candidate = _source_file(REPO / _relative_path(path, field="prompt-policy path"), label=path)
    try:
        return candidate.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as exc:
        raise PolicyError(f"cannot read prompt-policy file {path}: {exc}") from exc


def _frontmatter(path: str) -> dict[str, Any]:
    text = _read(path)
    if not text.startswith("---\n"):
        raise PolicyError(f"{path}: missing YAML frontmatter")
    try:
        raw_metadata, _body = text[4:].split("\n---\n", 1)
    except ValueError as exc:
        raise PolicyError(f"{path}: unterminated YAML frontmatter") from exc
    try:
        metadata = yaml.load(raw_metadata, Loader=_UniqueKeyLoader)
    except yaml.YAMLError as exc:
        raise PolicyError(f"{path}: invalid YAML frontmatter: {exc}") from exc
    if not isinstance(metadata, dict):
        raise PolicyError(f"{path}: frontmatter must be a mapping")
    return metadata


def _patterns(value: object, *, path: str, field: str) -> list[str]:
    values = [value] if isinstance(value, str) else value
    if not isinstance(values, list) or not values or not all(isinstance(v, str) for v in values):
        raise PolicyError(f"{path}: {field} must be a non-empty string or string list")
    if len(set(values)) != len(values):
        raise PolicyError(f"{path}: {field} contains duplicates")
    return values


def _check_startup(entries: list[object]) -> list[str]:
    reports: list[str] = []
    seen: set[str] = set()
    for entry in entries:
        if not isinstance(entry, dict) or set(entry) != {
            "path",
            "max_bytes",
            "max_lines",
            "imports",
        }:
            raise PolicyError("each startup entry needs path, max_bytes, max_lines, and imports")
        path = _relative_path(entry["path"], field="startup.path")
        if path in seen:
            raise PolicyError(f"duplicate startup inventory entry: {path}")
        seen.add(path)
        text = _read(path)
        byte_count = len(text.encode("utf-8"))
        line_count = len(text.splitlines())
        max_bytes = entry["max_bytes"]
        max_lines = entry["max_lines"]
        if not isinstance(max_bytes, int) or not isinstance(max_lines, int):
            raise PolicyError(f"{path}: startup budgets must be integers")
        if byte_count > max_bytes or line_count > max_lines:
            raise PolicyError(
                f"{path}: startup budget exceeded "
                f"({byte_count}/{max_bytes} bytes, {line_count}/{max_lines} lines)"
            )
        expected_imports = entry["imports"]
        if not isinstance(expected_imports, list) or not all(
            isinstance(value, str) and value for value in expected_imports
        ):
            raise PolicyError(f"{path}: imports must be a list")
        expected = set(expected_imports)
        actual = set(IMPORT_RE.findall(text))
        if actual != expected:
            raise PolicyError(
                f"{path}: imports differ from inventory: expected {sorted(expected)}, "
                f"got {sorted(actual)}"
            )
        for imported in actual:
            _source_file((REPO / path).parent / imported, label=f"{path}: import {imported}")
        reports.append(f"{path}: {byte_count} bytes, {line_count} lines")
    return reports


def _check_cursor_rules(entries: list[object]) -> int:
    expected_paths: set[str] = set()
    for entry in entries:
        if not isinstance(entry, dict) or set(entry) != {"path", "patterns"}:
            raise PolicyError("each Cursor rule needs path and patterns")
        path = _relative_path(entry["path"], field="cursor_rules.path")
        expected_patterns = _patterns(entry["patterns"], path="inventory.json", field="patterns")
        if path in expected_paths:
            raise PolicyError(f"duplicate Cursor rule inventory entry: {path}")
        expected_paths.add(path)

        metadata = _frontmatter(path)
        if set(metadata) != {"description", "globs", "alwaysApply"}:
            raise PolicyError(f"{path}: expected description, globs, and alwaysApply")
        if not isinstance(metadata["description"], str) or not metadata["description"].strip():
            raise PolicyError(f"{path}: description must be non-empty")
        if metadata["alwaysApply"] is not False:
            raise PolicyError(f"{path}: scoped rules must set alwaysApply: false")
        if _patterns(metadata["globs"], path=path, field="globs") != expected_patterns:
            raise PolicyError(f"{path}: globs differ from inventory")

    actual_paths = {
        path.relative_to(REPO).as_posix()
        for path in (REPO / ".cursor/rules").rglob("*")
        if path.is_file()
    }
    if actual_paths != expected_paths:
        raise PolicyError(
            f"Cursor rule inventory mismatch: expected {sorted(expected_paths)}, "
            f"got {sorted(actual_paths)}"
        )
    return len(entries)


def check() -> list[str]:
    inventory = _load_inventory()
    reports = _check_startup(inventory["startup"])
    rule_count = _check_cursor_rules(inventory["cursor_rules"])
    reports.append(f"scoped Cursor rules: {rule_count}")
    return reports


def main() -> int:
    try:
        reports = check()
    except PolicyError as exc:
        print(f"prompt-check: FAIL: {exc}", file=sys.stderr)
        return 1
    print("prompt-check: PASS")
    for report in reports:
        print(f"- {report}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
