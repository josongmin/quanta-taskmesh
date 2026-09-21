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
  parity is about enforcement, and a masked gate is a green check over nothing,
- a recipe script that git does not track (untracked, or swallowed by a
  .gitignore pattern — `tools/coverage/report.sh` sat under an unanchored
  `coverage/` rule and never reached CI): a gate whose script exists only on
  one machine is local luck, not a gate,
- a self-reporting recipe (its script prints `taskmesh-<gate> status=…`) whose
  gate declares no `status_line`, or a `status_line` whose marker no script of
  that recipe prints: the runner keeps the recipe's final marker line verbatim
  in the receipt and requires the declared token on it, so a missing
    declaration makes exit 0 alone a PASS and loses the verdict line (the
    bounded output tail does not reliably contain it), while a wrong marker
    makes every run NOT_RUN.
- an action ref that is not a full commit SHA, a workflow default token that
  is not read-only, or a write-capable job reachable outside trusted main.

Workflow invocations are read from the parsed YAML: every `run:` script of
every step, whatever its shape (`- run: just x`, a `|` block, `cd d && just x`,
`just x --flag`). A regex over single lines saw only the simplest form.

It does not run anything; `tools/gates/run.py` does.
"""

from __future__ import annotations

import json
import os
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

TIERS = {"fast", "matrix", "proof", "release"}
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


def recipe_body(recipe: str, text: str | None = None) -> str:
    """The indented body lines of `just <recipe>` (the commands it runs)."""
    text = JUSTFILE.read_text(encoding="utf-8") if text is None else text
    match = re.search(
        rf"^{re.escape(recipe)}(?:\s+[+*]?[A-Za-z_][A-Za-z0-9_]*(?:=\S+)?)*:[^\n]*\n"
        r"((?:[ \t]+[^\n]*\n?)*)",
        text,
        re.MULTILINE,
    )
    if not match:
        raise RuntimeError(f"Justfile has no `{recipe}:` recipe")
    return match.group(1)


SCRIPT_PATH = re.compile(r"\btools/[\w./-]+\.(?:sh|py)\b")
# A self-report line: `taskmesh-<gate> status=<VERDICT> …` on stdout. The
# receipt keeps only a bounded tail of a gate's output, so this line is the one
# thing about a run that must survive verbatim.
STATUS_MARKER = re.compile(r"(taskmesh-[a-z0-9-]+) status=")
FULL_ACTION_REF = re.compile(r"^[^\s@]+@[0-9a-f]{40}$")
PRODUCER_FIELDS = {
    "registration",
    "manifest",
    "producer_id",
    "version",
    "surface",
    "hosted_job",
    "artifact_name",
    "artifact_download_path",
    "required_configs",
    "summary",
    "envelope",
}

QUALIFICATION_MARGIN_SECONDS = 3600
PRODUCER_JOB_MARGIN_SECONDS = 600


def producer_handoff_problems(inventory: dict, workflow_path: Path) -> list[str]:
    """Validate producer job authority, artifact handoff, and hosted time budgets."""
    document = yaml.safe_load(workflow_path.read_text(encoding="utf-8"))
    jobs = document.get("jobs", {}) if isinstance(document, dict) else {}
    if not isinstance(jobs, dict):
        return ["ci.yml jobs are missing or malformed"]
    qualification = jobs.get("qualification")
    if not isinstance(qualification, dict):
        return ["ci.yml qualification job is missing"]
    problems: list[str] = []
    needs = qualification.get("needs", [])
    if isinstance(needs, str):
        needs = [needs]
    if not isinstance(needs, list):
        needs = []
    q_steps = qualification.get("steps", [])
    if not isinstance(q_steps, list):
        q_steps = []
    q_runs = "\n".join(str(step.get("run", "")) for step in q_steps if isinstance(step, dict))
    if "--consume-producers" not in q_runs:
        problems.append("qualification must consume prerequisite producer artifacts")
    collectors = [
        step
        for step in q_steps
        if isinstance(step, dict) and step.get("name") == "Collect the receipt"
    ]
    if len(collectors) != 1 or collectors[0].get("if") != "always()":
        problems.append("qualification receipt collector step must run under if: always()")
    else:
        collector = collectors[0]
        collector_env = collector.get("env")
        if (
            not isinstance(collector_env, dict)
            or collector_env.get("TASKMESH_PRODUCER_NEEDS_JSON") != "${{ toJSON(needs) }}"
        ):
            problems.append("qualification receipt must bind actual prerequisite job results")
        if (
            not str(collector.get("run", ""))
            .lstrip()
            .startswith("python3 tools/qualification/receipt.py collect")
        ):
            problems.append("qualification receipt collector requires baseline Python startup")
    downloaded = {
        step.get("with", {}).get("name"): step
        for step in q_steps
        if isinstance(step, dict)
        and str(step.get("uses", "")).startswith("actions/download-artifact@")
        and isinstance(step.get("with"), dict)
    }
    producers = [
        (gate, gate["producer"])
        for gate in inventory.get("gates", [])
        if isinstance(gate, dict) and isinstance(gate.get("producer"), dict)
    ]
    producer_jobs = {producer.get("hosted_job") for _, producer in producers}
    missing_needs = sorted(
        job for job in producer_jobs if isinstance(job, str) and job not in needs
    )
    if missing_needs:
        problems.append(f"qualification needs misses producer jobs {missing_needs}")
    if qualification.get("if") != "always()":
        problems.append("qualification with producer needs must run under if: always()")
    job_gate_seconds: dict[str, int] = {}
    for gate, producer in producers:
        job_name = producer.get("hosted_job")
        artifact_name = producer.get("artifact_name")
        download_path = producer.get("artifact_download_path")
        job = jobs.get(job_name)
        if not isinstance(job, dict):
            problems.append(f"{gate.get('id')}: hosted producer job {job_name!r} is missing")
            continue
        steps = job.get("steps", [])
        if not isinstance(steps, list):
            steps = []
        runs = "\n".join(str(step.get("run", "")) for step in steps if isinstance(step, dict))
        if not re.search(rf"\bjust\s+{re.escape(str(gate.get('recipe')))}\b", runs):
            problems.append(
                f"{gate.get('id')}: hosted producer job {job_name!r} does not run its recipe"
            )
        uploaded = {
            step.get("with", {}).get("name"): step
            for step in steps
            if isinstance(step, dict)
            and str(step.get("uses", "")).startswith("actions/upload-artifact@")
            and step.get("if") == "always()"
            and isinstance(step.get("with"), dict)
        }
        if artifact_name not in uploaded:
            problems.append(
                f"{gate.get('id')}: job {job_name!r} does not always upload {artifact_name!r}"
            )
        if artifact_name not in downloaded:
            problems.append(f"{gate.get('id')}: qualification does not download {artifact_name!r}")
        else:
            download = downloaded[artifact_name]
            actual_download_path = download.get("with", {}).get("path")
            if actual_download_path != download_path:
                problems.append(
                    f"{gate.get('id')}: artifact {artifact_name!r} download path differs from "
                    "the registered extraction root"
                )
            if download.get("continue-on-error") is not True:
                problems.append(
                    f"{gate.get('id')}: missing artifact {artifact_name!r} would skip receipt"
                )
        upload = uploaded.get(artifact_name)
        if isinstance(upload, dict):
            raw_paths = upload.get("with", {}).get("path", "")
            paths = [line.strip() for line in str(raw_paths).splitlines() if line.strip()]
            common = (
                os.path.commonpath(paths) if len(paths) > 1 else (paths[0] if paths else "")
            ) or "."
            if common != download_path:
                problems.append(
                    f"{gate.get('id')}: artifact {artifact_name!r} does not reconstruct "
                    "its registered repository path"
                )
        gate_seconds = gate.get("timeout_seconds", 3600)
        if isinstance(job_name, str) and type(gate_seconds) is int:
            job_gate_seconds[job_name] = job_gate_seconds.get(job_name, 0) + gate_seconds
    for job_name, gate_seconds in job_gate_seconds.items():
        job = jobs.get(job_name, {})
        job_minutes = job.get("timeout-minutes", 360) if isinstance(job, dict) else None
        if type(job_minutes) is not int or (
            gate_seconds > job_minutes * 60 - PRODUCER_JOB_MARGIN_SECONDS
        ):
            problems.append(
                f"{job_name}: serial producer gate timeouts exceed the job budget or margin"
            )
    budget = inventory.get("qualification_budget_seconds")
    qualification_minutes = qualification.get("timeout-minutes")
    if type(budget) is not int or budget <= 0:
        problems.append("qualification_budget_seconds must be a positive integer")
    elif type(qualification_minutes) is not int or (
        budget > qualification_minutes * 60 - QUALIFICATION_MARGIN_SECONDS
    ):
        problems.append(
            "qualification critical-path deadline exceeds its job budget or required margin"
        )
    return problems


def recipe_scripts(recipe: str, text: str | None = None) -> list[str]:
    """The `tools/…` script paths named in the body of `just <recipe>`."""
    return SCRIPT_PATH.findall(recipe_body(recipe, text))


def tracked_files(root: Path = REPO) -> set[str]:
    """Paths git tracks, relative to the repository root. Ignored and
    untracked files are absent by construction."""
    proc = subprocess.run(
        ["git", "ls-files", "-z"], capture_output=True, text=True, check=True, cwd=root
    )
    return {path for path in proc.stdout.split("\0") if path}


def untracked_recipe_scripts(recipe: str, tracked: set[str], text: str | None = None) -> list[str]:
    """Scripts `just <recipe>` runs that git does not track."""
    return [script for script in recipe_scripts(recipe, text) if script not in tracked]


def self_report_markers(recipe: str, text: str | None = None, root: Path = REPO) -> set[str]:
    """The status-line markers printed by the scripts `just <recipe>` runs
    (found by reading those scripts' sources). Empty for a recipe that runs
    cargo/uv directly."""
    markers: set[str] = set()
    for script in recipe_scripts(recipe, text):
        path = root / script
        if path.is_file():
            markers.update(STATUS_MARKER.findall(path.read_text(encoding="utf-8")))
    return markers


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


def workflow_trust_problems(path: Path, document: object) -> list[str]:
    """Validate immutable action identity and least-privilege token boundaries."""
    if not isinstance(document, dict):
        return [f"{path.name}: workflow is not an object"]
    problems: list[str] = []
    if document.get("permissions") != {"contents": "read"}:
        problems.append(f"{path.name}: top-level permissions must be exactly contents: read")
    jobs = document.get("jobs", {})
    if not isinstance(jobs, dict):
        return [*problems, f"{path.name}: jobs is not an object"]
    for job_id, job in jobs.items():
        if not isinstance(job, dict):
            continue
        condition = str(job.get("if", ""))
        permissions = job.get("permissions", {})
        writes = (
            [key for key, value in permissions.items() if value == "write"]
            if isinstance(permissions, dict)
            else []
        )
        trusted_main = (
            "github.event_name == 'push'" in condition
            and "github.ref == 'refs/heads/main'" in condition
        )
        if writes and not trusted_main:
            problems.append(
                f"{path.name}:{job_id}: write permissions {writes} are not restricted to "
                "a trusted main push"
            )
        for step in job.get("steps", []) or []:
            if not isinstance(step, dict):
                continue
            action = step.get("uses")
            if (
                isinstance(action, str)
                and not action.startswith("./")
                and not FULL_ACTION_REF.fullmatch(action)
            ):
                problems.append(f"{path.name}:{job_id}: mutable action ref {action!r}")
            with_values = step.get("with", {})
            if isinstance(with_values, dict) and "github-token" in with_values and not trusted_main:
                problems.append(f"{path.name}:{job_id}: PR-reachable action receives github-token")
    return problems


def all_workflow_trust_problems(workflows_dir: Path) -> list[str]:
    problems: list[str] = []
    for path in sorted(workflows_dir.glob("*.yml")):
        document = yaml.safe_load(path.read_text(encoding="utf-8"))
        problems.extend(workflow_trust_problems(path, document))
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
    self_reports: dict[str, set[str]] | None = None,
    untracked_scripts: dict[str, list[str]] | None = None,
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
    producer_registrations: set[str] = set()

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
        timeout_seconds = gate.get("timeout_seconds", 3600)
        if type(timeout_seconds) is not int or timeout_seconds <= 0 or timeout_seconds > 21600:
            problems.append(f"{gate_id}: timeout_seconds must be an integer in 1..21600")
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
        producer = gate.get("producer")
        if producer is not None:
            if not isinstance(producer, dict):
                problems.append(f"{gate_id}: producer registration is not an object")
            else:
                missing_fields = sorted(PRODUCER_FIELDS - set(producer))
                if missing_fields:
                    problems.append(
                        f"{gate_id}: producer registration misses fields {missing_fields}"
                    )
                registration = producer.get("registration")
                if not isinstance(registration, str) or not registration:
                    problems.append(f"{gate_id}: producer registration id is empty")
                elif registration in producer_registrations:
                    problems.append(f"duplicate producer registration: {registration}")
                else:
                    producer_registrations.add(registration)
                for field in PRODUCER_FIELDS - {"registration", "required_configs"}:
                    value = producer.get(field)
                    if not isinstance(value, str) or not value:
                        problems.append(f"{gate_id}: producer {field} is empty")
                required_configs = producer.get("required_configs")
                if (
                    not isinstance(required_configs, list)
                    or not required_configs
                    or not all(isinstance(path, str) and path for path in required_configs)
                ):
                    problems.append(f"{gate_id}: producer required_configs is empty or malformed")
                else:
                    if len(required_configs) != len(set(required_configs)):
                        problems.append(f"{gate_id}: producer required_configs contains duplicates")
                    tracked = tracked_files()
                    for path in required_configs:
                        if Path(path).is_absolute() or ".." in Path(path).parts:
                            problems.append(
                                f"{gate_id}: producer required config {path!r} is not repo-relative"
                            )
                        elif path not in tracked:
                            problems.append(
                                f"{gate_id}: producer required config {path!r} is not tracked"
                            )
                for field in ("manifest", "summary", "envelope"):
                    value = producer.get(field)
                    if isinstance(value, str) and (
                        Path(value).is_absolute() or ".." in Path(value).parts
                    ):
                        problems.append(f"{gate_id}: producer {field} is not repo-relative")
                    if field in {"summary", "envelope"} and isinstance(value, str):
                        parts = Path(value).parts
                        if len(parts) < 2 or parts[0] != "target":
                            problems.append(
                                f"{gate_id}: producer {field} must be below repository target/"
                            )
                manifest = producer.get("manifest")
                if isinstance(manifest, str) and manifest not in tracked_files():
                    problems.append(f"{gate_id}: producer manifest {manifest!r} is not tracked")
        if untracked_scripts is not None:
            for script in untracked_scripts.get(recipe, []):
                problems.append(
                    f"{gate_id}: `just {recipe}` runs {script}, which git does not track "
                    "(untracked or ignored) — CI cannot run this gate"
                )
        if self_reports is not None:
            printed = self_reports.get(recipe, set())
            spec = gate.get("status_line")
            if spec is not None and (
                not isinstance(spec, dict)
                or not isinstance(spec.get("marker"), str)
                or not isinstance(spec.get("require"), str)
                or not spec["marker"]
                or not spec["require"]
            ):
                problems.append(
                    f"{gate_id}: status_line must be {{marker: str, require: str}}, got {spec!r}"
                )
            elif spec is not None and spec["marker"] not in printed:
                problems.append(
                    f"{gate_id}: status_line marker {spec['marker']!r} is not printed by any "
                    f"script `just {recipe}` runs (found {sorted(printed)}): every run would "
                    "be NOT_RUN"
                )
            elif spec is None and printed:
                problems.append(
                    f"{gate_id}: `just {recipe}` self-reports ({sorted(printed)}) but the "
                    "inventory declares no status_line — the receipt would drop the verdict "
                    "line and exit 0 alone would count as PASS"
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
    required_producers = required.get("required_producers")
    if not isinstance(required_producers, dict):
        problems.append("required.json lacks required_producers")
    else:
        registered = {
            gate.get("producer", {}).get("registration"): gate.get("id")
            for gate in gates
            if isinstance(gate.get("producer"), dict)
            and isinstance(gate["producer"].get("registration"), str)
        }
        if registered != required_producers:
            problems.append(
                f"producer registrations {registered!r} do not match required_producers "
                f"{required_producers!r}"
            )

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
        enforcement = [
            *workflow_enforcement_problems(WORKFLOWS, inventory_recipes),
            *all_workflow_trust_problems(WORKFLOWS),
            *producer_handoff_problems(inventory, WORKFLOWS / "ci.yml"),
        ]
        self_reports = {
            recipe: self_report_markers(recipe) for recipe in inventory_recipes if recipe in recipes
        }
        tracked = tracked_files()
        untracked_scripts = {
            recipe: untracked_recipe_scripts(recipe, tracked)
            for recipe in inventory_recipes
            if recipe in recipes
        }
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
        self_reports,
        untracked_scripts,
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
