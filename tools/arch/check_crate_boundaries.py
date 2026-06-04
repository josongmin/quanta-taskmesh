#!/usr/bin/env python3
"""Fail-closed crate-boundary checker for the taskmesh workspace (ADR-0001).

Enforces:
- permitted workspace dependency edges (``ALLOWED_EDGES``),
- optional feature edge for host → rayon adapter,
- contract deps limited to serde,
- forbidden runtime imports in engine/contract/rayon source trees.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

# Target graph; taskmesh-core is transitional alias for taskmesh-engine.
ALLOWED_EDGES: dict[str, set[str]] = {
    "taskmesh-contract": set(),
    "taskmesh-engine": {"taskmesh-contract"},
    "taskmesh-core": {"taskmesh-contract"},
    "taskmesh": {"taskmesh-contract", "taskmesh-engine", "taskmesh-core", "tokio"},
    "taskmesh-rayon": {"taskmesh-contract", "rayon"},
}

OPTIONAL_EDGES: dict[str, set[str]] = {
    "taskmesh": {"taskmesh-rayon", "tokio-util"},
}

CONTRACT_ALLOWED_DEPS = {"serde"}

ENGINE_CRATES = {"taskmesh-engine", "taskmesh-core"}

FORBIDDEN_IN_ENGINE_SOURCE = (
    "use tokio",
    "extern crate tokio",
    "use rayon",
    "extern crate rayon",
    "use taskmesh;",
    "use taskmesh::",
)

FORBIDDEN_IN_RAYON_SOURCE = (
    "use tokio",
    "extern crate tokio",
    "use taskmesh;",
    "use taskmesh_engine",
    "use taskmesh_core",
)

FORBIDDEN_IN_CONTRACT_SOURCE = (
    "use tokio",
    "use parking_lot",
    "use rayon",
)

WORKSPACE_CRATES = set(ALLOWED_EDGES)


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
        raise RuntimeError(
            "could not run `cargo metadata`: cargo not found on PATH"
        ) from exc
    if proc.returncode != 0:
        raise RuntimeError(
            "`cargo metadata` failed (the workspace may not be wired up yet):\n"
            + proc.stderr.strip()
        )
    return json.loads(proc.stdout)


def load_metadata(
    metadata: dict,
) -> dict[str, set[str]]:
    packages = metadata.get("packages", [])
    names = {pkg["name"] for pkg in packages}
    workspace_names = names & WORKSPACE_CRATES

    normal_graph: dict[str, set[str]] = {}
    for pkg in packages:
        name = pkg["name"]
        if name not in workspace_names:
            continue
        normal: set[str] = set()
        for dep in pkg.get("dependencies", []):
            dep_name = dep["name"]
            if dep_name not in workspace_names:
                continue
            kind = dep.get("kind")
            if kind is None or kind == "normal" or kind == "build":
                normal.add(dep_name)
        normal_graph[name] = normal
    return normal_graph


def check_edges(metadata: dict) -> list[str]:
    normal_graph = load_metadata(metadata)
    violations: list[str] = []
    for crate, deps in sorted(normal_graph.items()):
        allowed = ALLOWED_EDGES.get(crate)
        if allowed is None:
            violations.append(
                f"unknown workspace crate '{crate}' has no declared boundary policy"
            )
            continue
        optional = OPTIONAL_EDGES.get(crate, set())
        for dep in sorted(deps):
            if dep == crate:
                continue
            if dep not in allowed and dep not in optional:
                violations.append(
                    f"forbidden dependency edge: '{crate}' -> '{dep}' "
                    f"(allowed: {sorted(allowed) or 'none'}; "
                    f"optional: {sorted(optional) or 'none'})"
                )
    return violations


def check_contract_deps(metadata: dict) -> list[str]:
    violations: list[str] = []
    for pkg in metadata.get("packages", []):
        if pkg["name"] != "taskmesh-contract":
            continue
        for dep in pkg.get("dependencies", []):
            # Only the shipped (runtime) dependency surface is constrained.
            # dev-dependencies (e.g. serde_json for tests) and build-deps are not
            # part of the public contract and must not be flagged.
            kind = dep.get("kind")
            if kind not in (None, "normal"):
                continue
            name = dep["name"]
            if name in WORKSPACE_CRATES:
                violations.append(
                    f"taskmesh-contract must not depend on workspace crate '{name}'"
                )
            elif name not in CONTRACT_ALLOWED_DEPS:
                violations.append(
                    f"taskmesh-contract forbidden dependency '{name}' "
                    f"(allowed: {sorted(CONTRACT_ALLOWED_DEPS)})"
                )
    return violations


def _scan_dir_for_patterns(crate_dir: Path, patterns: list[str]) -> list[str]:
    violations: list[str] = []
    if not crate_dir.is_dir():
        return violations
    for rs_file in sorted(crate_dir.rglob("*.rs")):
        if "/tests/" in str(rs_file) or rs_file.name.endswith("_tests.rs"):
            continue
        try:
            text = rs_file.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        for pattern in patterns:
            if pattern in text:
                violations.append(f"forbidden usage '{pattern}' in {rs_file}")
    return violations


def check_source_usages(root: Path) -> list[str]:
    crates_dir = root / "crates"
    violations: list[str] = []

    for crate in ENGINE_CRATES:
        violations.extend(
            _scan_dir_for_patterns(
                crates_dir / crate / "src", FORBIDDEN_IN_ENGINE_SOURCE
            )
        )

    violations.extend(
        _scan_dir_for_patterns(
            crates_dir / "taskmesh-contract" / "src", FORBIDDEN_IN_CONTRACT_SOURCE
        )
    )

    rayon_dir = crates_dir / "taskmesh-rayon"
    if rayon_dir.is_dir():
        violations.extend(
            _scan_dir_for_patterns(rayon_dir / "src", FORBIDDEN_IN_RAYON_SOURCE)
        )

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
