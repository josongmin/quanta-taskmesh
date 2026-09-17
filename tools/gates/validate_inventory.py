#!/usr/bin/env python3
"""Validate the gate inventory against the Justfile, the CI workflows, and the
independently-authored required set (H16-018).

Fails closed on:

- a required id that is not in the inventory (a gate was deleted but its
  requirement was not),
- an inventory id that is not a Justfile recipe (the definition is gone),
- a `just <recipe>` invocation in a workflow that is not in the inventory
  (CI runs something local users cannot see),
- an inventory gate marked `ci` that no workflow invokes (local users run
  something CI does not enforce),
- duplicate ids, unknown tiers, unknown platforms,
- `just gate`'s dependency list drifting from the inventory's `fast` tier,
- `just matrix`'s dependency list drifting from the inventory's `matrix` tier,
- `just proof` not expanding to exactly the required set (a required gate
  CI runs that the local proof surface skips is a local/CI proof gap),
- a workflow step that *invokes* an inventory gate but cannot *fail* on it
  (`continue-on-error`, an `if:` condition, `|| true`, `set +e`, `exit 0`):
  parity is about enforcement, and a masked gate is a green check over nothing.

Workflow invocations are read from the parsed YAML: every `run:` script of
every step, whatever its shape (`- run: just x`, a `|` block, `cd d && just x`,
`just x --flag`). A regex over single lines saw only the simplest form.

It does not run anything; `tools/gates/run.py` does.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parents[2]
INVENTORY = Path(__file__).resolve().parent / "inventory.json"
REQUIRED = Path(__file__).resolve().parent / "required.json"
WORKFLOWS = REPO / ".github" / "workflows"
JUSTFILE = REPO / "Justfile"

TIERS = {"fast", "matrix", "proof"}
PLATFORMS = {"any", "linux", "macos"}
# A `just <recipe>` token anywhere in a run script: at the start, after a
# newline, or after a shell separator (`&&`, `||`, `;`, `|`, `(`), and followed
# by whitespace, a separator, or the end.
JUST_INVOCATION = re.compile(r"(?:^|[\s;&|(])just\s+([a-z][a-z0-9-]*)(?=[\s;&|)]|$)", re.MULTILINE)


def load(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def just_recipes() -> set[str]:
    proc = subprocess.run(
        ["just", "--summary"], capture_output=True, text=True, check=True, cwd=REPO
    )
    return set(proc.stdout.split())


def recipe_dependencies(recipe: str, text: str | None = None) -> list[str]:
    """The recipes `just <recipe>` chains, straight from the Justfile."""
    text = JUSTFILE.read_text(encoding="utf-8") if text is None else text
    # `[ \t]*`, not `\s*`: a leaf recipe's dependency line is empty and `\s`
    # would swallow the newline and read the recipe body as dependencies. A
    # recipe may take parameters (`name *ARGS:`); they are not dependencies.
    match = re.search(
        rf"^{re.escape(recipe)}(?:\s+[+*]?[A-Za-z_][A-Za-z0-9_]*(?:=\S+)?)*:[ \t]*(.*)$",
        text,
        re.MULTILINE,
    )
    if not match:
        raise RuntimeError(f"Justfile has no `{recipe}:` recipe")
    return match.group(1).split()


def gate_recipe_dependencies() -> list[str]:
    return recipe_dependencies("gate")


def expand_recipe(recipe: str, text: str | None = None) -> set[str]:
    """Every leaf recipe `just <recipe>` runs, following chained recipes
    (`proof: gate matrix …` expands `gate` and `matrix`)."""
    text = JUSTFILE.read_text(encoding="utf-8") if text is None else text
    leaves: set[str] = set()
    pending = [recipe]
    seen: set[str] = set()
    while pending:
        current = pending.pop()
        if current in seen:
            continue
        seen.add(current)
        deps = recipe_dependencies(current, text)
        if deps:
            pending.extend(deps)
        else:
            leaves.add(current)
    leaves.discard(recipe)
    return leaves


def run_steps(document: object) -> list[dict]:
    """Every step with a `run:` script under every job of a parsed workflow."""
    steps: list[dict] = []
    jobs = document.get("jobs", {}) if isinstance(document, dict) else {}
    for job in jobs.values():
        if not isinstance(job, dict):
            continue
        for step in job.get("steps", []) or []:
            if isinstance(step, dict) and isinstance(step.get("run"), str):
                steps.append(step)
    return steps


def run_scripts(document: object) -> list[str]:
    """Every `run:` value under every job/step of a parsed workflow."""
    return [step["run"] for step in run_steps(document)]


# Shell shapes that let a gate fail without failing the step. Presence of the
# `just <gate>` token proves the gate is *invoked*; these prove it is not
# *enforced*, which is the property parity is supposed to deliver.
UNENFORCED_SHELL = re.compile(
    r"\|\|\s*true\b|\|\|\s*:|;\s*true\s*$|set\s+\+e|\bexit\s+0\b|\|\|\s*echo\b", re.MULTILINE
)


def unenforced_gate_steps(document: object, recipes_of_interest: set[str]) -> list[str]:
    """Steps that invoke an inventory recipe but cannot fail the job."""
    problems: list[str] = []
    for step in run_steps(document):
        script = step["run"]
        invoked = {m.group(1) for m in JUST_INVOCATION.finditer(script)}
        gates = sorted(invoked & recipes_of_interest)
        if not gates:
            continue
        label = step.get("name") or step.get("id") or f"`{script.strip().splitlines()[0]}`"
        if step.get("continue-on-error"):
            problems.append(
                f"step {label} runs {gates} with continue-on-error: the gate cannot fail the job"
            )
        if "if" in step:
            problems.append(
                f"step {label} runs {gates} under `if: {step['if']}`: the gate is conditional"
            )
        if UNENFORCED_SHELL.search(script):
            problems.append(
                f"step {label} runs {gates} in a script that masks failure: {script.strip()!r}"
            )
    return problems


def workflow_invocations(workflows_dir: Path) -> dict[str, set[str]]:
    """recipe -> set of workflow file names that invoke it."""
    found: dict[str, set[str]] = {}
    for path in sorted(workflows_dir.glob("*.yml")):
        document = yaml.safe_load(path.read_text(encoding="utf-8"))
        for script in run_scripts(document):
            for match in JUST_INVOCATION.finditer(script):
                found.setdefault(match.group(1), set()).add(path.name)
    return found


def workflow_enforcement_problems(workflows_dir: Path, recipes_of_interest: set[str]) -> list[str]:
    problems: list[str] = []
    for path in sorted(workflows_dir.glob("*.yml")):
        document = yaml.safe_load(path.read_text(encoding="utf-8"))
        problems.extend(
            f"{path.name}: {p}" for p in unenforced_gate_steps(document, recipes_of_interest)
        )
    return problems


def validate(
    inventory: dict,
    required: dict,
    recipes: set[str],
    invocations: dict[str, set[str]],
    gate_deps: list[str],
    matrix_deps: list[str] | None = None,
    proof_leaves: set[str] | None = None,
    enforcement_problems: list[str] | None = None,
) -> list[str]:
    problems: list[str] = list(enforcement_problems or [])
    gates = inventory.get("gates", [])
    ids = [gate.get("id") for gate in gates]

    seen: set[str] = set()
    for gate_id in ids:
        if not gate_id:
            problems.append("inventory gate without an id")
            continue
        if gate_id in seen:
            problems.append(f"duplicate gate id: {gate_id}")
        seen.add(gate_id)

    by_id = {gate["id"]: gate for gate in gates if gate.get("id")}

    for gate in gates:
        gate_id = gate.get("id", "<missing>")
        if gate.get("tier") not in TIERS:
            problems.append(f"{gate_id}: unknown tier {gate.get('tier')!r}")
        for platform in gate.get("platforms", []):
            if platform not in PLATFORMS:
                problems.append(f"{gate_id}: unknown platform {platform!r}")
        recipe = gate.get("recipe")
        if recipe not in recipes:
            problems.append(f"{gate_id}: recipe {recipe!r} is not a Justfile recipe")
        runs_in = set(gate.get("runs_in", []))
        if "ci" in runs_in and recipe not in invocations:
            problems.append(
                f"{gate_id}: declared to run in CI but no workflow invokes `just {recipe}`"
            )
        if (
            "ci" in runs_in
            and gate.get("workflow")
            and gate["workflow"] not in invocations.get(recipe, set())
        ):
            problems.append(
                f"{gate_id}: declared in workflow {gate['workflow']} "
                f"but that file does not invoke `just {recipe}`"
            )

    for recipe, files in invocations.items():
        if recipe not in {gate.get("recipe") for gate in gates}:
            problems.append(
                f"workflow {sorted(files)} invokes `just {recipe}`, which is not in the inventory"
            )

    required_ids = required.get("required", [])
    if len(set(required_ids)) != len(required_ids):
        problems.append("required.json lists an id more than once")
    for gate_id in required_ids:
        if gate_id not in by_id:
            problems.append(f"required gate {gate_id!r} is missing from the inventory")
    for gate_id in required.get("platform_conditional", {}):
        if gate_id not in required_ids:
            problems.append(f"platform_conditional names {gate_id!r}, which is not required")

    fast_ids = [gate["id"] for gate in gates if gate.get("tier") == "fast" and gate.get("id")]
    if sorted(fast_ids) != sorted(gate_deps):
        problems.append(
            f"`just gate` chains {sorted(gate_deps)} "
            f"but the inventory's fast tier is {sorted(fast_ids)}"
        )
    if matrix_deps is not None:
        matrix_ids = [
            gate["id"] for gate in gates if gate.get("tier") == "matrix" and gate.get("id")
        ]
        if sorted(matrix_ids) != sorted(matrix_deps):
            problems.append(
                f"`just matrix` chains {sorted(matrix_deps)} "
                f"but the inventory's matrix tier is {sorted(matrix_ids)}"
            )
    if proof_leaves is not None:
        required_set = set(required_ids)
        missing = sorted(required_set - proof_leaves)
        extra = sorted(proof_leaves - required_set)
        if missing:
            problems.append(
                f"`just proof` skips required gate(s) {missing}: a local proof is not the CI proof"
            )
        if extra:
            problems.append(f"`just proof` runs {extra}, which required.json does not require")

    return problems


def main() -> int:
    try:
        inventory = load(INVENTORY)
        required = load(REQUIRED)
        recipes = just_recipes()
        invocations = workflow_invocations(WORKFLOWS)
        gate_deps = gate_recipe_dependencies()
        matrix_deps = recipe_dependencies("matrix")
        proof_leaves = expand_recipe("proof")
        inventory_recipes = {g.get("recipe") for g in inventory.get("gates", []) if g.get("recipe")}
        enforcement = workflow_enforcement_problems(WORKFLOWS, inventory_recipes)
    except (
        OSError,
        json.JSONDecodeError,
        subprocess.CalledProcessError,
        RuntimeError,
        yaml.YAMLError,
    ) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    problems = validate(
        inventory,
        required,
        recipes,
        invocations,
        gate_deps,
        matrix_deps,
        proof_leaves,
        enforcement,
    )
    if problems:
        print("gate inventory FAILED:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    print(
        f"gate inventory OK: {len(inventory['gates'])} gates, "
        f"{len(required['required'])} required, "
        f"parity with {len(invocations)} workflow invocations"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
