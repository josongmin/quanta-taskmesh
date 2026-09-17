#!/usr/bin/env python3
"""Fail-closed crate-boundary checker for the taskmesh workspace (ADR-0001).

Enforces:

- every workspace member has a declared boundary policy (an *undeclared* member
  is a failure, not something to skip);
- permitted normal dependency edges, workspace and external alike;
- edges that are only permitted behind a feature really are optional;
- the contract crate's shipped dependency surface;
- forbidden runtime imports in engine/contract/rayon source trees — every file
  under `src/`, including `*_tests.rs` and `src/tests/` modules, and every
  spelling (`use tokio…`, `extern crate tokio`, and a fully-qualified
  `tokio::…` path with no `use` at all);
- that a test module living under `src/` (`mod x_tests;`, `mod tests;`) is
  declared under `#[cfg(test)]`, so "test-only" is a property the compiler
  enforces rather than a naming convention the scanner trusts.

The manifest edge check is the authority (an engine → tokio dependency is
refused there whatever the source says); the source scan is defence in depth
for the case where a permitted crate is reached through a path it should not be.

Fail-closed means the checker reports a problem when it cannot *verify* the
policy, not only when it can prove a violation. The previous version intersected
the dependency list with its own hardcoded crate set before checking anything,
so an unregistered workspace crate, an external dependency, and a
feature-optional edge were all filtered out before any rule could see them —
four negative fixtures returned zero violations. It also treated an unreadable
source file and a missing source directory as "nothing to report".
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

# Normal (shipped) dependency edges each workspace member may declare. Names are
# matched exactly and include external crates: a boundary policy that only knows
# about workspace crates cannot notice `engine -> tokio`.
ALLOWED_EDGES: dict[str, set[str]] = {
    "taskmesh-contract": {"serde"},
    # loom / shuttle are `[target.'cfg(loom)'.dependencies]`: present in the
    # graph, compiled only under the model-check cfgs (see engine `src/sync.rs`).
    "taskmesh-engine": {"taskmesh-contract", "parking_lot", "loom", "shuttle"},
    "taskmesh": {"taskmesh-contract", "taskmesh-engine", "tokio", "tokio-util"},
    "taskmesh-rayon": {"taskmesh-contract", "rayon"},
    # The bench harness is `publish = false` and sits outside the shipped graph;
    # it is allowed to depend on the whole workspace plus measurement crates.
    "taskmesh-bench": {
        "taskmesh",
        "taskmesh-contract",
        "taskmesh-engine",
        "hdrhistogram",
        "rand",
        "rand_distr",
        "iai-callgrind",
    },
}

# Edges permitted ONLY behind a feature flag. Declaring one of these as a
# non-optional dependency is a violation: it would make the adapter
# unconditional.
OPTIONAL_ONLY_EDGES: dict[str, set[str]] = {
    "taskmesh": {"taskmesh-rayon"},
    "taskmesh-bench": {"iai-callgrind"},
    # The model checkers must never become unconditional: a consumer's lockfile
    # would then resolve their dependency trees on the consumer's toolchain.
    "taskmesh-engine": {"loom", "shuttle"},
}

CONTRACT_ALLOWED_DEPS = {"serde"}

ENGINE_CRATES = {"taskmesh-engine"}

# Crates a source tree must not reach, by any spelling. `_crate_usage` turns
# each name into the three shapes Rust allows: `use name…`, `extern crate
# name`, and a qualified `name::…` path.
FORBIDDEN_CRATES_IN_ENGINE_SOURCE = ("tokio", "rayon", "taskmesh")
FORBIDDEN_CRATES_IN_RAYON_SOURCE = ("tokio", "tokio_util", "taskmesh", "taskmesh_engine")
FORBIDDEN_CRATES_IN_CONTRACT_SOURCE = ("tokio", "parking_lot", "rayon")

# `mod <name>;` declarations that name a test module: they must sit under
# `#[cfg(test)]`, or "test-only" is a convention the shipped binary ignores.
TEST_MODULE_DECLARATION = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;"
)
CFG_TEST = re.compile(r"^\s*#\[cfg\(test\)\]\s*$")
CRATE_USAGE = {
    name: re.compile(
        rf"(?m)^(?!\s*//)(?:.*?\b(?:use\s+{name}\b|extern\s+crate\s+{name}\b|{name}::))"
    )
    for name in set(
        FORBIDDEN_CRATES_IN_ENGINE_SOURCE
        + FORBIDDEN_CRATES_IN_RAYON_SOURCE
        + FORBIDDEN_CRATES_IN_CONTRACT_SOURCE
    )
}

# Source trees that must exist. A missing one means the check did not run, which
# is different from the check passing.
REQUIRED_SOURCE_DIRS = (
    "taskmesh-contract/src",
    "taskmesh-engine/src",
    "taskmesh/src",
    "taskmesh-rayon/src",
)


def run_cargo_metadata(root: Path) -> dict:
    cmd = [
        "cargo",
        "metadata",
        "--format-version",
        "1",
        "--no-deps",
        "--manifest-path",
        str(root / "Cargo.toml"),
    ]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
    except FileNotFoundError as exc:
        raise RuntimeError("could not run `cargo metadata`: cargo not found on PATH") from exc
    if proc.returncode != 0:
        raise RuntimeError(
            "`cargo metadata` failed (the workspace may not be wired up yet):\n"
            + proc.stderr.strip()
        )
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"`cargo metadata` produced unparseable JSON: {exc}") from exc


def workspace_member_names(metadata: dict) -> set[str]:
    """Workspace members according to cargo, not according to this file.

    Deriving the member list from the policy table is what made the
    "unknown workspace crate" branch unreachable: a crate absent from the table
    was filtered out before the check that was supposed to notice it.

    Members are matched by package *id*, which is the same opaque token cargo
    puts in `workspace_members`. Parsing names out of that token is
    version-dependent — cargo omits the name when it matches the directory —
    so the id is compared directly instead.
    """
    member_ids = set(metadata.get("workspace_members", []))
    packages = metadata.get("packages", [])
    names = {pkg["name"] for pkg in packages if pkg.get("id") in member_ids}
    if not names:
        raise RuntimeError(
            "could not match any workspace member to a package id; "
            "refusing to check a dependency graph this tool cannot read"
        )
    return names


def normal_dependencies(pkg: dict) -> list[dict]:
    """Shipped dependencies of a package, across every target and rename.

    `kind` is `None` for normal deps; dev and build deps are separate surfaces
    and are not part of the shipped boundary.
    """
    deps = []
    for dep in pkg.get("dependencies", []):
        if dep.get("kind") in (None, "normal"):
            deps.append(dep)
    return deps


def dependency_identity(dep: dict) -> str:
    """The name the boundary policy is written against.

    A renamed dependency still crosses the same boundary, so the *real* crate
    name is what matters, not the local alias.
    """
    return dep["name"]


def check_edges(metadata: dict) -> list[str]:
    violations: list[str] = []
    members = workspace_member_names(metadata)
    packages = {pkg["name"]: pkg for pkg in metadata.get("packages", [])}

    for name in sorted(members):
        allowed = ALLOWED_EDGES.get(name)
        if allowed is None:
            violations.append(f"unknown workspace crate '{name}' has no declared boundary policy")
            continue
        pkg = packages.get(name)
        if pkg is None:
            violations.append(f"workspace member '{name}' has no package entry in cargo metadata")
            continue
        optional_only = OPTIONAL_ONLY_EDGES.get(name, set())
        for dep in sorted(normal_dependencies(pkg), key=dependency_identity):
            dep_name = dependency_identity(dep)
            if dep_name == name:
                continue
            if dep_name in optional_only:
                if not dep.get("optional", False):
                    violations.append(
                        f"edge '{name}' -> '{dep_name}' must stay feature-optional, "
                        "but is declared as a non-optional dependency"
                    )
                continue
            if dep_name not in allowed:
                violations.append(
                    f"forbidden dependency edge: '{name}' -> '{dep_name}' "
                    f"(allowed: {sorted(allowed) or 'none'}; "
                    f"optional-only: {sorted(optional_only) or 'none'})"
                )

    # A policy entry for a crate that no longer exists is drift too.
    for declared in sorted(set(ALLOWED_EDGES) - members):
        violations.append(f"boundary policy declares '{declared}', which is not a workspace member")
    return violations


def check_contract_deps(metadata: dict) -> list[str]:
    violations: list[str] = []
    seen = False
    for pkg in metadata.get("packages", []):
        if pkg["name"] != "taskmesh-contract":
            continue
        seen = True
        for dep in normal_dependencies(pkg):
            name = dependency_identity(dep)
            if name in ALLOWED_EDGES:
                violations.append(f"taskmesh-contract must not depend on workspace crate '{name}'")
            elif name not in CONTRACT_ALLOWED_DEPS:
                violations.append(
                    f"taskmesh-contract forbidden dependency '{name}' "
                    f"(allowed: {sorted(CONTRACT_ALLOWED_DEPS)})"
                )
    if not seen:
        violations.append("taskmesh-contract is missing from cargo metadata")
    return violations


def _is_test_module_name(name: str) -> bool:
    return name == "tests" or name.endswith("_tests")


def check_test_modules_are_cfg_test(text: str, rs_file: Path) -> list[str]:
    """Every `mod tests;` / `mod x_tests;` in `text` must be preceded by
    `#[cfg(test)]` (attributes may stack; the cfg must be among them)."""
    violations: list[str] = []
    lines = text.splitlines()
    for index, line in enumerate(lines):
        match = TEST_MODULE_DECLARATION.match(line)
        if not match or not _is_test_module_name(match.group(1)):
            continue
        cursor = index - 1
        gated = False
        while cursor >= 0 and lines[cursor].strip().startswith(("#[", "//", "///")):
            if CFG_TEST.match(lines[cursor]):
                gated = True
                break
            cursor -= 1
        if not gated:
            violations.append(
                f"test module `mod {match.group(1)};` in {rs_file} is not under #[cfg(test)]: "
                "a test-only module the compiler ships is production code"
            )
    return violations


def _scan_dir_for_crates(crate_dir: Path, forbidden: tuple[str, ...]) -> list[str]:
    """Report every use of a forbidden crate under `crate_dir` — in every file,
    including test modules, since they are compiled into the crate."""
    violations: list[str] = []
    if not crate_dir.is_dir():
        # The caller decides whether absence is expected; see REQUIRED_SOURCE_DIRS.
        return violations
    for rs_file in sorted(crate_dir.rglob("*.rs")):
        try:
            text = rs_file.read_text(encoding="utf-8", errors="replace")
        except OSError as exc:
            # A file the checker could not read is a file the checker did not
            # check. Reporting it is the only honest outcome.
            violations.append(f"could not read {rs_file}: {exc}")
            continue
        for name in forbidden:
            match = CRATE_USAGE[name].search(text)
            if match:
                violations.append(
                    f"forbidden usage of crate '{name}' in {rs_file}: {match.group(0).strip()!r}"
                )
        violations.extend(check_test_modules_are_cfg_test(text, rs_file))
    return violations


def check_source_usages(root: Path) -> list[str]:
    crates_dir = root / "crates"
    violations: list[str] = []

    for relative in REQUIRED_SOURCE_DIRS:
        if not (crates_dir / relative).is_dir():
            violations.append(f"required source directory is missing: crates/{relative}")

    for crate in sorted(ENGINE_CRATES):
        violations.extend(
            _scan_dir_for_crates(crates_dir / crate / "src", FORBIDDEN_CRATES_IN_ENGINE_SOURCE)
        )
    violations.extend(
        _scan_dir_for_crates(
            crates_dir / "taskmesh-contract" / "src", FORBIDDEN_CRATES_IN_CONTRACT_SOURCE
        )
    )
    violations.extend(
        _scan_dir_for_crates(
            crates_dir / "taskmesh-rayon" / "src", FORBIDDEN_CRATES_IN_RAYON_SOURCE
        )
    )
    # The host may use tokio; its in-`src` test modules must still be gated.
    host_src = crates_dir / "taskmesh" / "src"
    if host_src.is_dir():
        for rs_file in sorted(host_src.rglob("*.rs")):
            try:
                text = rs_file.read_text(encoding="utf-8", errors="replace")
            except OSError as exc:
                violations.append(f"could not read {rs_file}: {exc}")
                continue
            violations.extend(check_test_modules_are_cfg_test(text, rs_file))

    return violations


def run_checks(root: Path) -> list[str]:
    metadata = run_cargo_metadata(root)
    violations = check_edges(metadata)
    violations.extend(check_contract_deps(metadata))
    violations.extend(check_source_usages(root))
    return violations


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Fail-closed crate-boundary checker for the taskmesh workspace."
    )
    parser.add_argument(
        "--manifest-path",
        type=Path,
        default=None,
        help="Path to the workspace root (defaults to repo root or cwd).",
    )
    args = parser.parse_args(argv)

    root = args.manifest_path
    if root is None:
        root = REPO_ROOT if (REPO_ROOT / "Cargo.toml").is_file() else Path.cwd()

    try:
        violations = run_checks(root)
    except RuntimeError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    if violations:
        print("crate-boundary check FAILED:", file=sys.stderr)
        for violation in violations:
            print(f"  - {violation}", file=sys.stderr)
        return 1

    print("crate-boundary check OK: no violations")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
