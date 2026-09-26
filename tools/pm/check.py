#!/usr/bin/env python3
"""Validate taskmesh prompt surfaces without rendering or rewriting them."""

from __future__ import annotations

import json
import os
import re
import sys
import textwrap
from pathlib import Path, PurePosixPath
from typing import Any

import yaml

PM_DIR = Path(__file__).resolve().parent
REPO = PM_DIR.parent.parent
INVENTORY = PM_DIR / "inventory.json"
IMPORT_RE = re.compile(r"(?<![\w`])@([A-Za-z0-9._/-]+)")
STARTUP_SURFACES = (
    "AGENTS.md",
    "AGENTS.override.md",
    "CLAUDE.md",
    ".claude/CLAUDE.md",
    ".cursorrules",
)


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
            _source_file(INVENTORY, label="inventory.json").read_text(encoding="utf-8"),
            object_pairs_hook=_unique_json,
        )
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise PolicyError(f"cannot load {INVENTORY.relative_to(REPO)}: {exc}") from exc
    if not isinstance(raw, dict) or type(raw.get("schema_version")) is not int:
        raise PolicyError("inventory.json must be an object with integer schema_version")
    if raw["schema_version"] != 2:
        raise PolicyError("inventory.json schema_version must be 2")
    if set(raw) != {"schema_version", "startup", "cursor_rules", "generated"}:
        raise PolicyError("inventory.json has unknown or missing top-level keys")
    if not all(isinstance(raw[key], list) for key in ("startup", "cursor_rules", "generated")):
        raise PolicyError("inventory startup, cursor_rules and generated must be lists")
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
    imports: dict[str, list[str]] = {}
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
        if type(max_bytes) is not int or type(max_lines) is not int:
            raise PolicyError(f"{path}: startup budgets must be integers")
        if max_bytes <= 0 or max_lines <= 0:
            raise PolicyError(f"{path}: startup budgets must be positive")
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
        if len(expected) != len(expected_imports):
            raise PolicyError(f"{path}: duplicate inventory imports")
        actual = set(IMPORT_RE.findall(text))
        if actual != expected:
            raise PolicyError(
                f"{path}: imports differ from inventory: expected {sorted(expected)}, "
                f"got {sorted(actual)}"
            )
        imports[path] = [
            _source_file((REPO / path).parent / imported, label=f"{path}: import {imported}")
            .relative_to(REPO)
            .as_posix()
            for imported in sorted(actual)
        ]
        reports.append(f"{path}: {byte_count} bytes, {line_count} lines")
    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(path: str) -> None:
        if path not in seen:
            raise PolicyError(f"startup import is not inventoried: {path}")
        if path in visiting:
            raise PolicyError(f"startup import cycle: {path}")
        if path in visited:
            return
        visiting.add(path)
        for imported in imports[path]:
            visit(imported)
        visiting.remove(path)
        visited.add(path)

    for path in sorted(seen):
        visit(path)
    return reports


def _check_startup_surfaces(entries: list[dict[str, Any]]) -> None:
    registered = {entry["path"] for entry in entries}
    if "AGENTS.md" not in registered:
        raise PolicyError("startup inventory must include AGENTS.md")
    actual = {
        path for path in STARTUP_SURFACES if (REPO / path).exists() or (REPO / path).is_symlink()
    }
    missing = sorted(actual - registered)
    if missing:
        raise PolicyError(f"unregistered startup instruction surfaces: {missing}")


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


ROUTE_START = "<!-- pm:routes:start -->"
ROUTE_END = "<!-- pm:routes:end -->"


def _unique_json(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise PolicyError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _registry(name: str, key: str) -> list[dict[str, Any]]:
    try:
        data = json.loads(_read(f"tools/pm/{name}.json"), object_pairs_hook=_unique_json)
    except json.JSONDecodeError as exc:
        raise PolicyError(f"{name}.json: invalid JSON: {exc}") from exc
    if not isinstance(data, dict) or set(data) != {"schema_version", key}:
        raise PolicyError(f"{name}.json: expected schema_version and {key}")
    if type(data["schema_version"]) is not int or data["schema_version"] != 1:
        raise PolicyError(f"{name}.json: schema_version must be 1")
    entries = data[key]
    if not isinstance(entries, list) or not entries:
        raise PolicyError(f"{name}.json: {key} must be non-empty")
    seen: set[str] = set()
    for entry in entries:
        if not isinstance(entry, dict):
            raise PolicyError(f"{name}.json: entries must be objects")
        identifier = entry.get("id")
        if not isinstance(identifier, str) or not re.fullmatch(r"[a-z][a-z0-9-]*", identifier):
            raise PolicyError(f"{name}.json: invalid id: {identifier!r}")
        if identifier in seen:
            raise PolicyError(f"{name}.json: duplicate id: {identifier}")
        seen.add(identifier)
    return entries


def _headings(body: str) -> list[str]:
    headings: list[str] = []
    fence: str | None = None
    for line in body.splitlines():
        marker = re.match(r"^\s*(`{3,}|~{3,})", line)
        if marker:
            token = marker.group(1)
            if fence is None:
                fence = token
            elif token[0] == fence[0] and len(token) >= len(fence):
                fence = None
        elif fence is None:
            match = re.match(r"^#{1,6} (.+?)(?: +#+)?$", line)
            if match:
                headings.append(match.group(1))
    return headings


def _budget(path: str, body: str, entry: dict[str, Any]) -> None:
    limits = (entry["max_bytes"], entry["max_lines"])
    if any(type(value) is not int or value <= 0 for value in limits):
        raise PolicyError(f"{path}: generated budgets must be positive integers")
    if len(body.encode("utf-8")) > limits[0] or len(body.splitlines()) > limits[1]:
        raise PolicyError(f"{path}: generated output exceeds budget")


def generated_outputs(inventory: dict[str, Any]) -> dict[str, str]:
    """Validate registries and render only marked router blocks, without writing."""
    references = _registry("references", "references")
    refs = {entry["id"]: entry for entry in references}
    paths: set[str] = set()
    headings: dict[str, list[str]] = {}
    for ref in references:
        if set(ref) != {"id", "path", "role", "status", "superseded_by"}:
            raise PolicyError(f"{ref['id']}: invalid reference fields")
        path = _relative_path(ref["path"], field="reference.path")
        if path != ref["path"] or path in paths:
            raise PolicyError(f"{ref['id']}: duplicate or noncanonical reference path")
        if not re.fullmatch(r"[A-Za-z0-9_./-]+", path):
            raise PolicyError(f"{ref['id']}: reference path must be plain repository text")
        paths.add(path)
        if not isinstance(ref["role"], str) or not ref["role"].strip():
            raise PolicyError(f"{ref['id']}: reference role is required")
        if ref["status"] not in ("active", "superseded", "archive"):
            raise PolicyError(f"{ref['id']}: invalid reference status")
        if ref["status"] == "active" and "archive" in PurePosixPath(path).parts:
            raise PolicyError(f"{ref['id']}: active reference points to archive")
        successor = ref["superseded_by"]
        if ref["status"] == "superseded":
            if not isinstance(successor, str) or successor not in refs or successor == ref["id"]:
                raise PolicyError(f"{ref['id']}: invalid superseded_by reference")
        elif successor is not None:
            raise PolicyError(f"{ref['id']}: only superseded references may have successors")
        headings[ref["id"]] = _headings(_read(path))
    for ref in references:
        seen: set[str] = set()
        current = ref
        while current["superseded_by"] is not None:
            if current["id"] in seen:
                raise PolicyError(f"{ref['id']}: superseded_by cycle")
            seen.add(current["id"])
            current = refs[current["superseded_by"]]

    registered = {entry["path"] for entry in inventory["startup"] + inventory["cursor_rules"]}
    outputs: dict[str, str] = {}
    limits: dict[str, dict[str, Any]] = {}
    for entry in inventory["generated"]:
        if not isinstance(entry, dict) or set(entry) != {"path", "max_bytes", "max_lines"}:
            raise PolicyError("generated entries need path, max_bytes and max_lines")
        path = _relative_path(entry["path"], field="generated.path")
        if path not in registered or path in limits:
            raise PolicyError(f"{path}: generated surface is unregistered or duplicated")
        limits[path] = entry
        outputs[path] = ""
    if "AGENTS.md" not in outputs:
        raise PolicyError("generated surfaces must include AGENTS.md")
    seen_conditions: set[str] = set()
    for route in _registry("routes", "routes"):
        if set(route) != {"id", "when", "targets", "surfaces"}:
            raise PolicyError(f"{route['id']}: invalid route fields")
        condition = route["when"]
        if (
            not isinstance(condition, str)
            or not condition.strip()
            or any(char in condition for char in "\n\r@`")
        ):
            raise PolicyError(f"{route['id']}: route condition must be plain single-line text")
        if condition.strip().casefold() in seen_conditions:
            raise PolicyError(f"{route['id']}: duplicate route condition")
        seen_conditions.add(condition.strip().casefold())
        surfaces = _patterns(route["surfaces"], path=route["id"], field="surfaces")
        if "AGENTS.md" not in surfaces or set(surfaces) - set(outputs):
            raise PolicyError(f"{route['id']}: routes need AGENTS.md and registered surfaces")
        targets = route["targets"]
        if not isinstance(targets, list) or not targets:
            raise PolicyError(f"{route['id']}: targets must be non-empty")
        rendered: list[str] = []
        seen_targets: set[tuple[str, str | None]] = set()
        for target in targets:
            if not isinstance(target, dict) or set(target) != {"ref", "section"}:
                raise PolicyError(f"{route['id']}: targets need ref and section")
            identifier, section = target["ref"], target["section"]
            if not isinstance(identifier, str) or identifier not in refs:
                raise PolicyError(f"{route['id']}: unknown document reference: {identifier!r}")
            if section is not None and (
                not isinstance(section, str)
                or not section
                or any(char in section for char in "\n\r@`")
            ):
                raise PolicyError(f"{route['id']}: invalid section")
            if (identifier, section) in seen_targets:
                raise PolicyError(f"{route['id']}: duplicate route target")
            seen_targets.add((identifier, section))
            ref = refs[identifier]
            if ref["status"] != "active":
                raise PolicyError(f"{route['id']}: route targets inactive reference: {identifier}")
            if section is not None and headings[identifier].count(section) != 1:
                raise PolicyError(f"{route['id']}: missing or ambiguous section: {section}")
            rendered.append(f"`{ref['path']}`" + (f" (section: {section})" if section else ""))
        line = textwrap.fill(
            f"- {condition}: {'; '.join(rendered)}.",
            width=96,
            subsequent_indent="  ",
            break_long_words=False,
            break_on_hyphens=False,
        )
        for surface in surfaces:
            outputs[surface] += line + "\n"
    for path, router in outputs.items():
        if not router:
            raise PolicyError(f"{path}: generated surface has no routes")
        body = _read(path)
        if body.count(ROUTE_START) != 1 or body.count(ROUTE_END) != 1:
            raise PolicyError(f"{path}: expected one generated router marker pair")
        before, remaining = body.split(ROUTE_START)
        if ROUTE_END not in remaining:
            raise PolicyError(f"{path}: reversed generated router markers")
        _, after = remaining.split(ROUTE_END)
        outputs[path] = before + ROUTE_START + "\n" + router + ROUTE_END + after
        _budget(path, outputs[path], limits[path])
    for entry in inventory["startup"]:
        if entry["path"] in outputs:
            _budget(entry["path"], outputs[entry["path"]], entry)
    return outputs


def build() -> list[str]:
    """Validate all candidates before updating only marked router blocks."""
    inventory = _load_inventory()
    _check_startup(inventory["startup"])
    _check_startup_surfaces(inventory["startup"])
    _check_cursor_rules(inventory["cursor_rules"])
    candidates = generated_outputs(inventory)
    changed = [path for path, body in candidates.items() if _read(path) != body]
    for path in changed:
        (REPO / path).write_text(candidates[path], encoding="utf-8")
    check()
    return changed


def check() -> list[str]:
    inventory = _load_inventory()
    reports = _check_startup(inventory["startup"])
    _check_startup_surfaces(inventory["startup"])
    rule_count = _check_cursor_rules(inventory["cursor_rules"])
    for path, rendered in generated_outputs(inventory).items():
        if _read(path) != rendered:
            raise PolicyError(f"{path}: generated router drift; run just prompt-build")
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
