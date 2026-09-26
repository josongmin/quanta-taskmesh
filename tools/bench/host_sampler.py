#!/usr/bin/env python3
"""Acquire balanced resource-sampler on/off public-host diagnostic pairs."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Any

import host_aa
import host_observer
import host_perf
from host_run import retain_control_bundle

from tools.inspection import read_regular_bytes

BUNDLE_VERSION = 2
OFF_REASON = "resource sampling intentionally disabled for paired control"


def mode_for(index: int) -> str:
    return host_observer.mode_for(index)


def verified_run(
    directory: Path, index: int, scenario_bytes: bytes
) -> tuple[dict[str, Any], dict[str, Any]]:
    row, identity = host_aa.verified_run(directory, index, scenario_bytes)
    paths = host_aa.paths(directory, index)
    provenance = host_perf.parse_object(
        read_regular_bytes(paths["provenance"]), "sampler provenance"
    )
    resources = host_perf.validate_resource_artifact(
        read_regular_bytes(paths["resources"]), provenance["runner_pid"]
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
            "expected_runs",
            "timeout_seconds",
            "executions",
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
    if not isinstance(runs, list) or len(runs) < 4 or len(runs) % 4:
        raise host_perf.ReceiptError("resource sampler study lacks balanced pairs")
    windows = []
    boots = set()
    cadences = set()
    host_aa.validate_executions(bundle, directory, runs)
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
    if any(first[1] > second[0] for first, second in zip(windows, windows[1:])):
        raise host_perf.ReceiptError(
            "resource sampler process windows overlap or index order reversed"
        )
    if windows[-1][1] - windows[0][0] > max_span_ns:
        raise host_perf.ReceiptError("resource sampler acquisition exceeds declared span")
    return bundle


def acquire(
    scenario: Path,
    calibration: Path,
    directory: Path,
    pairs: int,
    max_span_ns: int,
    features: tuple[str, ...] = (),
    *,
    timeout_seconds: float = 1800,
) -> dict[str, Any]:
    host_aa.validate_timeout(timeout_seconds)
    if pairs < 2 or pairs > 50 or pairs % 2 or max_span_ns <= 0:
        raise host_perf.ReceiptError(
            "sampler pairs must be even in 2..=50 and span must be positive"
        )
    directory.mkdir(parents=True, exist_ok=False)
    scenario_bytes = read_regular_bytes(scenario)
    host_perf.parse_object(scenario_bytes, "sampler scenario")
    runs = []
    executions = []
    failures = []
    common_identity = None
    common_binary = None
    for index in range(pairs * 2):
        mode = mode_for(index)
        paths = host_aa.paths(directory, index)
        terminal, reason = host_aa.execute_arm(
            directory,
            index,
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
            timeout_seconds,
        )
        executions.append(terminal)
        if reason:
            failures.append({"index": index, "reason": reason})
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
    host_aa.account_unlaunched(directory, executions, failures, pairs * 2, timeout_seconds)
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
        "expected_runs": pairs * 2,
        "timeout_seconds": timeout_seconds,
        "executions": executions,
    }
    retain_control_bundle(
        directory / "sampler-bundle.json",
        bundle,
        lambda data: verify_bundle(data, directory, scenario_bytes),
        "resource sampler",
    )
    return bundle


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scenario", type=Path)
    parser.add_argument("calibration", type=Path)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--pairs", type=int, default=2)
    parser.add_argument("--max-span-seconds", type=int, required=True)
    parser.add_argument("--feature", action="append", default=[])
    parser.add_argument("--timeout-seconds", type=float, default=1800)
    args = parser.parse_args()
    try:
        bundle = acquire(
            args.scenario,
            args.calibration,
            args.directory,
            args.pairs,
            args.max_span_seconds * 1_000_000_000,
            tuple(args.feature),
            timeout_seconds=args.timeout_seconds,
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
