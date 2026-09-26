#!/usr/bin/env python3
"""Acquire balanced Snapshot-on/off host pairs with identical work and binary."""

from __future__ import annotations

import argparse
import copy
import sys
from pathlib import Path
from typing import Any

import host_aa
import host_perf
from bench_process import run_control
from host_run import retain_control_bundle, write_new

BUNDLE_VERSION = 1


def mode_for(index: int) -> str:
    if index < 0:
        raise host_perf.ReceiptError("Snapshot run index must be nonnegative")
    return "on" if (index // 2 + index % 2) % 2 == 0 else "off"


def scenario_pair(source_bytes: bytes, cadence_ms: int) -> tuple[bytes, bytes]:
    if cadence_ms <= 0:
        raise host_perf.ReceiptError("Snapshot cadence must be positive")
    original = host_perf.parse_object(source_bytes, "observer scenario")
    if not isinstance(original.get("load"), dict):
        raise host_perf.ReceiptError("observer scenario lacks load envelope")
    off = copy.deepcopy(original)
    on = copy.deepcopy(original)
    off["load"]["snapshot_ms"] = 0
    on["load"]["snapshot_ms"] = cadence_ms
    return host_perf.canonical(off) + b"\n", host_perf.canonical(on) + b"\n"


def verify_bundle(bundle_bytes: bytes, directory: Path) -> dict[str, Any]:
    bundle = host_perf.parse_object(bundle_bytes, "Snapshot study")
    host_perf.exact_keys(
        bundle,
        {
            "schema_version",
            "kind",
            "status",
            "snapshot_ms",
            "off_scenario_sha256",
            "on_scenario_sha256",
            "binary_sha256",
            "identity",
            "runs",
            "failures",
        },
        "Snapshot study",
    )
    if bundle["schema_version"] != BUNDLE_VERSION or bundle["kind"] != "snapshot_on_off":
        raise host_perf.ReceiptError("unsupported Snapshot study")
    if bundle["status"] != "diagnostic_complete" or bundle["failures"] != []:
        raise host_perf.ReceiptError("Snapshot study is incomplete")
    cadence = host_perf.nat(bundle["snapshot_ms"], "snapshot_ms")
    if cadence == 0:
        raise host_perf.ReceiptError("Snapshot study cadence is zero")
    off_bytes = (directory / "scenario-off.json").read_bytes()
    on_bytes = (directory / "scenario-on.json").read_bytes()
    if (
        host_perf.sha256(off_bytes) != bundle["off_scenario_sha256"]
        or host_perf.sha256(on_bytes) != bundle["on_scenario_sha256"]
    ):
        raise host_perf.ReceiptError("Snapshot scenario artifact differs")
    derived_off, derived_on = scenario_pair(off_bytes, cadence)
    if off_bytes != derived_off or on_bytes != derived_on:
        raise host_perf.ReceiptError("Snapshot scenarios differ beyond sampling cadence")
    host_perf.validate_identity(bundle["identity"])
    runs = bundle["runs"]
    if not isinstance(runs, list) or len(runs) < 2 or len(runs) % 2:
        raise host_perf.ReceiptError("Snapshot study lacks complete pairs")
    for index, recorded in enumerate(runs):
        if not isinstance(recorded, dict):
            raise host_perf.ReceiptError("Snapshot run must be an object")
        host_perf.exact_keys(recorded, host_aa.RUN_KEYS | {"mode"}, f"Snapshot run {index}")
        mode = mode_for(index)
        if recorded["mode"] != mode:
            raise host_perf.ReceiptError("Snapshot pair order or mode differs")
        source = on_bytes if mode == "on" else off_bytes
        actual, identity = host_aa.verified_run(directory, index, source)
        actual["mode"] = mode
        if actual != recorded:
            raise host_perf.ReceiptError(f"Snapshot run {index} artifact or metric differs")
        if identity != bundle["identity"] or actual["binary_sha256"] != bundle["binary_sha256"]:
            raise host_perf.ReceiptError("Snapshot source, host or binary changed")
    host_aa.validate_study_windows(directory, runs, "Snapshot")
    return bundle


def acquire(
    scenario: Path,
    calibration: Path,
    directory: Path,
    pairs: int,
    cadence_ms: int,
    features: tuple[str, ...] = (),
) -> dict[str, Any]:
    if pairs < 1 or pairs > 50:
        raise host_perf.ReceiptError("Snapshot pairs must be 1..=50")
    off_bytes, on_bytes = scenario_pair(scenario.read_bytes(), cadence_ms)
    directory.mkdir(parents=True, exist_ok=False)
    off_path = directory / "scenario-off.json"
    on_path = directory / "scenario-on.json"
    write_new(off_path, off_bytes)
    write_new(on_path, on_bytes)
    runs = []
    failures = []
    common_identity = None
    common_binary = None
    for index in range(pairs * 2):
        mode = mode_for(index)
        source_path = on_path if mode == "on" else off_path
        source_bytes = on_bytes if mode == "on" else off_bytes
        artifact = host_aa.paths(directory, index)
        result = run_control(
            [
                sys.executable,
                str(Path(__file__).with_name("host_run.py")),
                str(source_path),
                str(artifact["raw"]),
                str(artifact["summary"]),
                str(calibration),
                *(part for feature in features for part in ("--feature", feature)),
            ],
            cwd=host_perf.REPO,
        )
        if result.returncode != 0:
            failures.append({"index": index, "reason": result.stderr[-1000:]})
            break
        try:
            row, identity = host_aa.verified_run(directory, index, source_bytes)
        except (OSError, host_perf.ReceiptError) as error:
            failures.append({"index": index, "reason": str(error)})
            break
        row["mode"] = mode
        if common_identity is None:
            common_identity = identity
            common_binary = row["binary_sha256"]
        elif identity != common_identity or row["binary_sha256"] != common_binary:
            failures.append({"index": index, "reason": "Snapshot source, host or binary changed"})
            break
        runs.append(row)
    bundle = {
        "schema_version": BUNDLE_VERSION,
        "kind": "snapshot_on_off",
        "status": "diagnostic_complete" if not failures else "incomplete",
        "snapshot_ms": cadence_ms,
        "off_scenario_sha256": host_perf.sha256(off_bytes),
        "on_scenario_sha256": host_perf.sha256(on_bytes),
        "binary_sha256": common_binary,
        "identity": common_identity,
        "runs": runs,
        "failures": failures,
    }
    retain_control_bundle(
        directory / "snapshot-bundle.json",
        bundle,
        lambda data: verify_bundle(data, directory),
        "Snapshot",
    )
    return bundle


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scenario", type=Path)
    parser.add_argument("calibration", type=Path)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--pairs", type=int, default=1)
    parser.add_argument("--snapshot-ms", type=int, required=True)
    parser.add_argument("--feature", action="append", default=[])
    args = parser.parse_args()
    try:
        bundle = acquire(
            args.scenario,
            args.calibration,
            args.directory,
            args.pairs,
            args.snapshot_ms,
            tuple(args.feature),
        )
        print(
            f"SNAPSHOT_DIAGNOSTIC performance=UNQUALIFIED runs={len(bundle['runs'])} "
            f"bundle={args.directory / 'snapshot-bundle.json'}"
        )
        return 0
    except (OSError, host_perf.ReceiptError, ValueError) as error:
        print(f"Snapshot study rejected: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
