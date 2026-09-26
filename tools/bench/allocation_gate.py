#!/usr/bin/env python3
"""Deterministic allocation gate (ADR 9000 / P2 allocation track, P8).

Reads the structured line the `alloc_probe` producer prints and compares it to
the threshold in `tools/bench/perf-gate.json`. Runs anywhere — no valgrind.

# Why a parser and not `sed | awk`

The previous gate extracted `[0-9.][0-9.]*` and compared it with `awk v + 0`.
`...` matched that pattern and compared as `0`; `3.0.0` compared as `3`. A
corrupted metric line was a passing gate. Here every field is parsed against a
strict grammar, the marker must appear exactly once, and the threshold is
validated with the same rules as the measurement — parse failure is a failure,
never a default.

Usage:
    python3 tools/bench/allocation_gate.py             # run the producer
    python3 tools/bench/allocation_gate.py --input F   # parse captured output
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from decimal import Decimal
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CONFIG = Path(__file__).resolve().parent / "perf-gate.json"

# A complete, finite, non-negative decimal: digits, optionally one point and
# more digits. Anchored on both ends so trailing junk cannot ride along.
DECIMAL = re.compile(r"^(?:0|[1-9][0-9]*)(?:\.[0-9]+)?$")
INTEGER = re.compile(r"^(?:0|[1-9][0-9]*)$")


class GateError(Exception):
    """A reason the gate cannot pass. Always reported, never defaulted."""


def load_config() -> dict:
    try:
        data = json.loads(CONFIG.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise GateError(f"cannot read {CONFIG}: {exc}") from exc
    section = data.get("allocation_gate")
    if not isinstance(section, dict):
        raise GateError(f"{CONFIG} has no allocation_gate section")
    return section


def parse_decimal(field: str, raw: str) -> Decimal:
    if not DECIMAL.fullmatch(raw):
        raise GateError(f"{field}={raw!r} is not a complete non-negative decimal")
    return Decimal(raw)


def parse_counter_check(raw: str) -> tuple[int, int]:
    """`<counted>/<expected>`, both plain non-negative integers."""
    parts = raw.split("/")
    if len(parts) != 2:
        raise GateError(f"counter_check must be <counted>/<expected>, got {raw!r}")
    return parse_integer("counter_check.counted", parts[0]), parse_integer(
        "counter_check.expected", parts[1]
    )


def parse_integer(field: str, raw: str) -> int:
    if not INTEGER.fullmatch(raw):
        raise GateError(f"{field}={raw!r} is not a non-negative integer")
    return int(raw)


def parse_metric_line(output: str, config: dict) -> dict:
    marker = config["producer_marker"]
    lines = [line for line in output.splitlines() if line.startswith(marker + " ")]
    if len(lines) != 1:
        raise GateError(
            f"expected exactly one {marker!r} line in producer output, found {len(lines)}"
        )
    fields: dict[str, str] = {}
    for token in lines[0].split()[1:]:
        if "=" not in token:
            raise GateError(f"malformed field {token!r} on the metric line")
        key, raw = token.split("=", 1)
        if key in fields:
            raise GateError(f"duplicate field {key!r} on the metric line")
        fields[key] = raw
    missing = [f for f in config["required_fields"] if f not in fields]
    if missing:
        raise GateError(f"metric line is missing fields: {missing}")
    extra = [f for f in fields if f not in config["required_fields"]]
    if extra:
        raise GateError(f"metric line carries unknown fields: {extra}")

    schema = parse_integer("schema", fields["schema"])
    if schema != int(config["producer_schema"]):
        raise GateError(
            f"producer schema {schema} does not match the gate's expected "
            f"{config['producer_schema']}; the measurement definition changed and the "
            "threshold must be re-approved"
        )
    attempted = parse_integer("attempted", fields["attempted"])
    completed = parse_integer("completed", fields["completed"])
    total_allocations = parse_integer("total_allocations", fields["total_allocations"])
    final_inflight = parse_integer("final_inflight", fields["final_inflight"])
    allocs_per_op = parse_decimal("allocs_per_op", fields["allocs_per_op"])
    counted, expected = parse_counter_check(fields["counter_check"])
    if counted != expected or expected == 0:
        raise GateError(
            f"producer counter self-check saw {counted} of {expected} known allocations; "
            "the instrument cannot be trusted with the measurement"
        )

    if attempted == 0:
        raise GateError("producer attempted zero operations; nothing was measured")
    if completed != attempted:
        raise GateError(
            f"producer completed {completed} of {attempted} operations; "
            "a partial run is not a measurement"
        )
    exact_per_op = Decimal(total_allocations) / completed
    if abs(allocs_per_op - exact_per_op) > Decimal("0.0005"):
        raise GateError("reported allocs_per_op disagrees with total_allocations")
    if final_inflight != 0:
        raise GateError(
            f"producer left {final_inflight} permits inflight; the ledger did not drain"
        )
    return {
        "schema": schema,
        "attempted": attempted,
        "completed": completed,
        "total_allocations": total_allocations,
        "allocs_per_op": allocs_per_op,
        "final_inflight": final_inflight,
        "counter_check": (counted, expected),
    }


PRODUCER = [
    "cargo",
    "run",
    "--locked",
    "-q",
    "-p",
    "taskmesh-bench",
    "--example",
    "alloc_probe",
    "--release",
]


def run_producer() -> str:
    proc = subprocess.run(
        PRODUCER,
        capture_output=True,
        text=True,
        check=False,
        cwd=REPO,
    )
    if proc.returncode != 0:
        # Producer failures propagate with their own code; they are not parse
        # failures and are not turned into one.
        sys.stderr.write(proc.stderr)
        raise SystemExit(proc.returncode)
    sys.stderr.write(proc.stderr)
    return proc.stdout


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--input", type=Path, help="parse this captured producer output instead of running it"
    )
    args = parser.parse_args(argv)

    try:
        config = load_config()
        threshold = parse_decimal("max_allocs_per_op", str(config["max_allocs_per_op"]))
        output = args.input.read_text(encoding="utf-8") if args.input else run_producer()
        metric = parse_metric_line(output, config)
    except GateError as exc:
        print(f"FAIL: allocation gate: {exc}", file=sys.stderr)
        return 1
    except OSError as exc:
        print(f"FAIL: allocation gate: cannot read input: {exc}", file=sys.stderr)
        return 1

    print(
        f"== allocation gate: admit+release allocs/op must be <= {threshold} "
        f"(schema {metric['schema']}, {metric['completed']} ops) =="
    )
    exact_per_op = Decimal(metric["total_allocations"]) / metric["completed"]
    if Decimal(metric["total_allocations"]) > threshold * metric["completed"]:
        print(
            f"FAIL: {metric['total_allocations']} allocations / {metric['completed']} ops "
            f"= {exact_per_op} exceeds baseline {threshold}",
            file=sys.stderr,
        )
        return 1
    print(
        f"ok: {metric['total_allocations']} allocations / {metric['completed']} ops "
        f"= {exact_per_op} (<= baseline {threshold})"
    )
    print("bench-gate: allocation gate passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
