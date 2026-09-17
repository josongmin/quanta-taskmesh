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
"""

from __future__ import annotations

import argparse
import json
import platform
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
INVENTORY = Path(__file__).resolve().parent / "inventory.json"
REQUIRED = Path(__file__).resolve().parent / "required.json"


def host_platform() -> str:
    system = platform.system().lower()
    return {"darwin": "macos", "linux": "linux"}.get(system, system)


def applicable(gate: dict, here: str) -> bool:
    platforms = gate.get("platforms", ["any"])
    return "any" in platforms or here in platforms


# No gate in the inventory legitimately runs this long; a gate that does has
# hung, and a hung gate must become a recorded FAIL, not a runner that never
# returns (which no receipt would ever describe).
GATE_TIMEOUT_SECONDS = 3600


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
    try:
        proc = subprocess.run(
            ["just", gate["recipe"]],
            cwd=REPO,
            capture_output=True,
            text=True,
            check=False,
            timeout=GATE_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as expired:
        duration = time.monotonic() - started
        partial = expired.stdout or b""
        tail = partial.decode("utf-8", "replace") if isinstance(partial, bytes) else str(partial)
        return {
            "id": gate["id"],
            "recipe": gate["recipe"],
            "status": "FAIL",
            "exit_code": None,
            "started_at": started_at,
            "duration_s": round(duration, 3),
            "output_tail": f"TIMEOUT after {GATE_TIMEOUT_SECONDS}s\n" + tail[-1500:],
        }
    duration = time.monotonic() - started
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
        "started_at": started_at,
        "duration_s": round(duration, 3),
        "output_tail": tail,
    }
    if gate.get("status_line"):
        # Kept verbatim: the output tail is bounded and cargo's stderr can push
        # this line out of it, and for coverage/tsan the line *is* the evidence.
        result["status_line"] = final_status_line(gate["status_line"]["marker"], proc.stdout)
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--id", action="append", default=[])
    parser.add_argument("--tier", action="append", default=[])
    parser.add_argument("--all", action="store_true")
    parser.add_argument(
        "--skip",
        action="append",
        default=[],
        help="leave these gate ids out of this run (the caller supplies their result "
        "separately; a skipped required gate still makes this receipt NOT qualified)",
    )
    parser.add_argument("--receipt", type=Path)
    args = parser.parse_args(argv)

    inventory = json.loads(INVENTORY.read_text(encoding="utf-8"))
    required = set(json.loads(REQUIRED.read_text(encoding="utf-8"))["required"])
    gates = inventory["gates"]
    by_id = {g["id"]: g for g in gates}

    selected: list[dict] = []
    if args.all:
        selected = list(gates)
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
    results = []
    for gate in selected:
        if not applicable(gate, here):
            results.append(
                {
                    "id": gate["id"],
                    "recipe": gate["recipe"],
                    "status": "SKIPPED_PLATFORM",
                    "exit_code": None,
                    "platform": here,
                }
            )
            print(f"{gate['id']:<20} SKIPPED_PLATFORM ({here} not in {gate.get('platforms')})")
            continue
        print(f"{gate['id']:<20} running …", flush=True)
        result = run_gate(gate)
        results.append(result)
        print(f"{gate['id']:<20} {result['status']} ({result['duration_s']}s)")

    ran_ids = {r["id"] for r in results}
    missing_required = sorted(required - ran_ids)
    not_passed_required = sorted(
        r["id"] for r in results if r["id"] in required and r["status"] != "PASS"
    )
    qualified = not missing_required and not not_passed_required

    receipt = {
        "schema_version": 1,
        "platform": here,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "qualified": qualified,
        "required_not_run": missing_required,
        "required_not_passed": not_passed_required,
        "results": results,
    }
    if args.receipt:
        args.receipt.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
        print(f"receipt: {args.receipt}")

    print()
    if qualified:
        print("gates: QUALIFIED — every required gate ran here and passed")
        return 0
    print("gates: NOT_QUALIFIED", file=sys.stderr)
    if missing_required:
        print(f"  required gates not run in this invocation: {missing_required}", file=sys.stderr)
    if not_passed_required:
        print(f"  required gates not passed: {not_passed_required}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
