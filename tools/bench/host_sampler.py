#!/usr/bin/env python3
"""Acquire balanced resource-sampler on/off public-host diagnostic pairs."""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path
from typing import Any

import host_aa
import host_observer
import host_perf
from host_run import write_new

BUNDLE_VERSION = 1
OFF_REASON = "resource sampling intentionally disabled for paired control"


def mode_for(index: int) -> str:
    return host_observer.mode_for(index)


def verified_run(
    directory: Path, index: int, scenario_bytes: bytes
) -> tuple[dict[str, Any], dict[str, Any]]:
    row, identity = host_aa.verified_run(directory, index, scenario_bytes)
    paths = host_aa.paths(directory, index)
    provenance = host_perf.parse_object(paths["provenance"].read_bytes(), "sampler provenance")
    resources = host_perf.validate_resource_artifact(
        paths["resources"].read_bytes(), provenance["runner_pid"]
    )
    mode = mode_for(index)
    if mode == "on":
        if resources["status"] != "complete":
            raise host_perf.ReceiptError("sampler-on arm has no complete resource samples")
    elif (
        resources["status"] != "unavailable"
        or resources["reason"] != OFF_REASON
        or resources["samples"] != []
    ):
        raise host_perf.ReceiptError("sampler-off arm collected resources or has wrong reason")
    start = resources["sampling_started_epoch_ns"]
    end = resources["sampling_ended_epoch_ns"]
    if start == 0 or end <= start:
        raise host_perf.ReceiptError("sampler process window is invalid")
    row["mode"] = mode
    row["resource_status"] = resources["status"]
    row["resource_reason"] = resources["reason"]
    row["window_epoch_ns"] = [start, end]
    row["boot_time_ns"] = resources["boot_time_ns"]
    row["resource_cadence_ms"] = resources["cadence_ms"]
    return row, identity


def verify_bundle(bundle_bytes: bytes, directory: Path, scenario_bytes: bytes) -> dict[str, Any]:
    bundle = host_perf.parse_object(bundle_bytes, "resource sampler study")
    host_perf.exact_keys(
        bundle,
        {
            "schema_version",
            "kind",
            "status",
            "performance",
            "scenario_sha256",
            "binary_sha256",
            "identity",
            "max_span_ns",
            "runs",
            "failures",
        },
        "resource sampler study",
    )
    if (
        type(bundle["schema_version"]) is not int
        or bundle["schema_version"] != BUNDLE_VERSION
        or bundle["kind"] != "resource_sampler_on_off"
        or bundle["status"] != "diagnostic_complete"
        or bundle["performance"] != "UNQUALIFIED"
        or bundle["failures"] != []
    ):
        raise host_perf.ReceiptError("resource sampler study is incomplete or unsupported")
    if bundle["scenario_sha256"] != host_perf.sha256(scenario_bytes):
        raise host_perf.ReceiptError("resource sampler scenario digest differs")
    host_perf.validate_identity(bundle["identity"])
    max_span_ns = host_perf.nat(bundle["max_span_ns"], "sampler max_span_ns")
    if max_span_ns == 0:
        raise host_perf.ReceiptError("sampler maximum span must be positive")
    runs = bundle["runs"]
    if not isinstance(runs, list) or len(runs) < 4 or len(runs) % 2:
        raise host_perf.ReceiptError("resource sampler study lacks balanced pairs")
    windows = []
    boots = set()
    cadences = set()
    for index, recorded in enumerate(runs):
        if not isinstance(recorded, dict):
            raise host_perf.ReceiptError("resource sampler run must be an object")
        host_perf.exact_keys(
            recorded,
            host_aa.RUN_KEYS
            | {
                "mode",
                "resource_status",
                "resource_reason",
                "window_epoch_ns",
                "boot_time_ns",
                "resource_cadence_ms",
            },
            f"resource sampler run {index}",
        )
        actual, identity = verified_run(directory, index, scenario_bytes)
        if recorded != actual:
            raise host_perf.ReceiptError(f"resource sampler run {index} artifact differs")
        if identity != bundle["identity"] or actual["binary_sha256"] != bundle["binary_sha256"]:
            raise host_perf.ReceiptError("resource sampler source, host or binary changed")
        windows.append((actual["window_epoch_ns"][0], actual["window_epoch_ns"][1]))
        boots.add(actual["boot_time_ns"])
        cadences.add(actual["resource_cadence_ms"])
    if len(boots) != 1 or len(cadences) != 1:
        raise host_perf.ReceiptError("resource sampler boot or cadence changed")
    ordered = sorted(windows)
    if any(first[1] > second[0] for first, second in zip(ordered, ordered[1:])):
        raise host_perf.ReceiptError("resource sampler process windows overlap")
    if ordered[-1][1] - ordered[0][0] > max_span_ns:
        raise host_perf.ReceiptError("resource sampler acquisition exceeds declared span")
    return bundle


def acquire(
    scenario: Path,
    calibration: Path,
    directory: Path,
    pairs: int,
    max_span_ns: int,
    features: tuple[str, ...] = (),
) -> dict[str, Any]:
    if pairs < 2 or pairs > 50 or max_span_ns <= 0:
        raise host_perf.ReceiptError("sampler pairs must be 2..=50 and span must be positive")
    directory.mkdir(parents=True, exist_ok=False)
    scenario_bytes = scenario.read_bytes()
    host_perf.parse_object(scenario_bytes, "sampler scenario")
    runs = []
    failures = []
    common_identity = None
    common_binary = None
    for index in range(pairs * 2):
        mode = mode_for(index)
        paths = host_aa.paths(directory, index)
        result = subprocess.run(
            [
                sys.executable,
                str(Path(__file__).with_name("host_run.py")),
                str(scenario),
                str(paths["raw"]),
                str(paths["summary"]),
                str(calibration),
                *(part for feature in features for part in ("--feature", feature)),
                *(["--no-resource-sampling"] if mode == "off" else []),
            ],
            cwd=host_perf.REPO,
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode != 0:
            failures.append({"index": index, "reason": result.stderr[-1000:]})
            break
        try:
            row, identity = verified_run(directory, index, scenario_bytes)
        except (OSError, host_perf.ReceiptError) as error:
            failures.append({"index": index, "reason": str(error)})
            break
        if common_identity is None:
            common_identity = identity
            common_binary = row["binary_sha256"]
        elif identity != common_identity or row["binary_sha256"] != common_binary:
            failures.append({"index": index, "reason": "sampler source, host or binary changed"})
            break
        runs.append(row)
    bundle = {
        "schema_version": BUNDLE_VERSION,
        "kind": "resource_sampler_on_off",
        "status": "diagnostic_complete" if not failures else "incomplete",
        "performance": "UNQUALIFIED",
        "scenario_sha256": host_perf.sha256(scenario_bytes),
        "binary_sha256": common_binary,
        "identity": common_identity,
        "max_span_ns": max_span_ns,
        "runs": runs,
        "failures": failures,
    }
    bundle_bytes = host_perf.canonical(bundle) + b"\n"
    write_new(directory / "sampler-bundle.json", bundle_bytes)
    if failures:
        raise host_perf.ReceiptError(f"resource sampler acquisition incomplete: {failures[0]}")
    verify_bundle(bundle_bytes, directory, scenario_bytes)
    return bundle


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scenario", type=Path)
    parser.add_argument("calibration", type=Path)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--pairs", type=int, default=2)
    parser.add_argument("--max-span-seconds", type=int, required=True)
    parser.add_argument("--feature", action="append", default=[])
    args = parser.parse_args()
    try:
        bundle = acquire(
            args.scenario,
            args.calibration,
            args.directory,
            args.pairs,
            args.max_span_seconds * 1_000_000_000,
            tuple(args.feature),
        )
        print(
            f"SAMPLER_DIAGNOSTIC performance=UNQUALIFIED runs={len(bundle['runs'])} "
            f"bundle={args.directory / 'sampler-bundle.json'}"
        )
        return 0
    except (OSError, host_perf.ReceiptError, ValueError) as error:
        print(f"resource sampler study rejected: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
