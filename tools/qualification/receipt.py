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
`collect` on an immutable CI checkout — never by `validate` on a file someone
hands over. A green `validate` is a necessary condition, not the evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
REQUIRED = REPO / "tools" / "gates" / "required.json"
SCHEMA_VERSION = 1
MUTATION_GATE_ID = "mutants-critical"
# A receipt without these was not produced by `collect`; whatever it says about
# gates, it is not the shape the collector attests.
REQUIRED_RECEIPT_KEYS = ("generated_at", "finished_at", "environment", "source", "gates")

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
        "immutable": False,
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


def run_json(cmd: list[str], receipt_path: Path) -> dict | None:
    proc = subprocess.run(
        cmd + ["--receipt", str(receipt_path)],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=False,
    )
    if receipt_path.exists():
        return json.loads(receipt_path.read_text(encoding="utf-8"))
    return {"error": proc.stderr[-2000:], "exit_code": proc.returncode}


def collect(out: Path, tiers: list[str], skip_mutations: bool) -> int:
    started = datetime.now(timezone.utc).isoformat()
    source = source_identity()
    env = environment()

    # The mutation gate runs ONCE, through its own runner, so its detailed
    # receipt exists; its inventory gate result is derived from that receipt
    # rather than running the (identical) recipe a second time.
    gate_args = [*sum([["--tier", t] for t in tiers], [])]
    if not skip_mutations:
        gate_args += ["--skip", MUTATION_GATE_ID]
    gate_receipt = run_json(
        [sys.executable, "tools/gates/run.py", *gate_args],
        out.with_suffix(".gates.json"),
    )
    mutation_receipt = None
    if not skip_mutations:
        mutation_receipt = run_json(
            [sys.executable, "tools/verification/run_mutations.py"],
            out.with_suffix(".mutations.json"),
        )
        if isinstance(gate_receipt, dict) and "results" in gate_receipt:
            gate_receipt["results"].append(
                {
                    "id": MUTATION_GATE_ID,
                    "recipe": MUTATION_GATE_ID,
                    "status": (
                        "PASS"
                        if isinstance(mutation_receipt, dict)
                        and mutation_receipt.get("problems") == 0
                        and mutation_receipt.get("total", 0) > 0
                        else "FAIL"
                    ),
                    "exit_code": None,
                    "derived_from": str(out.with_suffix(".mutations.json").name),
                }
            )
    msrv = subprocess.run(
        [sys.executable, "tools/consumer-msrv/check.py"],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=False,
    )

    # Source identity is re-derived AFTER the gates: a gate that modified the
    # tree (a mutation runner that failed to restore, a formatter) invalidates
    # the receipt, and this is where that shows.
    source_after = source_identity()

    receipt = {
        "schema_version": SCHEMA_VERSION,
        "generated_at": started,
        "finished_at": datetime.now(timezone.utc).isoformat(),
        "source": source,
        "source_after": {
            "paths_digest": source_after["paths_digest"],
            "dirty": source_after["dirty"],
        },
        "environment": env,
        "gates": gate_receipt,
        "mutations": mutation_receipt,
        "consumer_msrv": {
            "exit_code": msrv.returncode,
            "stdout": msrv.stdout.strip().splitlines()[-3:],
        },
        "limitations": [
            (
                "Local receipt on a mutable working tree: a before/after digest match "
                "does not prove no edit-and-restore happened in between. Final "
                "qualification requires an immutable checkout (see "
                "docs/plans/sep-16-hardening/tickets/VERIFICATION.md)."
            ),
            (
                "Linux-only gates (bench-iai) are SKIPPED_PLATFORM on macOS and leave "
                "the receipt NOT_QUALIFIED here by design."
            ),
        ],
    }
    verdict = evaluate(receipt, source_after)
    receipt["verdict"] = verdict
    out.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(f"receipt: {out}")
    print(f"verdict: {verdict['status']}")
    for reason in verdict["reasons"]:
        print(f"  - {reason}")
    return 0 if verdict["status"] == "QUALIFIED" else 1


def evaluate(receipt: dict, current: dict | None) -> dict:
    """Decide QUALIFIED / NOT_QUALIFIED with every reason listed.

    `current` is the re-derived source identity of the tree the validator runs
    in (`None` skips the comparison, for a receipt inspected away from its
    tree). Every field of it that the receipt also records must agree; the
    receipt's own claims are never taken over the tree in front of us.
    """
    reasons: list[str] = []
    if receipt.get("schema_version") != SCHEMA_VERSION:
        reasons.append(f"schema_version {receipt.get('schema_version')} != {SCHEMA_VERSION}")
    for key in REQUIRED_RECEIPT_KEYS:
        if key not in receipt:
            reasons.append(f"receipt lacks {key!r}: not the shape `collect` produces")
    source = receipt.get("source", {})
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
            reasons.append("current working tree is dirty: cannot qualify an immutable source")
    if receipt.get("source_after", {}).get("paths_digest") not in (
        None,
        source.get("paths_digest"),
    ):
        reasons.append("the working tree changed while the gates ran")
    if source.get("dirty"):
        reasons.append(
            "working tree is dirty: local evidence only, not an immutable-source qualification"
        )

    required = json.loads(REQUIRED.read_text(encoding="utf-8"))["required"]
    gates = receipt.get("gates") or {}
    results = {r["id"]: r for r in gates.get("results", [])}
    for gate_id in required:
        result = results.get(gate_id)
        if result is None:
            reasons.append(f"required gate {gate_id} was not run")
        elif result.get("status") != "PASS":
            reasons.append(f"required gate {gate_id}: {result.get('status')}")

    mutations = receipt.get("mutations")
    if mutations is None:
        reasons.append("mutation gate was not run")
    elif mutations.get("problems", 1) != 0:
        reasons.append(f"mutation gate reported {mutations.get('problems')} problem(s)")
    elif "dirty_tree" in mutations and bool(mutations["dirty_tree"]) != bool(source.get("dirty")):
        # The mutation runner records what it saw; a receipt whose top level
        # disagrees with its own sub-receipt was not produced by one collection.
        reasons.append("the mutation receipt's view of the tree disagrees with the receipt's")

    msrv = receipt.get("consumer_msrv", {})
    if msrv.get("exit_code") != 0:
        reasons.append(f"consumer MSRV check exit {msrv.get('exit_code')} (2 = NOT_RUN)")

    return {"status": "QUALIFIED" if not reasons else "NOT_QUALIFIED", "reasons": reasons}


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
    v = sub.add_parser("validate")
    v.add_argument("receipt", type=Path)
    v.add_argument(
        "--no-tree-check", action="store_true", help="do not compare against the current tree"
    )
    args = parser.parse_args(argv)
    if args.command == "collect":
        # `action="append"` onto a list default would *extend* the default, so
        # `--tier fast` used to collect every tier plus `fast` again.
        return collect(args.out, args.tier or ["fast", "matrix", "proof"], args.skip_mutations)
    code, _reasons_already_printed = validate(args.receipt, not args.no_tree_check)
    return code


if __name__ == "__main__":
    raise SystemExit(main())
