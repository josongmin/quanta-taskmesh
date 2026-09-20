#!/usr/bin/env python3
"""Mutation gate: prove the regression suite can actually fail.

A green test suite says the code passes the tests. It does not say the tests
would notice if the code were wrong — and for a hardening effort that is the
only question worth asking. Each entry in `mutations.json` reintroduces exactly
one of the defects this work fixed, and names the test that must reject it.

A mutation is **killed** only when the named test fails for the named reason.
Per `docs/plans/sep-16-hardening/tickets/VERIFICATION.md` none of the following
counts as a kill:

- a compile error (the mutant was never executed),
- a different test failing (the suite noticed something else),
- zero tests collected or selected,
- a timeout, unless the mutation is explicitly a hang fixture.

One entry is a deliberate no-op control: it must leave the suite GREEN. Without
it, a runner that simply broke the build would report a perfect score.

Two runners are supported, chosen per entry by `runner`:

- `cargo` (default): `package` + `test_target` (`lib` or a `tests/` target),
  optionally `profile: "release"` when a debug assertion would otherwise fire
  before the named test gets to observe the defect, plus `cargo_args` and
  `env` for targets that need a feature and a `--cfg` (the shuttle models).
- `pytest`: `test_target` is a pytest node id; the mutated file is one of the
  Python tools, whose own tests are the proof.

Every non-control entry must name `expect_message`: a mutation "killed" by any
assertion in the named test is not yet evidence that the test checks the
property the mutation breaks.

Usage:
    python3 tools/verification/run_mutations.py            # run every mutation
    python3 tools/verification/run_mutations.py --id <id>  # run one
    python3 tools/verification/run_mutations.py --list
    python3 tools/verification/run_mutations.py --receipt out.json
    python3 tools/verification/run_mutations.py --require-clean   # CI: refuse a dirty tree
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import time
from collections import Counter
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
INVENTORY = Path(__file__).resolve().parent / "mutations.json"
TEST_TIMEOUT_SECONDS = 900


@dataclass
class Outcome:
    mutation_id: str
    finding: str
    status: str
    detail: str
    duration_s: float
    expected_test: str = ""
    observed_failures: list[str] = field(default_factory=list)


RUNNERS = ("cargo", "pytest")
PROFILES = ("debug", "release")


def validate_entry(mutation: dict) -> None:
    """Refuse an inventory entry that cannot prove what it claims."""
    mutation_id = mutation.get("id") or "<missing id>"
    for key in ("id", "finding", "file", "find", "replace", "test_target"):
        if not mutation.get(key):
            raise SystemExit(f"mutation {mutation_id}: missing required field {key!r}")
    if mutation["find"] == mutation["replace"]:
        raise SystemExit(f"mutation {mutation_id}: find and replace are identical")
    runner = mutation.get("runner", "cargo")
    if runner not in RUNNERS:
        raise SystemExit(f"mutation {mutation_id}: unknown runner {runner!r}")
    if runner == "cargo" and not mutation.get("package"):
        raise SystemExit(f"mutation {mutation_id}: cargo runner needs a package")
    if mutation.get("profile", "debug") not in PROFILES:
        raise SystemExit(f"mutation {mutation_id}: unknown profile {mutation.get('profile')!r}")
    env = mutation.get("env", {})
    if not isinstance(env, dict) or not all(
        isinstance(k, str) and isinstance(v, str) for k, v in env.items()
    ):
        raise SystemExit(f"mutation {mutation_id}: env must be a string→string map")
    cargo_args = mutation.get("cargo_args", [])
    if not isinstance(cargo_args, list) or not all(isinstance(a, str) for a in cargo_args):
        raise SystemExit(f"mutation {mutation_id}: cargo_args must be a list of strings")
    if runner == "pytest" and (env or cargo_args):
        raise SystemExit(f"mutation {mutation_id}: env/cargo_args apply to the cargo runner only")
    control = bool(mutation.get("expect_no_failure"))
    if control:
        if mutation["finding"] != "control":
            raise SystemExit(
                f"mutation {mutation_id}: a control must be labelled finding=control, "
                f"not {mutation['finding']!r}"
            )
        return
    if mutation["finding"] == "control":
        raise SystemExit(f"mutation {mutation_id}: finding=control without expect_no_failure")
    if not mutation.get("expect_failing_test"):
        raise SystemExit(f"mutation {mutation_id}: missing expect_failing_test")
    if not mutation.get("expect_message"):
        raise SystemExit(
            f"mutation {mutation_id}: missing expect_message — a kill by an unnamed "
            "assertion does not show the test checks the property this mutation breaks"
        )


def load_inventory(path: Path) -> list[dict]:
    data = json.loads(path.read_text(encoding="utf-8"))
    mutations = data.get("mutations", [])
    if not mutations:
        raise SystemExit("mutation inventory is empty; refusing to report a perfect score")
    seen: set[str] = set()
    for mutation in mutations:
        validate_entry(mutation)
        mutation_id = mutation["id"]
        if mutation_id in seen:
            raise SystemExit(f"duplicate mutation id: {mutation_id}")
        seen.add(mutation_id)
    if not any(m.get("expect_no_failure") for m in mutations):
        raise SystemExit("mutation inventory has no control entry")
    return mutations


def test_command(mutation: dict) -> list[str]:
    runner = mutation.get("runner", "cargo")
    if runner == "pytest":
        return [
            sys.executable,
            "-m",
            "pytest",
            "-q",
            "-p",
            "no:cacheprovider",
            "-rfE",
            mutation["test_target"],
        ]
    test_target = mutation["test_target"]
    # `lib` selects the crate's unit tests; anything else is an integration
    # test target under `tests/`.
    selector = ["--lib"] if test_target == "lib" else ["--test", test_target]
    profile = ["--release"] if mutation.get("profile") == "release" else []
    # Extra cargo arguments (e.g. `--features shuttle` for a model-check target
    # that is `#![cfg(shuttle)]`); paired with `env` (RUSTFLAGS) in `run_tests`.
    extra = list(mutation.get("cargo_args", []))
    return [
        "cargo",
        "test",
        "-p",
        mutation["package"],
        *profile,
        *extra,
        *selector,
        "--",
        "--test-threads",
        "4",
    ]


def run_tests(mutation: dict) -> subprocess.CompletedProcess[str]:
    # No bytecode cache for the pytest runner: Python validates a `.pyc` by
    # mtime-seconds + size, so an equal-size edit within the same second as the
    # previous compile could run the *unmutated* module.
    env = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1", **mutation.get("env", {})}
    return subprocess.run(
        test_command(mutation),
        capture_output=True,
        text=True,
        check=False,
        cwd=REPO,
        timeout=TEST_TIMEOUT_SECONDS,
        env=env,
    )


PYTEST_SUMMARY = re.compile(r"\b\d+ (?:passed|failed|errors?)\b.* in \d+(?:\.\d+)?s")
PYTEST_COUNT = re.compile(r"\b(\d+) (passed|failed|errors?)\b")


def failing_tests(stdout: str, runner: str = "cargo") -> list[str]:
    names = []
    for line in stdout.splitlines():
        stripped = line.strip()
        if runner == "pytest":
            # `-rfE` summary lines: `FAILED path::test - message` and
            # `ERROR path::test - message` (a fixture error is a failure of the
            # named test to run, never a survivor).
            for prefix in ("FAILED ", "ERROR "):
                if stripped.startswith(prefix):
                    node = stripped[len(prefix) :].split(" - ", 1)[0]
                    names.append(node.rsplit("::", 1)[-1])
            continue
        if stripped.startswith("test ") and stripped.endswith(" ... FAILED"):
            names.append(stripped[len("test ") : -len(" ... FAILED")])
    return names


def collected_tests(stdout: str, runner: str = "cargo") -> int:
    total = 0
    for line in stdout.splitlines():
        stripped = line.strip()
        if runner == "pytest":
            # pytest -q prints e.g. `3 passed, 1 failed in 0.12s` — or, past a
            # minute, `195 passed in 80.09s (0:01:20)`.
            if PYTEST_SUMMARY.search(stripped):
                for count, _kind in PYTEST_COUNT.findall(stripped):
                    total += int(count)
            continue
        if stripped.startswith("running ") and stripped.endswith(("test", "tests")):
            parts = stripped.split()
            if len(parts) >= 2 and parts[1].isdigit():
                total += int(parts[1])
    return total


def compile_failed(proc: subprocess.CompletedProcess[str], runner: str = "cargo") -> bool:
    combined = proc.stdout + proc.stderr
    if runner == "pytest":
        # A mutant that breaks import/collection was never executed either.
        return "SyntaxError" in combined or "error during collection" in combined.lower()
    return "could not compile" in combined or "error[E" in combined


def evaluate(mutation: dict, proc: subprocess.CompletedProcess[str]) -> tuple[str, str, list[str]]:
    """Classify one mutant run. Returns (status, detail, failing test names)."""
    control = bool(mutation.get("expect_no_failure"))
    runner = mutation.get("runner", "cargo")
    combined = proc.stdout + proc.stderr

    if compile_failed(proc, runner):
        # The mutant never ran, so nothing was proved about the tests.
        return (
            "INVALID_COMPILE_ERROR",
            "the mutated source did not compile; the mutant was never executed",
            [],
        )

    collected = collected_tests(proc.stdout, runner)
    if collected == 0:
        return ("INVALID_NO_TESTS", "no tests were collected for the target", [])

    failures = failing_tests(proc.stdout, runner)

    if control:
        if failures:
            return (
                "CONTROL_BROKEN",
                f"a behaviour-preserving mutation still failed: {failures}",
                failures,
            )
        return ("CONTROL_GREEN", f"suite stayed green across {collected} tests", [])

    expected = mutation["expect_failing_test"]
    if not failures:
        return (
            "SURVIVED",
            f"no test rejected this mutation ({collected} tests ran)",
            [],
        )
    if expected not in failures:
        return (
            "WRONG_TEST",
            f"expected {expected!r} to fail; instead {failures} failed",
            failures,
        )

    needle = mutation.get("expect_message", "")
    if needle and needle not in failure_output(combined, expected, runner):
        return (
            "WRONG_REASON",
            f"{expected!r} failed, but not for the expected reason "
            f"({needle!r} absent from that test's own output)",
            failures,
        )
    return ("KILLED", f"{expected!r} failed as expected", failures)


def failure_output(combined: str, test_name: str, runner: str) -> str:
    """The part of the run output that belongs to `test_name`'s failure.

    Searching the whole output would let *another* failing test supply the
    expected message — several messages are shared helper strings within a
    file — and credit the named test with a reason it never gave.
    """
    if runner == "pytest":
        # The `___ name ___` section plus the `FAILED/ERROR …::name - msg` line.
        short = test_name.rsplit("::", 1)[-1]
        pieces: list[str] = []
        section = re.search(
            rf"^_{{2,}} .*\b{re.escape(short)}\b.* _{{2,}}$\n(.*?)(?=^_{{2,}} |^={{3,}} |\Z)",
            combined,
            re.MULTILINE | re.DOTALL,
        )
        if section:
            pieces.append(section.group(1))
        pieces.extend(
            line
            for line in combined.splitlines()
            if (line.startswith("FAILED ") or line.startswith("ERROR ")) and f"::{short}" in line
        )
        return "\n".join(pieces)
    # libtest: `---- <name> stdout ----` up to the next section or the summary.
    section = re.search(
        rf"^---- {re.escape(test_name)} stdout ----$\n(.*?)(?=^---- |^failures:$|^test result:)",
        combined,
        re.MULTILINE | re.DOTALL,
    )
    return section.group(1) if section else ""


def apply_mutation(mutation: dict) -> tuple[Path, str]:
    target = REPO / mutation["file"]
    original = target.read_text(encoding="utf-8")
    find = mutation["find"]
    occurrences = original.count(find)
    if occurrences != 1:
        raise SystemExit(
            f"mutation {mutation['id']}: anchor matched {occurrences} times in "
            f"{mutation['file']} (expected exactly 1). The mutation inventory has "
            "drifted from the source it mutates."
        )
    target.write_text(original.replace(find, mutation["replace"], 1), encoding="utf-8")
    return target, original


def run_one(mutation: dict, verbose: bool) -> Outcome:
    started = time.monotonic()
    target, original = apply_mutation(mutation)
    try:
        try:
            proc = run_tests(mutation)
        except subprocess.TimeoutExpired:
            return Outcome(
                mutation_id=mutation["id"],
                finding=mutation["finding"],
                status="INVALID_TIMEOUT",
                detail=f"tests exceeded {TEST_TIMEOUT_SECONDS}s",
                duration_s=time.monotonic() - started,
                expected_test=mutation.get("expect_failing_test", ""),
            )
        status, detail, failures = evaluate(mutation, proc)
        if verbose and status not in ("KILLED", "CONTROL_GREEN"):
            print(proc.stdout[-4000:], file=sys.stderr)
        return Outcome(
            mutation_id=mutation["id"],
            finding=mutation["finding"],
            status=status,
            detail=detail,
            duration_s=time.monotonic() - started,
            expected_test=mutation.get("expect_failing_test", ""),
            observed_failures=failures,
        )
    finally:
        # Always restore, including on an unexpected exception: leaving a mutated
        # tree behind would poison every later run.
        target.write_text(original, encoding="utf-8")


def dirty_paths() -> list[str]:
    """Tracked or untracked paths with uncommitted changes, per git."""
    proc = subprocess.run(
        ["git", "status", "--porcelain", "--untracked-files=all"],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise SystemExit(f"could not inspect the working tree with git: {proc.stderr.strip()}")
    return [line[3:] for line in proc.stdout.splitlines() if line.strip()]


def verify_tree(require_clean: bool) -> list[str]:
    """Report the tree's dirtiness; refuse to run on a dirty tree when asked.

    Locally a dirty tree is the normal state while a fix is being written, and
    the runner restores every mutated file from the copy it read. In CI the
    inventory's prerequisite is a clean checkout, and that is enforced rather
    than assumed: a mutation score over an unknown diff proves nothing about
    the commit under review.
    """
    dirty = dirty_paths()
    if require_clean and dirty:
        raise SystemExit(
            "refusing to run the mutation gate on a dirty tree (--require-clean): "
            + ", ".join(dirty[:8])
            + (" …" if len(dirty) > 8 else "")
        )
    return dirty


GOOD_STATUSES = {"KILLED", "CONTROL_GREEN"}


def receipt_scope(mutations: list[dict]) -> dict:
    runners = Counter(mutation.get("runner", "cargo") for mutation in mutations)
    sources: Counter[str] = Counter()
    for mutation in mutations:
        path = mutation["file"]
        runner = mutation.get("runner", "cargo")
        if runner == "pytest":
            sources["tooling_faults"] += 1
        elif re.match(r"^crates/[^/]+/src/", path):
            sources["rust_crate_src"] += 1
        elif re.match(r"^crates/[^/]+/tests/", path):
            sources["rust_crate_tests"] += 1
        else:
            sources["other"] += 1
    return {
        "runner_counts": dict(sorted(runners.items())),
        "source_counts": dict(sorted(sources.items())),
        "control_entries": sum(1 for mutation in mutations if mutation.get("expect_no_failure")),
    }


def build_receipt(
    mutations: list[dict],
    outcomes: list[Outcome],
    dirty: list[str],
    *,
    selection: str = "full",
) -> dict:
    bad = [outcome for outcome in outcomes if outcome.status not in GOOD_STATUSES]
    status_counts = Counter(outcome.status for outcome in outcomes)
    definitions = {mutation["id"]: mutation for mutation in mutations}
    return {
        "schema_version": 2,
        "kind": "curated-single-edit-inventory",
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "inventory": str(INVENTORY.relative_to(REPO)),
        "inventory_digest": hashlib.sha256(INVENTORY.read_bytes()).hexdigest(),
        "inventory_total": len(load_inventory(INVENTORY)),
        "selection": selection,
        "scope": receipt_scope(mutations),
        "dirty_tree": bool(dirty),
        "dirty_paths": dirty,
        "total": len(outcomes),
        "problems": len(bad),
        "status_counts": dict(sorted(status_counts.items())),
        "results": [
            {
                "id": outcome.mutation_id,
                "finding": outcome.finding,
                "runner": definitions[outcome.mutation_id].get("runner", "cargo"),
                "file": definitions[outcome.mutation_id]["file"],
                "control": bool(definitions[outcome.mutation_id].get("expect_no_failure")),
                "status": outcome.status,
                "detail": outcome.detail,
                "expected_test": outcome.expected_test,
                "observed_failures": outcome.observed_failures,
                "duration_s": round(outcome.duration_s, 3),
            }
            for outcome in outcomes
        ],
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--id", action="append", help="run only these mutation ids")
    parser.add_argument("--list", action="store_true", help="list the inventory and exit")
    parser.add_argument("--receipt", type=Path, help="write a JSON receipt here")
    parser.add_argument("--verbose", action="store_true", help="dump output for bad outcomes")
    parser.add_argument(
        "--require-clean",
        action="store_true",
        help="refuse to run unless the git tree has no uncommitted changes (CI)",
    )
    args = parser.parse_args(argv)

    mutations = load_inventory(INVENTORY)
    if args.list:
        for mutation in mutations:
            kind = "control" if mutation.get("expect_no_failure") else mutation["finding"]
            print(f"{mutation['id']:<48} {kind:<12} {mutation['file']}")
        return 0

    selection = "full"
    if args.id:
        selection = "subset"
        wanted = set(args.id)
        unknown = wanted - {m["id"] for m in mutations}
        if unknown:
            raise SystemExit(f"unknown mutation id(s): {sorted(unknown)}")
        mutations = [m for m in mutations if m["id"] in wanted]

    # Every anchor is checked before anything runs: a formatter pass that moved
    # one anchor would otherwise surface as the 28th entry aborting a
    # three-minute run, with the 27 results before it thrown away.
    drifted = [
        f"{m['id']} ({(REPO / m['file']).read_text(encoding='utf-8').count(m['find'])} matches)"
        for m in mutations
        if (REPO / m["file"]).read_text(encoding="utf-8").count(m["find"]) != 1
    ]
    if drifted:
        raise SystemExit(
            "mutation anchors have drifted from the source they mutate (expected exactly "
            "one match each): " + ", ".join(drifted)
        )

    dirty = verify_tree(args.require_clean)
    if dirty:
        print(
            f"note: running on a dirty tree ({len(dirty)} path(s) modified); "
            "the receipt records this",
            file=sys.stderr,
        )

    outcomes: list[Outcome] = []
    for index, mutation in enumerate(mutations, start=1):
        print(
            f"[{index}/{len(mutations)}] {mutation['id']} ({mutation['finding']}) … ",
            end="",
            flush=True,
        )
        outcome = run_one(mutation, args.verbose)
        outcomes.append(outcome)
        print(f"{outcome.status} ({outcome.duration_s:.1f}s)")
        if outcome.status not in GOOD_STATUSES:
            print(f"      {outcome.detail}", file=sys.stderr)

    bad = [o for o in outcomes if o.status not in GOOD_STATUSES]
    print()
    counts = Counter(outcome.status for outcome in outcomes)
    print(
        f"curated mutations: {len(outcomes)}  killed: {counts['KILLED']}  "
        f"controls green: {counts['CONTROL_GREEN']}  problems: {len(bad)}"
    )

    if args.receipt:
        args.receipt.write_text(
            json.dumps(build_receipt(mutations, outcomes, dirty, selection=selection), indent=2)
            + "\n",
            encoding="utf-8",
        )
        print(f"receipt: {args.receipt}")

    if bad:
        print("\nunkilled or invalid mutations:", file=sys.stderr)
        for outcome in bad:
            print(
                f"  - {outcome.mutation_id}: {outcome.status} — {outcome.detail}", file=sys.stderr
            )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
