#!/usr/bin/env python3
"""Qualification receipts (H16-022).

A receipt binds a set of gate results to the *exact* source they ran against.
"Exact" includes the working tree, not just `HEAD`: a dirty overlay on the same
commit is a different source, and a receipt that names only the commit could be
reused for code that was never tested. So the source identity is a digest over
every tracked and untracked file's path, content, mode, and symlink target.

Two commands:

    collect   run the gates (via tools/gates/run.py) plus the mutation gate and
              the consumer-MSRV check, and write a receipt for this tree
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
`collect --hosted-ci` in the isolated GitHub Actions qualification job — never
by `validate` on a file someone hands over. The collector verifies the Actions
environment, workspace and exact `GITHUB_SHA`; a green `validate` is a necessary
condition, not the evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import subprocess
import sys
import time
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from typing import NamedTuple

REPO = Path(__file__).resolve().parents[2]
REQUIRED = REPO / "tools" / "gates" / "required.json"
SCHEMA_VERSION = 2
GATE_SCHEMA_VERSION = 2
MUTATION_SCHEMA_VERSION = 2
MUTATION_GATE_ID = "mutants-critical"
MUTATION_INVENTORY = REPO / "tools" / "verification" / "mutations.json"
ATTESTATION_CHECK_NAMES = {
    "github_actions",
    "ci",
    "head_matches_github_sha",
    "workspace_matches_repo",
    "source_clean",
}
# A receipt without these was not produced by `collect`; whatever it says about
# gates, it is not the shape the collector attests.
REQUIRED_RECEIPT_KEYS = (
    "generated_at",
    "finished_at",
    "environment",
    "source",
    "source_after",
    "attestation",
    "gates",
    "mutations",
    "generated_mutation_sweep",
)

# Excluded from the source digest: build output and tool caches that do not
# affect what is compiled or tested. Everything else — including untracked
# files — is in.
DIGEST_EXCLUDES = {
    "target",
    ".git",
    ".venv",
    "__pycache__",
    ".pytest_cache",
    ".ruff_cache",
    ".mypy_cache",
    "tools/consumer-msrv/target",
    # Receipts describe a source revision; they are not part of it. Writing one
    # while collecting must not invalidate the collection.
    "docs/plans/sep-16-hardening/receipts",
}


def _git(*args: str, root: Path = REPO) -> str:
    return subprocess.run(
        ["git", *args], cwd=root, capture_output=True, text=True, check=True
    ).stdout.strip()


def _excluded(rel: Path) -> bool:
    parts = rel.parts
    for i in range(1, len(parts) + 1):
        if "/".join(parts[:i]) in DIGEST_EXCLUDES or parts[i - 1] in DIGEST_EXCLUDES:
            return True
    return False


def tree_paths(root: Path) -> list[Path]:
    """Tracked plus untracked-but-not-ignored paths, as git sees them.

    `rglob` would also digest gitignored files (`.DS_Store`, a stray
    `bench-results/`), so two identical checkouts could disagree on their
    identity for reasons that are not source. Git's own view is the authority
    on what the source *is*; a deleted-but-tracked path is skipped below.
    """
    listing = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=root,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return sorted({Path(p) for p in listing.split("\0") if p})


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
    dirty = bool(status.strip())

    hasher = hashlib.sha256()
    count = 0
    for rel in tree_paths(root):
        path = root / rel
        if _excluded(rel):
            continue
        if path.is_symlink():
            hasher.update(f"L {rel.as_posix()} -> {os.readlink(path)}\n".encode())
            count += 1
            continue
        if not path.is_file():
            # A tracked path deleted in the working tree: `status` records the
            # deletion, so the identity already differs from HEAD.
            continue
        mode = path.stat().st_mode & 0o777
        content = hashlib.sha256(path.read_bytes()).hexdigest()
        hasher.update(f"F {rel.as_posix()} {mode:o} {content}\n".encode())
        count += 1
    return {
        "head": head,
        "tree": tree,
        "dirty": dirty,
        "isolated_checkout": False,
        "paths_digest": hasher.hexdigest(),
        "paths_counted": count,
        "status": status.splitlines(),
    }


def environment() -> dict:
    rustc = subprocess.run(["rustc", "-Vv"], capture_output=True, text=True, check=False).stdout
    lock = REPO / "Cargo.lock"
    return {
        "toolchain": rustc.strip().splitlines()[0] if rustc else "unknown",
        "rustc_verbose": rustc.strip(),
        "lock_digest": hashlib.sha256(lock.read_bytes()).hexdigest() if lock.exists() else None,
        "os_target": f"{platform.system().lower()}-{platform.machine()}",
        "runner": platform.node(),
        "python": sys.version.split()[0],
    }


def collection_attestation(source: dict, hosted_ci: bool) -> dict:
    checks = {
        "github_actions": os.environ.get("GITHUB_ACTIONS") == "true",
        "ci": os.environ.get("CI") == "true",
        "head_matches_github_sha": os.environ.get("GITHUB_SHA") == source.get("head"),
        "workspace_matches_repo": (
            bool(os.environ.get("GITHUB_WORKSPACE"))
            and Path(os.environ["GITHUB_WORKSPACE"]).resolve() == REPO.resolve()
        ),
        "source_clean": not source.get("dirty", True),
    }
    eligible = hosted_ci and all(checks.values())
    return {
        "kind": "github-actions" if hosted_ci else "local",
        "hosted_qualification_eligible": eligible,
        "head": source.get("head"),
        "checks": checks,
    }


class JsonRun(NamedTuple):
    payload: dict
    exit_code: int
    started_at: str
    duration_s: float


def run_json(cmd: list[str], receipt_path: Path) -> JsonRun:
    # Never mistake a sidecar from an earlier collection for this process's
    # output when the process dies before writing anything.
    receipt_path.unlink(missing_ok=True)
    started_at = datetime.now(timezone.utc).isoformat()
    started = time.monotonic()
    proc = subprocess.run(
        cmd + ["--receipt", str(receipt_path)],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=False,
    )
    duration_s = round(time.monotonic() - started, 3)
    if receipt_path.exists():
        try:
            payload = json.loads(receipt_path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as exc:
            payload = {"error": f"runner wrote invalid JSON: {exc}"}
    else:
        payload = {
            "error": "runner did not write its receipt",
            "stderr_tail": proc.stderr[-2000:],
        }
    return JsonRun(payload, proc.returncode, started_at, duration_s)


def required_gate_ids() -> list[str]:
    return json.loads(REQUIRED.read_text(encoding="utf-8"))["required"]


def summarize_gate_results(results: list[dict]) -> dict:
    required = set(required_gate_ids())
    ran = {result.get("id") for result in results if isinstance(result, dict)}
    not_run = sorted(required - ran)
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


def canonical_digest(payload: dict) -> str:
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def mutation_scope_from_results(results: list[dict]) -> dict:
    runners = Counter(result.get("runner") for result in results)
    sources: Counter[str] = Counter()
    for result in results:
        runner = result.get("runner")
        path = result.get("file", "")
        if runner == "pytest":
            sources["tooling_faults"] += 1
        elif isinstance(path, str) and path.startswith("crates/") and "/src/" in path:
            sources["rust_crate_src"] += 1
        elif isinstance(path, str) and path.startswith("crates/") and "/tests/" in path:
            sources["rust_crate_tests"] += 1
        else:
            sources["other"] += 1
    return {
        "runner_counts": dict(sorted(runners.items(), key=lambda item: str(item[0]))),
        "source_counts": dict(sorted(sources.items())),
        "control_entries": sum(bool(result.get("control")) for result in results),
    }


def mutation_receipt_problems(mutations: object) -> list[str]:
    if not isinstance(mutations, dict):
        return ["mutation receipt is missing or is not an object"]
    problems: list[str] = []
    if mutations.get("schema_version") != MUTATION_SCHEMA_VERSION:
        problems.append(
            f"mutation schema_version {mutations.get('schema_version')} != "
            f"{MUTATION_SCHEMA_VERSION}"
        )
    if mutations.get("kind") != "curated-single-edit-inventory":
        problems.append("mutation receipt does not identify curated-single-edit-inventory")
    inventory_digest = hashlib.sha256(MUTATION_INVENTORY.read_bytes()).hexdigest()
    inventory = json.loads(MUTATION_INVENTORY.read_text(encoding="utf-8"))["mutations"]
    inventory_ids = [mutation["id"] for mutation in inventory]
    if mutations.get("inventory_digest") != inventory_digest:
        problems.append("mutation inventory digest does not match the current source")
    if mutations.get("inventory_total") != len(inventory_ids):
        problems.append("mutation inventory_total does not match the current inventory")
    if mutations.get("selection") != "full":
        problems.append("mutation receipt is not a full-inventory run")
    results = mutations.get("results")
    if not isinstance(results, list):
        return [*problems, "mutation receipt results is not a list"]
    ids = [result.get("id") for result in results if isinstance(result, dict)]
    if len(ids) != len(results) or any(not isinstance(mutation_id, str) for mutation_id in ids):
        problems.append("mutation receipt has a result without a string id")
    if len(ids) != len(set(ids)):
        problems.append("mutation receipt contains duplicate result ids")
    if ids != inventory_ids:
        problems.append("mutation result ids do not match the full inventory in order")
    for result in results:
        if not isinstance(result, dict):
            continue
        if result.get("runner") not in {"cargo", "pytest"}:
            problems.append(f"mutation {result.get('id')} has an invalid runner")
        if not isinstance(result.get("file"), str) or not result.get("file"):
            problems.append(f"mutation {result.get('id')} has no source file")
        if result.get("status") == "CONTROL_GREEN" and not result.get("control"):
            problems.append(f"mutation {result.get('id')} is an undeclared control")
        if result.get("control") and result.get("status") != "CONTROL_GREEN":
            problems.append(f"mutation {result.get('id')} control did not stay green")
    if mutations.get("total") != len(results):
        problems.append(
            f"mutation total {mutations.get('total')} does not match {len(results)} results"
        )
    status_counts = Counter(result.get("status") for result in results if isinstance(result, dict))
    expected_counts = dict(sorted(status_counts.items(), key=lambda item: str(item[0])))
    if mutations.get("status_counts") != expected_counts:
        problems.append("mutation status_counts does not match results")
    expected_scope = mutation_scope_from_results(
        [result for result in results if isinstance(result, dict)]
    )
    if mutations.get("scope") != expected_scope:
        problems.append("mutation scope does not match results")
    bad_count = sum(
        count
        for status, count in status_counts.items()
        if status not in {"KILLED", "CONTROL_GREEN"}
    )
    if mutations.get("problems") != bad_count:
        problems.append(
            f"mutation problems {mutations.get('problems')} does not match {bad_count} bad results"
        )
    if not results:
        problems.append("mutation receipt contains no results")
    return problems


def generated_mutation_disclosure() -> dict:
    """Describe the generated-mutation evidence this collector does not run.

    The required mutation gate is the curated single-edit inventory above. A
    cargo-mutants campaign has a different denominator and must never be
    inferred from that gate's KILLED count.
    """
    return {
        "tool": "cargo-mutants",
        "status": "NOT_RUN",
        "included_in_qualification": False,
        "reason": "generated mutation sweep is a separate campaign, not a qualification gate",
    }


def generated_mutation_disclosure_problems(disclosure: object) -> list[str]:
    if not isinstance(disclosure, dict):
        return ["generated mutation sweep disclosure is missing or is not an object"]
    expected = generated_mutation_disclosure()
    return [
        f"generated mutation sweep disclosure {key}={disclosure.get(key)!r} "
        f"does not match {expected_value!r}"
        for key, expected_value in expected.items()
        if disclosure.get(key) != expected_value
    ]


def derived_mutation_result(run: JsonRun) -> dict:
    issues = mutation_receipt_problems(run.payload)
    passed = run.exit_code == 0 and not issues
    return {
        "id": MUTATION_GATE_ID,
        "recipe": MUTATION_GATE_ID,
        "status": "PASS" if passed else "FAIL",
        "exit_code": run.exit_code,
        "started_at": run.started_at,
        "duration_s": run.duration_s,
        "evidence_kind": "derived",
        "derived_from": "mutations",
        "evidence_digest": canonical_digest(run.payload),
    }


def finalize_gate_receipt(gates: dict) -> dict:
    results = gates.get("results")
    if not isinstance(results, list):
        results = []
        gates["results"] = results
    gates["schema_version"] = GATE_SCHEMA_VERSION
    gates.update(summarize_gate_results(results))
    gates["finalized_at"] = datetime.now(timezone.utc).isoformat()
    return gates


def collect(out: Path, tiers: list[str], skip_mutations: bool, hosted_ci: bool = False) -> int:
    started = datetime.now(timezone.utc).isoformat()
    source = source_identity()
    attestation = collection_attestation(source, hosted_ci)
    source["isolated_checkout"] = attestation["hosted_qualification_eligible"]
    env = environment()

    # The mutation gate runs ONCE, through its own runner, so its detailed
    # receipt exists; its inventory gate result is derived from that receipt
    # rather than running the (identical) recipe a second time.
    gate_args = [*sum([["--tier", t] for t in tiers], [])]
    # The detailed mutation runner owns this gate. Always omit it from the
    # generic gate process; --skip-mutations must genuinely leave it NOT_RUN.
    gate_args += ["--skip", MUTATION_GATE_ID]
    gate_run = run_json(
        [sys.executable, "tools/gates/run.py", *gate_args],
        out.with_suffix(".gates.json"),
    )
    gate_receipt = gate_run.payload
    mutation_receipt = None
    if not skip_mutations:
        mutation_run = run_json(
            [sys.executable, "tools/verification/run_mutations.py"],
            out.with_suffix(".mutations.json"),
        )
        mutation_receipt = mutation_run.payload
        if isinstance(gate_receipt, dict) and "results" in gate_receipt:
            gate_receipt["results"].append(derived_mutation_result(mutation_run))
    else:
        out.with_suffix(".mutations.json").unlink(missing_ok=True)
    if not isinstance(gate_receipt, dict):
        gate_receipt = {"error": "gate runner did not produce an object", "results": []}
    finalize_gate_receipt(gate_receipt)
    # The sidecar is an authoritative summary of the same result set embedded
    # in the combined receipt, not the pre-derivation intermediate document.
    out.with_suffix(".gates.json").write_text(
        json.dumps(gate_receipt, indent=2) + "\n", encoding="utf-8"
    )

    # Source identity is re-derived AFTER the gates: a gate that modified the
    # tree (a mutation runner that failed to restore, a formatter) invalidates
    # the receipt, and this is where that shows.
    source_after = source_identity()

    limitations = []
    if not attestation["hosted_qualification_eligible"]:
        limitations.append(
            "Local collection: exact-source evidence only, never a hosted qualification. "
            "The receipt of record must be collected by the GitHub Actions qualification job."
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
            "paths_digest": source_after["paths_digest"],
            "dirty": source_after["dirty"],
        },
        "attestation": attestation,
        "environment": env,
        "gates": gate_receipt,
        "mutations": mutation_receipt,
        "generated_mutation_sweep": generated_mutation_disclosure(),
        "limitations": limitations,
    }
    verdict = evaluate(receipt, source_after)
    receipt["verdict"] = verdict
    out.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(f"receipt: {out}")
    print(f"verdict: {verdict['status']}")
    for reason in verdict["reasons"]:
        print(f"  - {reason}")
    return 0 if verdict["status"] == "QUALIFIED" else 1


def evaluate(receipt: object, current: dict | None) -> dict:
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
    attestation = receipt.get("attestation", {})
    if not isinstance(attestation, dict):
        reasons.append("receipt attestation is not an object")
        attestation = {}
    if attestation.get("kind") != "github-actions" or not attestation.get(
        "hosted_qualification_eligible"
    ):
        reasons.append("receipt was not collected by the hosted qualification job")
    checks = attestation.get("checks")
    if (
        not isinstance(checks, dict)
        or set(checks) != ATTESTATION_CHECK_NAMES
        or not all(value is True for value in checks.values())
    ):
        reasons.append("hosted qualification attestation checks are incomplete or failed")
    if attestation.get("head") != source.get("head"):
        reasons.append("attestation HEAD does not match receipt source HEAD")
    if source.get("isolated_checkout") is not True:
        reasons.append("receipt source is not marked as an isolated hosted checkout")
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
            reasons.append("current working tree is dirty: cannot qualify an isolated source")
    source_after = receipt.get("source_after", {})
    if not isinstance(source_after, dict):
        reasons.append("receipt source_after is not an object")
        source_after = {}
    if source_after.get("paths_digest") not in (
        None,
        source.get("paths_digest"),
    ):
        reasons.append("the working tree changed while the gates ran")
    if source_after.get("dirty") != source.get("dirty"):
        reasons.append("the working tree dirtiness changed while the gates ran")
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
    valid_gate_results = [result for result in gate_results if isinstance(result, dict)]
    gate_ids = [result.get("id") for result in valid_gate_results]
    if len(valid_gate_results) != len(gate_results) or any(
        not isinstance(gate_id, str) for gate_id in gate_ids
    ):
        reasons.append("gate receipt has a result without a string id")
    if len(gate_ids) != len(set(gate_ids)):
        reasons.append("gate receipt contains duplicate result ids")
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

    mutations = receipt.get("mutations")
    mutation_issues = mutation_receipt_problems(mutations)
    reasons.extend(mutation_issues)
    reasons.extend(generated_mutation_disclosure_problems(receipt.get("generated_mutation_sweep")))
    if (
        isinstance(mutations, dict)
        and "dirty_tree" in mutations
        and bool(mutations["dirty_tree"]) != bool(source.get("dirty"))
    ):
        # The mutation runner records what it saw; a receipt whose top level
        # disagrees with its own sub-receipt was not produced by one collection.
        reasons.append("the mutation receipt's view of the tree disagrees with the receipt's")
    mutation_gate = results.get(MUTATION_GATE_ID)
    if mutation_gate is not None:
        expected_status = "PASS" if not mutation_issues else "FAIL"
        if mutation_gate.get("evidence_kind") != "derived":
            reasons.append("mutation gate result is not marked as derived evidence")
        if mutation_gate.get("derived_from") != "mutations":
            reasons.append(
                "mutation gate result does not reference the embedded mutations evidence"
            )
        if isinstance(mutations, dict) and mutation_gate.get("evidence_digest") != canonical_digest(
            mutations
        ):
            reasons.append("mutation gate evidence digest does not match the embedded receipt")
        if mutation_gate.get("status") != expected_status:
            reasons.append(
                f"mutation gate status {mutation_gate.get('status')} does not match "
                f"embedded evidence {expected_status}"
            )
        if expected_status == "PASS" and mutation_gate.get("exit_code") != 0:
            reasons.append("passing mutation gate does not record exit_code 0")

    status = "QUALIFIED" if not reasons else "NOT_QUALIFIED"
    computed = {"status": status, "reasons": list(reasons)}
    stored = receipt.get("verdict")
    if stored is not None and stored != computed:
        reasons.append("stored verdict does not match the validator's recomputed verdict")
        status = "NOT_QUALIFIED"
    return {"status": status, "reasons": reasons}


def validate(path: Path, check_tree: bool) -> tuple[int, list[str]]:
    """Exit code plus the reasons, so a caller (or test) can see *why*."""
    try:
        receipt = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        print(f"FAIL: cannot read receipt: {exc}", file=sys.stderr)
        return 1, [f"cannot read receipt: {exc}"]
    current = source_identity() if check_tree else None
    verdict = evaluate(receipt, current)
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
        help="collect only these tiers (repeatable); default: fast, matrix, proof",
    )
    c.add_argument("--skip-mutations", action="store_true")
    c.add_argument(
        "--hosted-ci",
        action="store_true",
        help="claim hosted qualification only when GitHub Actions identity checks pass",
    )
    v = sub.add_parser("validate")
    v.add_argument("receipt", type=Path)
    v.add_argument(
        "--no-tree-check", action="store_true", help="do not compare against the current tree"
    )
    args = parser.parse_args(argv)
    if args.command == "collect":
        # `action="append"` onto a list default would *extend* the default, so
        # `--tier fast` used to collect every tier plus `fast` again.
        return collect(
            args.out,
            args.tier or ["fast", "matrix", "proof"],
            args.skip_mutations,
            args.hosted_ci,
        )
    code, _reasons_already_printed = validate(args.receipt, not args.no_tree_check)
    return code


if __name__ == "__main__":
    raise SystemExit(main())
