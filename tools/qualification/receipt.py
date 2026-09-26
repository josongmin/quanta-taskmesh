#!/usr/bin/env python3
"""Qualification receipts (H16-022).

A receipt binds a set of gate results to the *exact* source they ran against.
"Exact" includes the working tree, not just `HEAD`: a dirty overlay on the same
commit is a different source, and a receipt that names only the commit could be
reused for code that was never tested. So the source identity is a digest over
every tracked and untracked file's path, content, mode, and symlink target.

Two commands:

    collect   run the canonical gates via tools/gates/run.py, ingest each
              registered producer envelope/summary, and write a receipt
    validate  check a receipt: schema, source identity against the current
              tree, and that every required gate ran and passed — anything
              else is NOT_QUALIFIED, reported with the reason

A receipt is never "qualified" by default. It becomes qualified only when the
validator can re-derive the same source identity and every required gate has a
PASS result attached.

Attestation boundary — what `validate` proves and what it does not
------------------------------------------------------------------
`validate` re-derives the **source identity** from the tree it runs in: the
path digest, `HEAD`, and whether the tree is dirty. A receipt whose recorded
identity disagrees with any of those is NOT_QUALIFIED whatever it claims about
gates. It also checks the receipt's internal consistency (sub-receipts agree
with the top level about the tree they saw).

What `validate` cannot re-derive is the **gate results** themselves: it does
not re-run the gates, so a hand-written receipt that lies about a PASS is only
caught where it also has to lie coherently about the tree. Gate results are
therefore *attested by the collector*, and the qualification rule in
`docs/release-checklist.md` is that the receipt of record is produced by
`collect --local-qualified` from a clean local checkout where every required
gate passes. Hosted collection remains a disabled compatibility path. A green
`validate` is a necessary condition, not evidence that omitted gates ran.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import NamedTuple

REPO = Path(__file__).resolve().parents[2]
if str(REPO) not in sys.path:
    sys.path.insert(0, str(REPO))

from tools.gates.status_line import pass_status_line_problems  # noqa: E402
from tools.process_supervisor import run_process  # noqa: E402
from tools.qualification.evidence import (  # noqa: E402
    canonical_digest,  # noqa: F401 - compatibility re-export for callers
    git_source_paths,
    head_worktree_mismatches,
    runtime_action,
    source_tree_digest,
)
from tools.qualification.producer_evidence import (  # noqa: E402
    clear_registered_outputs,
    collect_records,
    enrich_gate_results,
    producer_records_problems,
    registrations,
)

REQUIRED = REPO / "tools" / "gates" / "required.json"
INVENTORY = REPO / "tools" / "gates" / "inventory.json"
SCHEMA_VERSION = 4
GATE_SCHEMA_VERSION = 3
MUTATION_GATE_ID = "mutants-critical"
RUNNER_FINALIZATION_GRACE_SECONDS = 120
ATTESTATION_CHECK_NAMES = {
    "github_actions",
    "ci",
    "head_matches_github_sha",
    "workspace_matches_repo",
    "source_clean",
    "workflow_is_ci",
    "job_is_qualification",
    "action_source_matches",
    "producer_jobs_success",
}
LOCAL_ATTESTATION_CHECK_NAMES = {
    "not_github_actions",
    "workspace_matches_repo",
    "source_clean",
    "action_is_local",
    "action_source_matches",
}
# A receipt without these was not produced by `collect`; whatever it says about
# gates, it is not the shape the collector attests.
REQUIRED_RECEIPT_KEYS = (
    "generated_at",
    "finished_at",
    "environment",
    "gate_runner",
    "source",
    "source_after",
    "attestation",
    "gates",
    "mutations",
    "generated_mutation_sweep",
    "producers",
)


def _git(*args: str, root: Path = REPO) -> str:
    return subprocess.run(
        ["git", *args], cwd=root, capture_output=True, text=True, check=True
    ).stdout.strip()


def source_identity(root: Path | None = None) -> dict:
    """HEAD plus a digest over the whole working tree (tracked + untracked).

    `root` is the repository to describe (default: this one, resolved at call
    time); tests point it at a throwaway repository so that exercising the
    digest never writes into the live tree.
    """
    root = REPO if root is None else root
    head = _git("rev-parse", "HEAD", root=root)
    tree = _git("rev-parse", "HEAD^{tree}", root=root)
    status = _git("status", "--porcelain=v1", "--untracked-files=all", root=root)
    head_mismatches = head_worktree_mismatches(root)
    dirty = bool(status.strip()) or bool(head_mismatches)

    # This is exactly the V02 campaign source set. Git already excludes ignored
    # build outputs, while tracked historic receipts remain source inputs.
    paths = git_source_paths(root)
    return {
        "head": head,
        "tree": tree,
        "dirty": dirty,
        "isolated_checkout": False,
        "paths_digest": source_tree_digest(root, paths),
        "paths_counted": len(paths),
        "status": status.splitlines() + [f"HEAD_MISMATCH {path}" for path in head_mismatches],
    }


def environment() -> dict:
    try:
        rustc = subprocess.run(["rustc", "-Vv"], capture_output=True, text=True, check=False).stdout
    except OSError:
        rustc = ""
    lock = REPO / "Cargo.lock"
    return {
        "toolchain": rustc.strip().splitlines()[0] if rustc else "unknown",
        "rustc_verbose": rustc.strip(),
        "lock_digest": hashlib.sha256(lock.read_bytes()).hexdigest() if lock.exists() else None,
        "os_target": f"{platform.system().lower()}-{platform.machine()}",
        "runner": platform.node(),
        "python": sys.version.split()[0],
    }


def collection_attestation(source: dict, hosted_ci: bool, local_qualified: bool = False) -> dict:
    if hosted_ci and local_qualified:
        raise ValueError("hosted and local qualification modes are mutually exclusive")
    expected_jobs = {spec["hosted_job"] for spec in registrations(INVENTORY)}
    try:
        needs = json.loads(os.environ.get("TASKMESH_PRODUCER_NEEDS_JSON", ""))
    except json.JSONDecodeError:
        needs = None
    producer_jobs = (
        {job: value.get("result") for job, value in needs.items() if isinstance(value, dict)}
        if isinstance(needs, dict)
        else {}
    )
    try:
        action = runtime_action(
            root=REPO,
            source_head=source.get("head", ""),
            local_workflow=".github/workflows/ci.yml",
            local_job="local-qualification",
        )
    except (OSError, RuntimeError, ValueError) as exc:
        action = {"error": str(exc)}
    hosted_checks = {
        "github_actions": os.environ.get("GITHUB_ACTIONS") == "true",
        "ci": os.environ.get("CI") == "true",
        "head_matches_github_sha": os.environ.get("GITHUB_SHA") == source.get("head"),
        "workspace_matches_repo": (
            bool(os.environ.get("GITHUB_WORKSPACE"))
            and Path(os.environ["GITHUB_WORKSPACE"]).resolve() == REPO.resolve()
        ),
        "source_clean": not source.get("dirty", True),
        "workflow_is_ci": action.get("workflow") == ".github/workflows/ci.yml",
        "job_is_qualification": action.get("job") == "qualification",
        "action_source_matches": action.get("source_sha") == source.get("head"),
        "producer_jobs_success": set(producer_jobs) == expected_jobs
        and all(result == "success" for result in producer_jobs.values()),
    }
    local_checks = {
        "not_github_actions": os.environ.get("GITHUB_ACTIONS") != "true",
        "workspace_matches_repo": Path.cwd().resolve() == REPO.resolve(),
        "source_clean": not source.get("dirty", True),
        "action_is_local": action.get("context") == "local" and action.get("event") == "local",
        "action_source_matches": action.get("source_sha") == source.get("head"),
    }
    hosted_eligible = hosted_ci and all(hosted_checks.values())
    local_eligible = local_qualified and all(local_checks.values())
    return {
        "kind": "github-actions" if hosted_ci else "local",
        "hosted_qualification_eligible": hosted_eligible,
        "local_qualification_eligible": local_eligible,
        "head": source.get("head"),
        "checks": hosted_checks if hosted_ci else local_checks,
        "action": action,
        "producer_jobs": producer_jobs,
    }


class JsonRun(NamedTuple):
    payload: dict
    exit_code: int | None
    started_at: str
    duration_s: float
    timed_out: bool = False
    interrupted_by_signal: int | None = None


def run_json(cmd: list[str], receipt_path: Path, *, timeout_seconds: float) -> JsonRun:
    # Never mistake a sidecar from an earlier collection for this process's
    # output when the process dies before writing anything.
    receipt_path.unlink(missing_ok=True)
    started_at = datetime.now(timezone.utc).isoformat()
    started = time.monotonic()
    process = run_process(
        cmd + ["--receipt", str(receipt_path)],
        cwd=REPO,
        env=os.environ.copy(),
        timeout_seconds=timeout_seconds,
    )
    duration_s = round(time.monotonic() - started, 3)
    if receipt_path.exists():
        try:
            payload = json.loads(receipt_path.read_text(encoding="utf-8"))
        except (UnicodeError, json.JSONDecodeError) as exc:
            payload = {"error": f"runner wrote invalid JSON or UTF-8: {type(exc).__name__}: {exc}"}
        except OSError as exc:
            payload = {"error": f"runner sidecar could not be read: {exc}"}
    else:
        payload = {
            "error": "runner did not write its receipt",
            "stderr_tail": process.stderr[-2000:],
        }
    return JsonRun(
        payload,
        process.returncode,
        started_at,
        duration_s,
        process.timed_out,
        process.interrupted_by_signal,
    )


def required_gate_ids() -> list[str]:
    return json.loads(REQUIRED.read_text(encoding="utf-8"))["required"]


def summarize_gate_results(results: list[dict]) -> dict:
    required = set(required_gate_ids())
    ran = {result.get("id") for result in results if isinstance(result, dict)}
    not_run = sorted(
        (required - ran)
        | {
            result.get("id")
            for result in results
            if isinstance(result, dict)
            and result.get("id") in required
            and result.get("status") == "NOT_RUN"
        }
    )
    not_passed = sorted(
        result.get("id")
        for result in results
        if isinstance(result, dict)
        and result.get("id") in required
        and result.get("status") != "PASS"
    )
    return {
        "qualified": not not_run and not not_passed,
        "required_not_run": not_run,
        "required_not_passed": not_passed,
    }


def gate_runner_problems(
    value: object,
    gate_results: list[dict],
    producer_specs: list[dict],
    *,
    hosted_attested: bool,
) -> list[str]:
    """Validate the gate-runner process separately from its JSON sidecar."""
    if not isinstance(value, dict):
        return ["gate runner process evidence is missing or malformed"]
    problems: list[str] = []
    exit_code = value.get("exit_code")
    started_at = value.get("started_at")
    duration_s = value.get("duration_s")
    skipped = value.get("skipped_gate_ids")
    raw_required = value.get("raw_required")
    if type(exit_code) is not int:
        problems.append("gate runner exit_code must be an integer")
    if value.get("timed_out") is not False:
        problems.append("gate runner timed_out/timeout invalidates qualification")
    if "interrupted_by_signal" not in value or value["interrupted_by_signal"] is not None:
        problems.append("gate runner interrupted_by_signal invalidates qualification")
    if not isinstance(started_at, str) or not started_at:
        problems.append("gate runner started_at is missing")
    if not (
        (type(duration_s) is int and duration_s >= 0)
        or (type(duration_s) is float and math.isfinite(duration_s) and duration_s >= 0)
    ):
        problems.append("gate runner duration_s must be non-negative")
    if not isinstance(skipped, list) or any(not isinstance(gate_id, str) for gate_id in skipped):
        problems.append("gate runner skipped_gate_ids are malformed")
        skipped = []
    elif skipped != sorted(set(skipped)):
        problems.append("gate runner skipped_gate_ids are duplicated or unsorted")
    raw_results = [result for result in gate_results if result.get("id") not in set(skipped)]
    recomputed_raw = summarize_gate_results(raw_results)
    if raw_required != recomputed_raw:
        problems.append("gate runner raw required summary differs from pre-import results")

    expected_imports = sorted(spec["gate_id"] for spec in producer_specs)
    imported = sorted(
        result["id"]
        for result in gate_results
        if result.get("imported_producer") is True and isinstance(result.get("id"), str)
    )
    if skipped:
        if not hosted_attested:
            problems.append("only hosted qualification may import skipped producer gates")
        if skipped != expected_imports or imported != expected_imports:
            problems.append("gate runner producer import set differs from inventory")
        if exit_code != 1:
            problems.append("hosted producer-import gate runner must exit 1 before enrichment")
        if isinstance(raw_required, dict) and (
            raw_required.get("required_not_run") != expected_imports
            or raw_required.get("required_not_passed") != []
        ):
            problems.append("hosted gate runner non-pass is not limited to imported producers")
    else:
        if imported:
            problems.append("direct gate runner receipt cannot contain imported producer gates")
        if exit_code != 0:
            problems.append("direct gate runner did not exit cleanly")
        if isinstance(raw_required, dict) and raw_required.get("qualified") is not True:
            problems.append("direct gate runner raw required set is not qualified")
    return problems


def finalize_gate_receipt(gates: dict) -> dict:
    results = gates.get("results")
    if not isinstance(results, list):
        results = []
        gates["results"] = results
    gates["schema_version"] = GATE_SCHEMA_VERSION
    gates.update(summarize_gate_results(results))
    gates["finalized_at"] = datetime.now(timezone.utc).isoformat()
    return gates


def collect(
    out: Path,
    tiers: list[str] | None,
    skip_mutations: bool,
    hosted_ci: bool = False,
    consume_producers: bool = False,
    local_qualified: bool = False,
) -> int:
    if consume_producers and not hosted_ci:
        raise ValueError("imported producer artifacts are accepted only in hosted CI")
    started = datetime.now(timezone.utc).isoformat()
    source = source_identity()
    attestation = collection_attestation(source, hosted_ci, local_qualified)
    source["isolated_checkout"] = attestation["hosted_qualification_eligible"]
    env = environment()

    producer_specs = registrations(INVENTORY)
    # A producer crash must not be able to leave a prior run's envelope or
    # summary available for collection.
    if not consume_producers:
        clear_registered_outputs(REPO, producer_specs)
    inventory = json.loads(INVENTORY.read_text(encoding="utf-8"))
    deadline_seconds = inventory.get("qualification_budget_seconds")
    if type(deadline_seconds) is not int or deadline_seconds <= 0:
        raise ValueError("inventory qualification_budget_seconds must be a positive integer")
    # No explicit tier means the canonical required set. Keep that as the
    # receipt default so qualification scope follows required.json, not tier
    # composition or optional diagnostic gates.
    gate_args = (
        ["--required"] if tiers is None else [*sum([["--tier", tier] for tier in tiers], [])]
    )
    gate_args += ["--deadline-seconds", str(deadline_seconds)]
    if hosted_ci or local_qualified:
        gate_args.append("--require-clean-source")
    if consume_producers:
        for spec in producer_specs:
            gate_args += ["--skip", spec["gate_id"]]
    elif skip_mutations:
        gate_args += ["--skip", MUTATION_GATE_ID, "--skip", "mutants-generated"]
    gate_run = run_json(
        [sys.executable, "tools/gates/run.py", *gate_args],
        out.with_suffix(".gates.json"),
        timeout_seconds=deadline_seconds + RUNNER_FINALIZATION_GRACE_SECONDS,
    )
    gate_receipt = gate_run.payload
    if not isinstance(gate_receipt, dict):
        gate_receipt = {"error": "gate runner did not produce an object", "results": []}
    producer_records = collect_records(REPO, producer_specs)
    results = gate_receipt.get("results")
    if not isinstance(results, list):
        results = []
        gate_receipt["results"] = results
    raw_required = summarize_gate_results(results)
    skipped_gate_ids = (
        sorted(spec["gate_id"] for spec in producer_specs) if consume_producers else []
    )
    enrich_gate_results(results, producer_records, derive_missing=consume_producers)
    finalize_gate_receipt(gate_receipt)
    # The sidecar is an authoritative summary of the same result set embedded
    # in the combined receipt, not the pre-derivation intermediate document.
    out.with_suffix(".gates.json").write_text(
        json.dumps(gate_receipt, indent=2) + "\n", encoding="utf-8"
    )
    out.with_suffix(".producers.json").write_text(
        json.dumps(producer_records, indent=2) + "\n", encoding="utf-8"
    )

    producer_by_surface = {
        record.get("surface"): record
        for record in producer_records
        if isinstance(record, dict) and isinstance(record.get("surface"), str)
    }
    curated_record = producer_by_surface.get("curated", {})
    generated_record = producer_by_surface.get("generated", {})
    curated_summary = curated_record.get("summary")
    generated_summary = generated_record.get("summary")
    mutation_receipt = curated_summary.get("payload") if isinstance(curated_summary, dict) else None
    generated_receipt = (
        generated_summary.get("payload") if isinstance(generated_summary, dict) else None
    )
    for suffix, payload in (
        (".mutations.json", mutation_receipt),
        (".generated-mutations.json", generated_receipt),
    ):
        sidecar = out.with_suffix(suffix)
        if isinstance(payload, dict):
            sidecar.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
        else:
            sidecar.unlink(missing_ok=True)

    # Source identity is re-derived AFTER the gates: a gate that modified the
    # tree (a mutation runner that failed to restore, a formatter) invalidates
    # the receipt, and this is where that shows.
    source_after = source_identity()

    limitations = []
    if not (
        attestation["hosted_qualification_eligible"] or attestation["local_qualification_eligible"]
    ):
        limitations.append(
            "Evidence-only collection: pass --local-qualified from a clean local Linux checkout "
            "to request a full local qualification verdict."
        )
    if platform.system().lower() != "linux":
        limitations.append(
            "Linux-only gates (bench-iai) are SKIPPED_PLATFORM off Linux and leave the "
            "receipt NOT_QUALIFIED by design."
        )

    receipt = {
        "schema_version": SCHEMA_VERSION,
        "generated_at": started,
        "finished_at": datetime.now(timezone.utc).isoformat(),
        "source": source,
        "source_after": {
            "head": source_after["head"],
            "tree": source_after["tree"],
            "paths_digest": source_after["paths_digest"],
            "dirty": source_after["dirty"],
        },
        "attestation": attestation,
        "environment": env,
        "gate_runner": {
            "exit_code": gate_run.exit_code,
            "started_at": gate_run.started_at,
            "duration_s": gate_run.duration_s,
            "timed_out": gate_run.timed_out,
            "interrupted_by_signal": gate_run.interrupted_by_signal,
            "skipped_gate_ids": skipped_gate_ids,
            "raw_required": raw_required,
        },
        "gates": gate_receipt,
        "mutations": mutation_receipt,
        "generated_mutation_sweep": generated_receipt,
        "producers": producer_records,
        "limitations": limitations,
    }
    verdict = evaluate(receipt, source_after, artifact_root=REPO)
    receipt["verdict"] = verdict
    out.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(f"receipt: {out}")
    print(f"verdict: {verdict['status']}")
    for reason in verdict["reasons"]:
        print(f"  - {reason}")
    return 0 if verdict["status"] == "QUALIFIED" else 1


def evaluate(receipt: object, current: dict | None, *, artifact_root: Path | None = None) -> dict:
    """Decide QUALIFIED / NOT_QUALIFIED with every reason listed.

    `current` is the re-derived source identity of the tree the validator runs
    in (`None` skips the comparison, for a receipt inspected away from its
    tree). Every field of it that the receipt also records must agree; the
    receipt's own claims are never taken over the tree in front of us.
    """
    if not isinstance(receipt, dict):
        return {"status": "NOT_QUALIFIED", "reasons": ["receipt is not an object"]}
    reasons: list[str] = []
    if receipt.get("schema_version") != SCHEMA_VERSION:
        reasons.append(f"schema_version {receipt.get('schema_version')} != {SCHEMA_VERSION}")
    for key in REQUIRED_RECEIPT_KEYS:
        if key not in receipt:
            reasons.append(f"receipt lacks {key!r}: not the shape `collect` produces")
    source = receipt.get("source", {})
    if not isinstance(source, dict):
        reasons.append("receipt source is not an object")
        source = {}
    source_digest = source.get("paths_digest")
    if not isinstance(source_digest, str) or not source_digest:
        reasons.append("receipt source paths_digest is missing or empty")
    attestation = receipt.get("attestation", {})
    if not isinstance(attestation, dict):
        reasons.append("receipt attestation is not an object")
        attestation = {}
    hosted_attested = (
        attestation.get("kind") == "github-actions"
        and attestation.get("hosted_qualification_eligible") is True
    )
    local_attested = (
        attestation.get("kind") == "local"
        and attestation.get("local_qualification_eligible") is True
    )
    if not hosted_attested and not local_attested:
        reasons.append(
            "receipt lacks an eligible hosted or explicit local qualification attestation"
        )
    checks = attestation.get("checks")
    expected_checks = (
        ATTESTATION_CHECK_NAMES
        if hosted_attested
        else LOCAL_ATTESTATION_CHECK_NAMES
        if local_attested
        else set()
    )
    if (
        not isinstance(checks, dict)
        or set(checks) != expected_checks
        or not all(value is True for value in checks.values())
    ):
        reasons.append("qualification attestation checks are incomplete or failed")
    if attestation.get("head") != source.get("head"):
        reasons.append("attestation HEAD does not match receipt source HEAD")
    expected_producer_jobs = {spec["hosted_job"] for spec in registrations(INVENTORY)}
    producer_jobs = attestation.get("producer_jobs")
    if hosted_attested:
        if (
            not isinstance(producer_jobs, dict)
            or set(producer_jobs) != expected_producer_jobs
            or any(result != "success" for result in producer_jobs.values())
        ):
            reasons.append("producer prerequisite jobs were not all successful")
        if source.get("isolated_checkout") is not True:
            reasons.append("receipt source is not marked as an isolated hosted checkout")
    elif local_attested and producer_jobs not in ({}, None):
        reasons.append("local qualification cannot claim hosted producer job results")
    if current is not None:
        if source.get("paths_digest") != current.get("paths_digest"):
            reasons.append(
                "source digest does not match the current working tree (tracked+untracked)"
            )
        if source.get("head") != current.get("head"):
            reasons.append(
                f"receipt HEAD {source.get('head')} is not the current HEAD {current.get('head')}"
            )
        if current.get("dirty") and not source.get("dirty"):
            reasons.append("receipt claims a clean tree but the current working tree is dirty")
        if current.get("dirty"):
            reasons.append("current working tree is dirty: cannot qualify exact local source")
    source_after = receipt.get("source_after", {})
    if not isinstance(source_after, dict):
        reasons.append("receipt source_after is not an object")
        source_after = {}
    for field in ("head", "tree", "paths_digest", "dirty"):
        if field not in source_after:
            reasons.append(f"receipt source_after {field} is missing")
        elif field != "dirty" and (
            not isinstance(source_after.get(field), str) or not source_after.get(field)
        ):
            reasons.append(f"receipt source_after {field} is missing or empty")
        elif source_after.get(field) != source.get(field):
            label = "dirtiness" if field == "dirty" else field
            reasons.append(f"the working tree {label} changed while the gates ran")
    if source.get("dirty"):
        reasons.append(
            "working tree is dirty: local evidence only, not an isolated-source qualification"
        )

    required = required_gate_ids()
    gates = receipt.get("gates", {})
    if not isinstance(gates, dict):
        reasons.append("gate receipt is not an object")
        gates = {}
    gate_results = gates.get("results", [])
    if not isinstance(gate_results, list):
        reasons.append("gate results is not a list")
        gate_results = []
    valid_gate_results = [
        result
        for result in gate_results
        if isinstance(result, dict) and isinstance(result.get("id"), str)
    ]
    gate_ids = [result["id"] for result in valid_gate_results]
    if len(valid_gate_results) != len(gate_results):
        reasons.append("gate receipt has a result without a string id")
    if len(gate_ids) != len(set(gate_ids)):
        reasons.append("gate receipt contains duplicate result ids")
    for result in valid_gate_results:
        if result.get("status") == "PASS":
            exit_code = result.get("exit_code")
            # bool is an int subclass in Python, but it is not a process exit
            # code.  PASS is collector evidence only when the process exited
            # with the exact integer code zero.
            if type(exit_code) is not int or exit_code != 0:
                reasons.append(f"gate {result.get('id')} reports PASS without integer exit_code 0")
            if (
                ("timed_out" in result and result.get("timed_out") is not False)
                or result.get("interrupted_by_signal") is not None
                or result.get("signal") is not None
            ):
                reasons.append(f"gate {result.get('id')} reports PASS after interrupted execution")
    reasons.extend(
        pass_status_line_problems(valid_gate_results, INVENTORY, allow_imported_producers=True)
    )
    # The combined receipt is independently re-opened for release decisions.
    # Recheck the embedded case denominator instead of trusting a PASS line
    # that was emitted by the earlier gate runner.
    from tools.gates.execution_evidence import REPORTS, report_problems

    execution_source = {**source, "isolated_checkout": False}
    for result in valid_gate_results:
        if result.get("id") in REPORTS and result.get("status") == "PASS":
            reasons.extend(
                report_problems(
                    result["id"],
                    result.get("execution"),
                    result.get("status_line"),
                    execution_source,
                )
            )
    computed_summary = summarize_gate_results(valid_gate_results)
    if gates.get("schema_version") != GATE_SCHEMA_VERSION:
        reasons.append(
            f"gate schema_version {gates.get('schema_version')} != {GATE_SCHEMA_VERSION}"
        )
    for key, expected in computed_summary.items():
        if gates.get(key) != expected:
            reasons.append(
                f"gate summary {key}={gates.get(key)!r} does not match results {expected!r}"
            )
    results = {
        result["id"]: result for result in valid_gate_results if isinstance(result.get("id"), str)
    }
    for gate_id in required:
        result = results.get(gate_id)
        if result is None:
            reasons.append(f"required gate {gate_id} was not run")
        elif result.get("status") != "PASS":
            reasons.append(f"required gate {gate_id}: {result.get('status')}")

    try:
        producer_specs = registrations(INVENTORY)
    except (OSError, json.JSONDecodeError, ValueError) as exc:
        reasons.append(f"producer inventory cannot be loaded: {exc}")
    else:
        reasons.extend(
            gate_runner_problems(
                receipt.get("gate_runner"),
                valid_gate_results,
                producer_specs,
                hosted_attested=hosted_attested,
            )
        )
        try:
            reasons.extend(
                producer_records_problems(
                    receipt.get("producers"),
                    producer_specs,
                    results,
                    source,
                    attestation.get("action"),
                    root=artifact_root,
                )
            )
        except Exception as exc:  # untrusted nested JSON must never escape collect
            reasons.append(f"producer evidence is malformed: {type(exc).__name__}")
        producers = receipt.get("producers")
        if isinstance(producers, list):
            surfaces = {
                record.get("surface"): record
                for record in producers
                if isinstance(record, dict) and isinstance(record.get("surface"), str)
            }
            curated_record = surfaces.get("curated", {})
            generated_record = surfaces.get("generated", {})
            curated_summary = curated_record.get("summary")
            generated_summary = generated_record.get("summary")
            curated = curated_summary.get("payload") if isinstance(curated_summary, dict) else None
            generated = (
                generated_summary.get("payload") if isinstance(generated_summary, dict) else None
            )
            if receipt.get("mutations") != curated:
                reasons.append("curated mutation sidecar differs from embedded producer summary")
            if receipt.get("generated_mutation_sweep") != generated:
                reasons.append("generated mutation sidecar differs from embedded producer summary")

    status = "QUALIFIED" if not reasons else "NOT_QUALIFIED"
    computed = {"status": status, "reasons": list(reasons)}
    stored = receipt.get("verdict")
    if stored is not None and stored != computed:
        reasons.append("stored verdict does not match the validator's recomputed verdict")
        status = "NOT_QUALIFIED"
    return {"status": status, "reasons": reasons}


def validate(
    path: Path,
    check_tree: bool,
    *,
    current: dict | None = None,
    artifact_root: Path | None = None,
) -> tuple[int, list[str]]:
    """Exit code plus the reasons, so a caller (or test) can see *why*."""
    try:
        receipt = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        print(f"FAIL: cannot read receipt: {exc}", file=sys.stderr)
        return 1, [f"cannot read receipt: {exc}"]
    if not check_tree:
        current = None
    elif current is None:
        current = source_identity()
    verdict = evaluate(
        receipt, current, artifact_root=REPO if artifact_root is None else artifact_root
    )
    sidecar_reasons: list[str] = []
    if isinstance(receipt, dict):
        for suffix, field in (
            (".gates.json", "gates"),
            (".producers.json", "producers"),
            (".mutations.json", "mutations"),
            (".generated-mutations.json", "generated_mutation_sweep"),
        ):
            sidecar = path.with_suffix(suffix)
            try:
                payload = json.loads(sidecar.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError) as exc:
                sidecar_reasons.append(f"cannot read {field} sidecar: {exc}")
            else:
                if payload != receipt.get(field):
                    sidecar_reasons.append(f"{field} sidecar differs from embedded {field}")
    if sidecar_reasons:
        verdict = {
            "status": "NOT_QUALIFIED",
            "reasons": [*verdict["reasons"], *sidecar_reasons],
        }
    if verdict["status"] == "QUALIFIED":
        # Say what was and was not re-derived, every time: the identity was,
        # the gate results were not (they are the collector's attestation).
        print(
            f"receipt {path}: QUALIFIED (source identity re-derived; gate results "
            "attested by `collect`, not re-run here)"
        )
    else:
        print(f"receipt {path}: {verdict['status']}")
    for reason in verdict["reasons"]:
        print(f"  - {reason}")
    return (0 if verdict["status"] == "QUALIFIED" else 1), verdict["reasons"]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    c = sub.add_parser("collect")
    c.add_argument("--out", type=Path, required=True)
    c.add_argument(
        "--tier",
        action="append",
        help="diagnostic subset: collect these tiers (repeatable); default: required set",
    )
    c.add_argument("--skip-mutations", action="store_true")
    c.add_argument(
        "--hosted-ci",
        action="store_true",
        help="claim hosted qualification only when GitHub Actions identity checks pass",
    )
    c.add_argument(
        "--local-qualified",
        action="store_true",
        help=(
            "request full qualification from a clean local checkout (all required gates must pass)"
        ),
    )
    c.add_argument(
        "--consume-producers",
        action="store_true",
        help="consume producer artifacts downloaded from registered prerequisite CI jobs",
    )
    v = sub.add_parser("validate")
    v.add_argument("receipt", type=Path)
    v.add_argument(
        "--no-tree-check", action="store_true", help="do not compare against the current tree"
    )
    args = parser.parse_args(argv)
    if args.command == "collect":
        # None is semantically different from an explicit tier subset: the
        # canonical path runs required.json exactly, including its cost order.
        return collect(
            args.out,
            args.tier,
            args.skip_mutations,
            args.hosted_ci,
            args.consume_producers,
            args.local_qualified,
        )
    code, _reasons_already_printed = validate(args.receipt, not args.no_tree_check)
    return code


if __name__ == "__main__":
    raise SystemExit(main())
