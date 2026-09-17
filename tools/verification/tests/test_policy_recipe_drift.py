"""TM16-033 regression: policy text may only reference recipes that exist.

The test-quality rule pack used to point readers at `just mutants-critical` and
`just cov-gate` as the layers that catch what semgrep cannot. Neither recipe
existed. A policy that names a phantom gate is worse than one that admits a
gap, because it tells the reader the gap is covered.
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
POLICY_FILES = [
    REPO / "tools" / "semgrep" / "rules",
    REPO / "tools" / "semgrep" / "README.md",
    REPO / "tools" / "pm" / "README.md",
    REPO / "README.md",
    REPO / "docs" / "release-checklist.md",
    REPO / "AGENTS.md",
]
RECIPE_REFERENCE = re.compile(r"`just ([a-z][a-z0-9-]*)`|just ([a-z][a-z0-9-]*)\b")


def just_recipes() -> set[str]:
    proc = subprocess.run(
        ["just", "--summary"],
        capture_output=True,
        text=True,
        check=True,
        cwd=REPO,
    )
    return set(proc.stdout.split())


def referenced_recipes() -> dict[str, set[Path]]:
    found: dict[str, set[Path]] = {}
    for root in POLICY_FILES:
        paths = [root] if root.is_file() else list(root.rglob("*")) if root.is_dir() else []
        for path in paths:
            if not path.is_file() or path.suffix not in {".md", ".yml", ".yaml", ".txt", ""}:
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            for match in RECIPE_REFERENCE.finditer(text):
                name = match.group(1) or match.group(2)
                if name in {"just", "list"}:
                    continue
                found.setdefault(name, set()).add(path.relative_to(REPO))
    return found


def test_every_referenced_recipe_exists() -> None:
    recipes = just_recipes()
    assert recipes, "just reported no recipes"
    phantom = {
        name: sorted(str(p) for p in paths)
        for name, paths in referenced_recipes().items()
        if name not in recipes
    }
    assert not phantom, f"policy references recipes that do not exist: {phantom}"


def test_the_mutation_gate_recipe_exists_and_is_wired() -> None:
    recipes = just_recipes()
    assert "mutants-critical" in recipes
    inventory = REPO / "tools" / "verification" / "mutations.json"
    assert inventory.is_file(), "the recipe points at an inventory that must exist"


def test_no_coverage_gate_is_claimed() -> None:
    """No `cov-gate` recipe was adopted; no policy text may say otherwise."""
    for root in POLICY_FILES:
        paths = [root] if root.is_file() else list(root.rglob("*")) if root.is_dir() else []
        for path in paths:
            if not path.is_file():
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            assert "cov-gate" not in text, f"{path.relative_to(REPO)} still references cov-gate"
