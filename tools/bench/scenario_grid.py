#!/usr/bin/env python3
"""Expand one fixed public-host offer into a bounded uniform open-loop rate point.

The caller chooses rates before a measured comparison. This generator records
the exact intended schedule but does not approve a rate grid or performance.
"""

from __future__ import annotations

import argparse
import copy
import sys
from pathlib import Path
from typing import Any

import host_perf
from host_run import write_new

SCHEMA_VERSION = 1
MAX_SCENARIO_BYTES = 16 * 1024 * 1024


def uniform_rate_point(source: dict[str, Any], rate_per_second: int) -> dict[str, Any]:
    if type(rate_per_second) is not int or rate_per_second <= 0:
        raise ValueError("absolute rate must be a positive integer per second")
    load = source.get("load")
    offers = source.get("offers")
    if not isinstance(load, dict) or not isinstance(offers, list) or len(offers) != 1:
        raise ValueError("rate template requires exactly one declared offer")
    template = offers[0]
    if (
        not isinstance(template, dict)
        or type(template.get("send_time_ns")) is not int
        or template["send_time_ns"] != 0
    ):
        raise ValueError("rate template offer must start at zero")
    injection_ms = load.get("injection_ms")
    if type(injection_ms) is not int or injection_ms <= 0 or injection_ms % 1000:
        raise ValueError("rate template injection window must contain whole seconds")
    max_records = load.get("max_records")
    max_outstanding = load.get("max_outstanding")
    if (
        type(max_records) is not int
        or type(max_outstanding) is not int
        or max_records <= 0
        or max_outstanding <= 0
        or max_outstanding > max_records
    ):
        raise ValueError("rate template has invalid producer or record bounds")
    population = rate_per_second * (injection_ms // 1000)
    if population <= 0 or population > 1_000_000:
        raise ValueError("absolute rate exceeds the scenario record bound")
    source_id = source.get("id")
    if not isinstance(source_id, str):
        raise ValueError("rate point scenario id is invalid")
    output_id = f"{source_id}-r{rate_per_second}"
    if len(output_id.encode()) > 128:
        raise ValueError("rate point scenario id is invalid")
    derived = copy.deepcopy(source)
    derived["id"] = output_id
    derived["load"]["max_records"] = max(max_records, population)
    derived["offers"] = [
        {**template, "send_time_ns": index * 1_000_000_000 // rate_per_second}
        for index in range(population)
    ]
    if len(host_perf.canonical(derived)) + 1 > MAX_SCENARIO_BYTES:
        raise ValueError("rate point exceeds the Rust scenario preflight input bound")
    return derived


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("template", type=Path)
    parser.add_argument("rate_per_second", type=int)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    manifest_path = args.output.with_name(args.output.name + ".manifest.json")
    try:
        source_bytes = args.template.read_bytes()
        source = host_perf.parse_object(source_bytes, "rate template")
        derived = uniform_rate_point(source, args.rate_per_second)
        scenario_bytes = host_perf.canonical(derived) + b"\n"
        manifest = {
            "schema_version": SCHEMA_VERSION,
            "status": "diagnostic_rate_point_unqualified",
            "template_sha256": host_perf.sha256(source_bytes),
            "scenario_sha256": host_perf.sha256(scenario_bytes),
            "rate_per_second": args.rate_per_second,
            "injection_ms": derived["load"]["injection_ms"],
            "intended": len(derived["offers"]),
        }
        if args.output.exists() or manifest_path.exists():
            raise host_perf.ReceiptError("rate point artifact paths must be fresh")
        write_new(args.output, scenario_bytes)
        write_new(manifest_path, host_perf.canonical(manifest) + b"\n")
        print(f"RATE_POINT_DIAGNOSTIC intended={manifest['intended']} scenario={args.output}")
        return 0
    except (OSError, ValueError, host_perf.ReceiptError) as error:
        print(f"rate point rejected: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
