"""Exact source and Cargo-target inventory for the bench integration suite."""

from __future__ import annotations

import re
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

PACKAGE = "taskmesh-bench"
TARGET = "bench_suite"
SUITE = "bench_suite.rs"
MEMBER = re.compile(r"[a-z][a-z0-9_]*\.rs\Z")
HEADER = "// This file is checked byte for byte against Cargo.toml's suite inventory.\n"


def _skip_trivia(source: str, index: int) -> int:
    """Skip only Rust whitespace and comments between attribute punctuation."""
    while index < len(source):
        if source[index].isspace():
            index += 1
        elif source.startswith("//", index):
            newline = source.find("\n", index + 2)
            index = len(source) if newline < 0 else newline + 1
        elif source.startswith("/*", index):
            depth = 1
            index += 2
            while index < len(source) and depth:
                if source.startswith("/*", index):
                    depth += 1
                    index += 2
                elif source.startswith("*/", index):
                    depth -= 1
                    index += 2
                else:
                    index += 1
            if depth:
                raise ValueError("bench suite member contains an unterminated block comment")
        else:
            break
    return index


def _has_inner_attribute_opener(source: str) -> bool:
    """Reject any raw `# ! [` token opening, including within strings/comments."""
    for index, character in enumerate(source):
        if character != "#":
            continue
        bang = _skip_trivia(source, index + 1)
        if bang < len(source) and source[bang] == "!":
            bracket = _skip_trivia(source, bang + 1)
            if bracket < len(source) and source[bracket] == "[":
                return True
    return False


def render(members: tuple[str, ...]) -> str:
    return HEADER + "".join(
        f'#[path = "{member}"]\nmod {member[:-3]};\n' for member in members
    )


def checked_members(root: Path) -> tuple[str, ...]:
    """Reject an inactive, incomplete, or altered suite before trusting case paths."""
    # `is_symlink` on descendants does not reveal a linked root ancestor.
    if root.absolute() != root.resolve(strict=True):
        raise ValueError(f"bench suite root has a symlink ancestor: {root}")
    package_root = root / "crates" / PACKAGE
    tests = package_root / "tests"
    manifest = package_root / "Cargo.toml"
    suite = tests / SUITE
    for path in (root / "crates", package_root, tests, manifest, suite):
        if path.is_symlink():
            raise ValueError(f"bench suite path is a symlink: {path}")
    if not tests.is_dir() or not manifest.is_file() or not suite.is_file():
        raise ValueError("bench suite manifest, directory, or target is missing")
    document = tomllib.loads(manifest.read_text(encoding="utf-8"))
    package = document.get("package")
    if not isinstance(package, dict) or package.get("name") != PACKAGE:
        raise ValueError("bench suite package identity differs")
    if package.get("autotests") is not False:
        raise ValueError("bench suite requires autotests=false")
    declared = document.get("test")
    if declared != [{"name": TARGET, "path": f"tests/{SUITE}"}]:
        raise ValueError("bench suite requires one explicit active test target")
    metadata = package.get("metadata")
    inventory = metadata.get("taskmesh_bench_suite") if isinstance(metadata, dict) else None
    if not isinstance(inventory, dict) or set(inventory) != {"target", "members"}:
        raise ValueError("bench suite inventory metadata is missing or malformed")
    if inventory["target"] != TARGET:
        raise ValueError("bench suite inventory target differs")
    entries = inventory["members"]
    if (
        not isinstance(entries, list)
        or not entries
        or not all(isinstance(item, str) and MEMBER.fullmatch(item) for item in entries)
        or SUITE in entries
        or entries != sorted(set(entries))
    ):
        raise ValueError("bench suite members must be unique sorted source basenames")
    members = tuple(entries)
    sources: set[str] = set()
    for path in tests.rglob("*"):
        if path.is_symlink():
            raise ValueError(f"bench suite path is a symlink: {path}")
    for path in tests.iterdir():
        if path.is_dir():
            if any(nested.suffix == ".rs" for nested in path.rglob("*.rs")):
                raise ValueError(f"bench suite has an unregistered nested source: {path}")
        elif path.suffix == ".rs":
            if not path.is_file():
                raise ValueError(f"bench suite source is not a regular file: {path}")
            sources.add(path.name)
    if sources != {*members, SUITE}:
        raise ValueError(
            "bench suite physical sources differ from inventory: "
            f"missing={sorted((set(members) | {SUITE}) - sources)} "
            f"extra={sorted(sources - set(members) - {SUITE})}"
        )
    for member in members:
        # No current member has an inner attribute. Reject any opener, including
        # whitespace/comment-separated forms, without parsing arbitrary Rust.
        if _has_inner_attribute_opener((tests / member).read_text(encoding="utf-8")):
            raise ValueError(f"bench suite member has an inner attribute: {member}")
    if suite.read_text(encoding="utf-8") != render(members):
        raise ValueError("bench suite module declarations differ from exact inventory")
    return members


def check_cargo_targets(root: Path, package: dict) -> tuple[str, ...]:
    expected_manifest = root / "crates" / PACKAGE / "Cargo.toml"
    manifest_path = package.get("manifest_path")
    if (
        not isinstance(manifest_path, str)
        or Path(manifest_path).absolute() != expected_manifest.absolute()
    ):
        raise ValueError("bench suite metadata manifest is outside owning package")
    members = checked_members(root)
    targets = package.get("targets")
    if not isinstance(targets, list) or any(not isinstance(target, dict) for target in targets):
        raise ValueError("bench suite Cargo metadata targets are malformed")
    tests = [target for target in targets if target.get("kind") == ["test"]]
    expected_path = root / "crates" / PACKAGE / "tests" / SUITE
    if (
        len(tests) != 1
        or tests[0].get("name") != TARGET
        or not isinstance(tests[0].get("src_path"), str)
        or Path(tests[0]["src_path"]).absolute() != expected_path.absolute()
        or tests[0].get("test") is not True
        or tests[0].get("required-features")
    ):
        raise ValueError("bench suite Cargo metadata does not select the sole explicit target")
    return members


def selected_case(root: Path, relative_source: str, case: str) -> str | None:
    """Return the canonical Nextest case name for a mapped bench source."""
    prefix = f"crates/{PACKAGE}/tests/"
    if not relative_source.startswith(prefix):
        return None
    member = relative_source[len(prefix):]
    if member not in checked_members(root):
        raise ValueError(f"bench scenario source is not an active suite member: {member}")
    if not isinstance(case, str) or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", case):
        raise ValueError("bench scenario case has an invalid identifier")
    return f"{member[:-3]}::{case}"
