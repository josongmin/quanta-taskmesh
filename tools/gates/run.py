#!/usr/bin/env python3
"""Run gates from the inventory and write a receipt (H16-018 / H16-022).

Each gate is invoked as `just <recipe>` — the same command a developer runs —
and its exit code, duration, and platform are recorded. A required gate that
was not run, was skipped for a platform reason, or reported a conditional
non-result (exit 2 = NOT_RUN by convention) is recorded as such and makes the
receipt NOT_QUALIFIED. Nothing here turns "did not run" into "passed".

A gate with `status_line: {marker, require}` in the inventory self-reports on
stdout (`taskmesh-<gate> status=… …`). Its *final* marker line is the verdict:
it is kept verbatim in the result (`status_line`) — the output tail is bounded
and cannot be relied on to contain it — and the run is PASS only if that line
carries the required token. Exit 0 without it is NOT_RUN.

Usage:
    python3 tools/gates/run.py --tier fast              # e.g. the fast gate
    python3 tools/gates/run.py --id clippy --id test    # named gates
    python3 tools/gates/run.py --all --receipt r.json   # everything applicable
    python3 tools/gates/run.py --required --allow-platform-skips  # clean host scope
    python3 tools/gates/run.py --required --allow-dirty-source --keep-going  # diagnostic sweep
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
INVENTORY = Path(__file__).resolve().parent / "inventory.json"
REQUIRED = Path(__file__).resolve().parent / "required.json"
if str(REPO) not in sys.path:
    sys.path.insert(0, str(REPO))

from tools.gates.parallel_policy import PARALLEL_GATE_LIMIT, PARALLEL_GROUP_MEMBERS  # noqa: E402
from tools.process_supervisor import (  # noqa: E402
    SupervisedCommand,
    run_process,
    run_process_batch,
)
from tools.qualification.receipt import source_identity  # noqa: E402


def host_platform() -> str:
    system = platform.system().lower()
    return {"darwin": "macos", "linux": "linux"}.get(system, system)


def applicable(gate: dict, here: str) -> bool:
    platforms = gate.get("platforms", ["any"])
    return "any" in platforms or here in platforms


def source_stability_problems(before: dict, after: dict) -> list[str]:
    """Require a clean, unchanged exact source for any qualified local receipt."""
    problems: list[str] = []
    if before.get("dirty") is not False:
        problems.append("source was dirty before local verification")
    if after.get("dirty") is not False:
        problems.append("source was dirty after local verification")
    for field in ("head", "tree", "paths_digest"):
        if before.get(field) != after.get(field):
            problems.append(f"source {field} changed while local verification ran")
    return problems


def platform_scope_qualification(
    required: set[str],
    results: list[dict],
    platform_conditional: dict[str, str],
    here: str,
) -> tuple[bool, list[str]]:
    """Return whether every applicable required gate passed on this host.

    A platform-scoped pass is deliberately distinct from ``qualified``: the
    latter still requires every required gate to be PASS. The only tolerated
    non-PASS result is a ``SKIPPED_PLATFORM`` for the exact inventory-declared
    platform condition. This lets a Mac prove its complete local surface
    without relabelling the Linux-only IAI gate as a pass.
    """
    by_id = {result["id"]: result for result in results}
    excluded: list[str] = []
    for gate_id in sorted(required):
        result = by_id.get(gate_id)
        if result is None:
            return False, excluded
        if (
            result.get("status") == "PASS"
            and type(result.get("exit_code")) is int
            and result["exit_code"] == 0
        ):
            continue
        if (
            result.get("status") == "SKIPPED_PLATFORM"
            and platform_conditional.get(gate_id)
            and platform_conditional[gate_id] != here
        ):
            excluded.append(gate_id)
            continue
        return False, excluded
    return True, excluded


def result_record_problems(results: list[dict]) -> list[str]:
    """Validate the status/exit contract before trusting a saved receipt."""
    problems: list[str] = []
    for result in results:
        gate_id = result.get("id", "<missing>")
        status = result.get("status")
        exit_code = result.get("exit_code")
        if status == "PASS":
            if type(exit_code) is not int or exit_code != 0:
                problems.append(f"local receipt gate {gate_id} PASS requires integer exit_code 0")
        elif status == "SKIPPED_PLATFORM":
            if exit_code is not None:
                problems.append(
                    f"local receipt gate {gate_id} SKIPPED_PLATFORM requires null exit_code"
                )
        elif status == "NOT_RUN":
            if exit_code is not None and (type(exit_code) is not int or exit_code != 2):
                problems.append(
                    f"local receipt gate {gate_id} NOT_RUN exit_code must be null or integer 2"
                )
        elif status == "FAIL":
            if exit_code is not None and (type(exit_code) is not int or exit_code == 0):
                problems.append(
                    f"local receipt gate {gate_id} FAIL exit_code must be null or a nonzero integer"
                )
        else:
            problems.append(f"local receipt gate {gate_id} has unknown status {status!r}")
    return problems


def local_receipt_problems(
    value: object,
    current: dict,
    required: set[str],
    platform_conditional: dict[str, str],
    here: str,
    expected_head: str | None = None,
) -> list[str]:
    """Re-derive whether a saved platform receipt admits the current source."""
    if not isinstance(value, dict):
        return ["local receipt is not an object"]
    problems: list[str] = []
    if value.get("schema_version") != 2:
        problems.append("local receipt schema_version is not 2")
    if value.get("platform") != here:
        problems.append("local receipt platform differs from this host")
    source = value.get("source")
    if not isinstance(source, dict):
        source = {}
        problems.append("local receipt source identity is missing")
    for field in ("head", "tree", "paths_digest", "dirty"):
        if source.get(field) != current.get(field):
            problems.append(f"local receipt source {field} differs from current source")
    if source.get("dirty") is not False or current.get("dirty") is not False:
        problems.append("local receipt and current source must both be clean")
    if expected_head is not None and source.get("head") != expected_head:
        problems.append("local receipt HEAD differs from the ref being pushed")
    source_after = value.get("source_after")
    expected_after = {
        field: source.get(field) for field in ("head", "tree", "paths_digest", "dirty")
    }
    if source_after != expected_after:
        problems.append("local receipt source changed while verification ran")
    if value.get("source_problems") != []:
        problems.append("local receipt reports source stability problems")
    results = value.get("results")
    if not isinstance(results, list):
        results = []
        problems.append("local receipt results are missing")
    valid_results = [
        result
        for result in results
        if isinstance(result, dict) and isinstance(result.get("id"), str)
    ]
    ids = [result["id"] for result in valid_results]
    if len(valid_results) != len(results) or len(ids) != len(set(ids)):
        problems.append("local receipt result ids are malformed or duplicated")
    problems.extend(result_record_problems(valid_results))
    recomputed, excluded = platform_scope_qualification(
        required, valid_results, platform_conditional, here
    )
    scope = value.get("platform_scope")
    if not isinstance(scope, dict):
        scope = {}
    if (
        not recomputed
        or scope.get("qualified") is not True
        or scope.get("excluded_required_gates") != excluded
    ):
        problems.append("local receipt platform qualification does not match its results")
    return problems


# No gate in the inventory legitimately runs this long; a gate that does has
# hung, and a hung gate must become a recorded FAIL, not a runner that never
# returns (which no receipt would ever describe).
GATE_TIMEOUT_SECONDS = 3600
MAX_GATE_TIMEOUT_SECONDS = 21600


def execute_gate_process(
    argv: list[str], timeout_seconds: int
) -> tuple[subprocess.CompletedProcess, bool]:
    process = run_process(argv, cwd=REPO, env=os.environ.copy(), timeout_seconds=timeout_seconds)
    completed = subprocess.CompletedProcess(
        args=argv,
        returncode=process.returncode,
        stdout=process.stdout,
        stderr=process.stderr,
    )
    completed.interrupted_by_signal = process.interrupted_by_signal
    return completed, process.timed_out


def status_line_qualifies(gate: dict, stdout: str) -> bool:
    """For a gate that self-reports (`status_line` in the inventory: a marker
    and a token that must appear on the marker's line), exit 0 alone is not
    PASS. A missing marker line is treated as not qualified too: a recipe that
    stopped printing its verdict has not proved anything."""
    spec = gate.get("status_line")
    if not spec:
        return True
    verdict = final_status_line(spec["marker"], stdout)
    return verdict is not None and spec["require"] in verdict.split()


def final_status_line(marker: str, stdout: str) -> str | None:
    """The *last* `<marker> …` line on stdout: a recipe that reports per-surface
    lines before its summary (consumer-msrv) ends with the verdict, and an
    earlier line must not stand in for it."""
    lines = [line for line in stdout.splitlines() if line.startswith(marker + " ")]
    return lines[-1] if lines else None


def run_gate(gate: dict) -> dict:
    started = time.monotonic()
    started_at = datetime.now(timezone.utc).isoformat()
    timeout_seconds = gate.get("timeout_seconds", GATE_TIMEOUT_SECONDS)
    try:
        proc, timed_out = execute_gate_process(["just", gate["recipe"]], timeout_seconds)
    except OSError as exc:
        return {
            "id": gate["id"],
            "recipe": gate["recipe"],
            "status": "FAIL",
            "exit_code": None,
            "signal": None,
            "timed_out": False,
            "timeout_seconds": timeout_seconds,
            "started_at": started_at,
            "duration_s": round(time.monotonic() - started, 3),
            "output_tail": f"gate command could not start: {exc}",
        }
    return gate_process_result(gate, proc, timed_out, started_at, time.monotonic() - started)


def run_gate_batch(gates: list[dict]) -> list[dict]:
    """Launch an approved wave under one main-thread process supervisor."""
    if not 1 <= len(gates) <= PARALLEL_GATE_LIMIT:
        raise ValueError("parallel gate batch exceeds the approved worker limit")
    started_at = datetime.now(timezone.utc).isoformat()
    commands = [
        SupervisedCommand(
            argv=["just", gate["recipe"]],
            cwd=REPO,
            env=os.environ.copy(),
            timeout_seconds=gate.get("timeout_seconds", GATE_TIMEOUT_SECONDS),
        )
        for gate in gates
    ]
    try:
        completed = run_process_batch(commands)
    except OSError as exc:
        return [
            {
                "id": gate["id"],
                "recipe": gate["recipe"],
                "status": "FAIL",
                "exit_code": None,
                "timed_out": False,
                "output_tail": f"gate command could not start: {exc}",
            }
            for gate in gates
        ]
    results = []
    for gate, batch_result in zip(gates, completed):
        process = batch_result.process
        proc = subprocess.CompletedProcess(
            args=["just", gate["recipe"]],
            returncode=process.returncode,
            stdout=process.stdout,
            stderr=process.stderr,
        )
        proc.interrupted_by_signal = process.interrupted_by_signal
        results.append(
            gate_process_result(
                gate,
                proc,
                process.timed_out,
                started_at,
                batch_result.duration_s,
            )
        )
    return results


def gate_process_result(
    gate: dict,
    proc: subprocess.CompletedProcess,
    timed_out: bool,
    started_at: str,
    duration: float,
) -> dict:
    """Apply the same fail-closed status contract to serial and batch processes."""
    timeout_seconds = gate.get("timeout_seconds", GATE_TIMEOUT_SECONDS)
    if timed_out:
        return {
            "id": gate["id"],
            "recipe": gate["recipe"],
            "status": "FAIL",
            "exit_code": proc.returncode,
            "signal": -proc.returncode
            if proc.returncode is not None and proc.returncode < 0
            else None,
            "interrupted_by_signal": getattr(proc, "interrupted_by_signal", None),
            "timed_out": True,
            "timeout_seconds": timeout_seconds,
            "started_at": started_at,
            "duration_s": round(duration, 3),
            "output_tail": f"TIMEOUT after {timeout_seconds}s\n"
            + (proc.stdout + proc.stderr)[-1500:],
        }
    if proc.returncode == 0 and not status_line_qualifies(gate, proc.stdout):
        # The recipe ran and exited 0 but its own machine-readable line says
        # it did not reach the qualifying state (e.g. bench-iai recorded a
        # baseline instead of comparing against one). That is not a pass.
        status = "NOT_RUN"
    elif proc.returncode == 0:
        status = "PASS"
    elif proc.returncode == 2 and gate.get("conditional"):
        # Conditional gates use exit 2 for "could not run here"; that is a
        # distinct outcome from failure, and from success. (A shell or argparse
        # usage error also exits 2; it lands here as NOT_RUN, which is still
        # NOT_QUALIFIED — never PASS.)
        status = "NOT_RUN"
    else:
        status = "FAIL"
    tail = (proc.stdout + proc.stderr)[-2000:]
    result = {
        "id": gate["id"],
        "recipe": gate["recipe"],
        "status": status,
        "exit_code": proc.returncode,
        "signal": -proc.returncode
        if proc.returncode is not None and proc.returncode < 0
        else None,
        "timed_out": False,
        "interrupted_by_signal": getattr(proc, "interrupted_by_signal", None),
        "timeout_seconds": timeout_seconds,
        "started_at": started_at,
        "duration_s": round(duration, 3),
        "output_tail": tail,
    }
    if gate.get("status_line"):
        # Kept verbatim: the output tail is bounded and cargo's stderr can push
        # this line out of it, and for coverage/tsan the line *is* the evidence.
        result["status_line"] = final_status_line(gate["status_line"]["marker"], proc.stdout)
    return result


def run_selected_gates(
    selected: list[dict],
    here: str,
    deadline: float | None,
    keep_going: bool,
    runner=None,
    preflight_blocker: str | None = None,
) -> list[dict]:
    """Run selected gates in order, stopping useful work after the first non-pass.

    Fail-fast does not shrink the receipt denominator. Every later applicable
    gate gets an explicit NOT_RUN result tied to the gate that fixed the final
    verdict. Platform exclusions remain SKIPPED_PLATFORM because they are known
    without executing a command. ``runner`` is injectable for control-flow tests.
    """
    if runner is None:
        runner = run_gate
    results: list[dict] = []
    blocked_by = preflight_blocker

    def record_unrun(gate: dict, blocker: str) -> dict:
        return {
            "id": gate["id"],
            "recipe": gate["recipe"],
            "status": "NOT_RUN",
            "exit_code": None,
            "platform": here,
            "blocked_by": blocker,
            "output_tail": f"NOT RUN: blocked by earlier non-pass gate {blocker}",
        }

    def run_with_deadline(gate: dict) -> dict:
        effective_gate = gate
        if deadline is not None:
            remaining = int(deadline - time.monotonic())
            if remaining <= 0:
                return {
                    "id": gate["id"],
                    "recipe": gate["recipe"],
                    "status": "FAIL",
                    "exit_code": None,
                    "signal": None,
                    "timed_out": True,
                    "timeout_seconds": 0,
                    "output_tail": "QUALIFICATION DEADLINE EXHAUSTED before gate start",
                }
            effective_gate = {
                **gate,
                "timeout_seconds": min(
                    gate.get("timeout_seconds", GATE_TIMEOUT_SECONDS), remaining
                ),
            }
        return runner(effective_gate)

    index = 0
    while index < len(selected):
        gate = selected[index]
        group = gate.get("parallel_group")
        members = PARALLEL_GROUP_MEMBERS.get(group) if isinstance(group, str) else None
        if group is not None and (
            members is None or gate["id"] not in members or gate.get("platforms") != ["any"]
        ):
            result = {
                "id": gate["id"],
                "recipe": gate["recipe"],
                "status": "FAIL",
                "exit_code": None,
                "duration_s": 0.0,
                "output_tail": f"unapproved parallel gate policy: {group!r}/{gate['id']}",
            }
            results.append(result)
            print(f"{gate['id']:<20} FAIL (unapproved parallel group)")
            blocked_by = gate["id"]
            index += 1
            continue
        if not applicable(gate, here):
            result = {
                "id": gate["id"],
                "recipe": gate["recipe"],
                "status": "SKIPPED_PLATFORM",
                "exit_code": None,
                "platform": here,
            }
            print(f"{gate['id']:<20} SKIPPED_PLATFORM ({here} not in {gate.get('platforms')})")
            results.append(result)
            index += 1
            continue
        if blocked_by is not None:
            results.append(record_unrun(gate, blocked_by))
            print(f"{gate['id']:<20} NOT_RUN (blocked by {blocked_by})")
            index += 1
            continue

        if group:
            assert members is not None
            # Only explicitly inventoried, contiguous groups run concurrently.
            # Bound fan-out and preserve receipt/output order independent of
            # completion order. If a batch fails, later batches never start.
            wave: list[dict] = []
            while (
                index < len(selected)
                and selected[index].get("parallel_group") == group
                and selected[index]["id"] in members
                and selected[index].get("platforms") == ["any"]
            ):
                candidate = selected[index]
                index += 1
                if applicable(candidate, here):
                    wave.append(candidate)
                else:
                    results.append(
                        {
                            "id": candidate["id"],
                            "recipe": candidate["recipe"],
                            "status": "SKIPPED_PLATFORM",
                            "exit_code": None,
                            "platform": here,
                        }
                    )
            for offset in range(0, len(wave), PARALLEL_GATE_LIMIT):
                batch = wave[offset : offset + PARALLEL_GATE_LIMIT]
                for item in batch:
                    print(f"{item['id']:<20} running …", flush=True)
                by_id: dict[str, dict] = {}
                if runner is run_gate:
                    remaining = int(deadline - time.monotonic()) if deadline is not None else None
                    if remaining is not None and remaining <= 0:
                        by_id[batch[0]["id"]] = run_with_deadline(batch[0])
                        for item in batch[1:]:
                            by_id[item["id"]] = record_unrun(item, batch[0]["id"])
                    else:
                        effective_batch = [
                            {
                                **item,
                                "timeout_seconds": min(
                                    item.get("timeout_seconds", GATE_TIMEOUT_SECONDS), remaining
                                ),
                            }
                            if remaining is not None
                            else item
                            for item in batch
                        ]
                        try:
                            batch_results = run_gate_batch(effective_batch)
                            by_id = {result["id"]: result for result in batch_results}
                        except Exception as exc:
                            by_id = {
                                item["id"]: {
                                    "id": item["id"],
                                    "recipe": item["recipe"],
                                    "status": "FAIL",
                                    "exit_code": None,
                                    "timed_out": False,
                                    "output_tail": (
                                        f"gate runner raised {type(exc).__name__}: {exc}"
                                    ),
                                }
                                for item in batch
                            }
                else:
                    # Injectable fake runners are thread-safe control-flow tests;
                    # real subprocesses use the main-thread batch supervisor.
                    with ThreadPoolExecutor(max_workers=len(batch)) as pool:
                        futures = {pool.submit(run_with_deadline, item): item for item in batch}
                        for future in as_completed(futures):
                            item = futures[future]
                            try:
                                by_id[item["id"]] = future.result()
                            except Exception as exc:
                                by_id[item["id"]] = {
                                    "id": item["id"],
                                    "recipe": item["recipe"],
                                    "status": "FAIL",
                                    "exit_code": None,
                                    "timed_out": False,
                                    "output_tail": (
                                        f"gate runner raised {type(exc).__name__}: {exc}"
                                    ),
                                }
                batch_results = [by_id[item["id"]] for item in batch]
                deadline_blocker: str | None = None
                for item, result in zip(batch, batch_results):
                    if result.get("timeout_seconds") == 0:
                        if deadline_blocker is None:
                            deadline_blocker = item["id"]
                            blocked_by = deadline_blocker
                        else:
                            result = record_unrun(item, deadline_blocker)
                    results.append(result)
                    print(
                        f"{item['id']:<20} {result['status']} "
                        f"({result.get('duration_s', 0)}s)"
                    )
                    if result.get("interrupted_by_signal") is not None and blocked_by is None:
                        blocked_by = f"signal-{result['interrupted_by_signal']}"
                    elif result.get("timeout_seconds") == 0 and blocked_by is None:
                        blocked_by = item["id"]
                    elif result["status"] != "PASS" and not keep_going and blocked_by is None:
                        blocked_by = item["id"]
                if blocked_by is not None and not keep_going:
                    break
            continue

        index += 1
        print(f"{gate['id']:<20} running …", flush=True)
        result = run_with_deadline(gate)
        results.append(result)
        print(f"{gate['id']:<20} {result['status']} ({result.get('duration_s', 0)}s)")
        if result.get("interrupted_by_signal") is not None:
            blocked_by = f"signal-{result['interrupted_by_signal']}"
        elif result.get("timeout_seconds") == 0:
            # A spent global budget cannot be bypassed by --keep-going.
            blocked_by = gate["id"]
        elif result["status"] != "PASS" and not keep_going:
            blocked_by = gate["id"]
    return results


def source_preflight_blocker(source: dict, *, require_clean_source: bool) -> str | None:
    """Return the qualification preflight blocker."""
    if require_clean_source and source.get("dirty") is not False:
        return "dirty-source-preflight"
    return None


def clean_source_required(
    *,
    select_required: bool,
    select_all: bool,
    explicit_requirement: bool,
    allow_dirty_source: bool,
) -> bool:
    """Full-denominator runs reject dirty source unless diagnostics opt out."""
    return not allow_dirty_source and (select_required or select_all or explicit_requirement)


def summarize_required(required: set[str], results: list[dict]) -> tuple[list[str], list[str]]:
    """Return honest required NOT_RUN and non-PASS sets for a receipt."""
    ran_ids = {result["id"] for result in results}
    not_run = (required - ran_ids) | {
        result["id"]
        for result in results
        if result["id"] in required and result["status"] == "NOT_RUN"
    }
    not_passed = {
        result["id"]
        for result in results
        if result["id"] in required and result["status"] != "PASS"
    }
    return sorted(not_run), sorted(not_passed)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--id", action="append", default=[])
    parser.add_argument("--tier", action="append", default=[])
    parser.add_argument("--all", action="store_true")
    parser.add_argument(
        "--required",
        action="store_true",
        help="run exactly the ordinary required gate ids from required.json",
    )
    parser.add_argument(
        "--allow-platform-skips",
        action="store_true",
        help="exit zero only when every applicable required gate passes; retain platform skips",
    )
    parser.add_argument(
        "--validate-receipt",
        type=Path,
        help="validate a saved local receipt against the current exact source",
    )
    parser.add_argument(
        "--expected-head",
        help="also require the receipt to match this pushed commit SHA",
    )
    parser.add_argument(
        "--skip",
        action="append",
        default=[],
        help="leave these gate ids out of this run (the caller supplies their result "
        "separately; a skipped required gate still makes this receipt NOT qualified)",
    )
    parser.add_argument("--receipt", type=Path)
    parser.add_argument(
        "--deadline-seconds",
        type=int,
        help="bound the complete sequential run; the active process is terminated at the deadline",
    )
    parser.add_argument(
        "--keep-going",
        action="store_true",
        help="run later gates after a non-pass for diagnostic sweeps (default: fail fast)",
    )
    parser.add_argument(
        "--require-clean-source",
        action="store_true",
        help="record gates as NOT_RUN without starting them when the source is dirty "
        "(the default for --required and --all)",
    )
    parser.add_argument(
        "--allow-dirty-source",
        action="store_true",
        help="diagnostic override: allow --required/--all gates to run on dirty source; "
        "the receipt still cannot qualify",
    )
    args = parser.parse_args(argv)
    if args.require_clean_source and args.allow_dirty_source:
        parser.error("--require-clean-source conflicts with --allow-dirty-source")
    if args.deadline_seconds is not None and args.deadline_seconds <= 0:
        raise SystemExit("--deadline-seconds must be positive")
    inventory = json.loads(INVENTORY.read_text(encoding="utf-8"))
    required_document = json.loads(REQUIRED.read_text(encoding="utf-8"))
    required_ids = required_document["required"]
    required = set(required_ids)
    if args.validate_receipt is not None:
        try:
            value = json.loads(args.validate_receipt.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            print(f"local receipt INVALID: {exc}", file=sys.stderr)
            return 1
        problems = local_receipt_problems(
            value,
            source_identity(),
            required,
            required_document.get("platform_conditional", {}),
            host_platform(),
            args.expected_head,
        )
        if problems:
            print("local receipt INVALID:", file=sys.stderr)
            for problem in problems:
                print(f"  - {problem}", file=sys.stderr)
            return 1
        print(
            f"local receipt VALID: {args.validate_receipt} "
            f"head={value['source']['head']} platform={value['platform']}"
        )
        return 0
    if args.expected_head is not None:
        raise SystemExit("--expected-head requires --validate-receipt")
    selectors = int(args.all) + int(args.required) + int(bool(args.tier)) + int(bool(args.id))
    if selectors != 1:
        raise SystemExit("select exactly one of --all, --required, --tier, or --id")

    source = source_identity()
    gates = inventory["gates"]
    by_id = {g["id"]: g for g in gates}

    selected: list[dict] = []
    if args.all:
        selected = list(gates)
    elif args.required:
        # required.json owns both membership and execution order. Cheap/static
        # blockers precede expensive proof producers so fail-fast saves work.
        selected = [by_id[gate_id] for gate_id in required_ids]
    else:
        for tier in args.tier:
            selected.extend(g for g in gates if g["tier"] == tier)
        for gate_id in args.id:
            if gate_id not in by_id:
                raise SystemExit(f"unknown gate id: {gate_id}")
            selected.append(by_id[gate_id])
    if not selected:
        raise SystemExit("no gates selected (use --all, --tier, or --id)")
    # De-duplicate, preserve order.
    seen: set[str] = set()
    selected = [g for g in selected if not (g["id"] in seen or seen.add(g["id"]))]
    unknown_skips = sorted(set(args.skip) - set(by_id))
    if unknown_skips:
        raise SystemExit(f"unknown gate id(s) in --skip: {unknown_skips}")
    selected = [g for g in selected if g["id"] not in set(args.skip)]

    here = host_platform()
    deadline = (
        time.monotonic() + args.deadline_seconds if args.deadline_seconds is not None else None
    )
    preflight_blocker = source_preflight_blocker(
        source,
        require_clean_source=clean_source_required(
            select_required=args.required,
            select_all=args.all,
            explicit_requirement=args.require_clean_source,
            allow_dirty_source=args.allow_dirty_source,
        ),
    )
    if preflight_blocker is not None:
        print(
            "source-preflight     NOT_RUN (qualification requires a clean source)",
            file=sys.stderr,
        )
    results = run_selected_gates(
        selected,
        here,
        deadline,
        args.keep_going,
        preflight_blocker=preflight_blocker,
    )
    interrupted = next(
        (
            result["interrupted_by_signal"]
            for result in results
            if result.get("interrupted_by_signal") is not None
        ),
        None,
    )
    missing_required, not_passed_required = summarize_required(required, results)
    source_after = source_identity()
    source_problems = source_stability_problems(source, source_after)
    qualified = (
        not missing_required
        and not not_passed_required
        and not source_problems
        and interrupted is None
    )
    platform_conditional = required_document.get("platform_conditional", {})
    gates_scoped_qualified, excluded_platform_gates = platform_scope_qualification(
        required, results, platform_conditional, here
    )
    scoped_qualified = gates_scoped_qualified and not source_problems and interrupted is None

    receipt = {
        "schema_version": 2,
        "platform": here,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "source": source,
        "source_after": {
            field: source_after.get(field) for field in ("head", "tree", "paths_digest", "dirty")
        },
        "source_problems": source_problems,
        "qualified": qualified,
        "required_not_run": missing_required,
        "required_not_passed": not_passed_required,
        "platform_scope": {
            "qualified": scoped_qualified,
            "excluded_required_gates": excluded_platform_gates,
        },
        "results": results,
    }
    if args.receipt:
        args.receipt.parent.mkdir(parents=True, exist_ok=True)
        args.receipt.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
        print(f"receipt: {args.receipt}")

    print()
    if qualified:
        print("gates: QUALIFIED — every required gate ran here and passed")
        return 0
    if args.allow_platform_skips and scoped_qualified:
        print(
            f"gates: PLATFORM_QUALIFIED ({here}) — every applicable required gate passed; "
            f"excluded platform gate(s): {excluded_platform_gates}"
        )
        return 0
    if interrupted is not None:
        print(f"gates: INTERRUPTED by signal {interrupted}", file=sys.stderr)
        return 128 + interrupted
    print("gates: NOT_QUALIFIED", file=sys.stderr)
    if missing_required:
        print(f"  required gates not run in this invocation: {missing_required}", file=sys.stderr)
    if not_passed_required:
        print(f"  required gates not passed: {not_passed_required}", file=sys.stderr)
    for problem in source_problems:
        print(f"  {problem}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
