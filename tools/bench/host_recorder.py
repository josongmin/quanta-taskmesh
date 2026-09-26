#!/usr/bin/env python3
"""Acquire balanced full/minimal recorder pairs on one public-host binary."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Any

import host_aa
import host_observer
import host_perf
from host_run import retain_control_bundle

BUNDLE_VERSION = 3


def mode_for(index: int) -> str:
    return "full" if host_observer.mode_for(index) == "on" else "minimal"


def verified_run(
    directory: Path, index: int, scenario_bytes: bytes, mode: str
) -> tuple[dict, dict]:
    artifact_paths = host_aa.paths(directory, index)
    if mode == "full":
        row, identity = host_aa.verified_run(directory, index, scenario_bytes)
    elif mode == "minimal":
        artifacts = {
            name: path.read_bytes() for name, path in artifact_paths.items() if name != "summary"
        }
        provenance = host_perf.parse_object(artifacts["provenance"], "minimal provenance")
        identity = provenance.get("end_identity")
        host_perf.validate_identity(identity)
        host_perf.validate_with_rust(
            scenario_bytes,
            artifacts["raw"],
            identity["features"],
            artifacts["topology"],
            raw_kind="minimal",
        )
        host_perf.validate_execution_provenance(
            artifacts["provenance"],
            artifacts["raw"],
            scenario_bytes,
            identity,
            artifacts["binary"],
            artifacts["topology"],
            artifacts["resources"],
            runner_mode="minimal",
        )
        raw = host_perf.parse_object(artifacts["raw"], "minimal raw")
        if (
            raw.get("recorder_mode") != "minimal"
            or raw.get("response_latency_available") is not False
        ):
            raise host_perf.ReceiptError("minimal recorder falsely reports latency")
        row = {
            "index": index,
            "pair": index // 2,
            "member": "A1" if index % 2 == 0 else "A2",
            "raw_sha256": host_perf.sha256(artifacts["raw"]),
            "summary_sha256": None,
            "provenance_sha256": host_perf.sha256(artifacts["provenance"]),
            "binary_sha256": host_perf.sha256(artifacts["binary"]),
            "topology_sha256": host_perf.sha256(artifacts["topology"]),
            "resources_sha256": host_perf.sha256(artifacts["resources"]),
            "metrics": {
                "counts": raw["counts"],
                "cohort_goodput": None,
                "success_latency_by_class_path": None,
                "max_producer_lag_ns": raw["max_producer_lag_ns"],
            },
        }
    else:
        raise host_perf.ReceiptError("unknown recorder mode")
    row["mode"] = mode
    provenance = host_perf.parse_object(artifact_paths["provenance"].read_bytes(), "provenance")
    resources = host_perf.validate_resource_artifact(
        artifact_paths["resources"].read_bytes(), provenance["runner_pid"]
    )
    row["resource_cadence_ms"] = resources["cadence_ms"]
    row["probe_process_duration_ns"] = (
        resources["sampling_ended_monotonic_ns"] - resources["sampling_started_monotonic_ns"]
    )
    if row["probe_process_duration_ns"] <= 0:
        raise host_perf.ReceiptError("recorder probe duration must be positive")
    return row, identity


def verify_bundle(bundle_bytes: bytes, directory: Path, scenario_bytes: bytes) -> dict[str, Any]:
    bundle = host_perf.parse_object(bundle_bytes, "recorder study")
    host_perf.exact_keys(
        bundle,
        {
            "schema_version",
            "kind",
            "status",
            "scenario_sha256",
            "binary_sha256",
            "identity",
            "runs",
            "failures",
            "expected_runs",
            "timeout_seconds",
            "executions",
        },
        "recorder study",
    )
    if (
        type(bundle["schema_version"]) is not int
        or bundle["schema_version"] != BUNDLE_VERSION
        or bundle["kind"] != "recorder_full_minimal"
    ):
        raise host_perf.ReceiptError("unsupported recorder study")
    if bundle["status"] != "diagnostic_complete" or bundle["failures"] != []:
        raise host_perf.ReceiptError("recorder study is incomplete")
    if bundle["scenario_sha256"] != host_perf.sha256(scenario_bytes):
        raise host_perf.ReceiptError("recorder scenario digest differs")
    scenario = host_perf.parse_object(scenario_bytes, "recorder scenario")
    load = scenario.get("load")
    if not isinstance(load, dict) or load.get("snapshot_ms") != 0:
        raise host_perf.ReceiptError("recorder study must disable Snapshot in both arms")
    host_perf.validate_identity(bundle["identity"])
    runs = bundle["runs"]
    if not isinstance(runs, list) or len(runs) < 2 or len(runs) % 2:
        raise host_perf.ReceiptError("recorder study lacks complete pairs")
    expected_cadence = None
    host_aa.validate_executions(bundle, directory, runs)
    for index, recorded in enumerate(runs):
        if not isinstance(recorded, dict):
            raise host_perf.ReceiptError("recorder run must be an object")
        host_perf.exact_keys(
            recorded,
            host_aa.RUN_KEYS | {"mode", "resource_cadence_ms", "probe_process_duration_ns"},
            f"recorder run {index}",
        )
        mode = mode_for(index)
        if recorded["mode"] != mode:
            raise host_perf.ReceiptError("recorder pair order or mode differs")
        actual, identity = verified_run(directory, index, scenario_bytes, mode)
        if recorded != actual:
            raise host_perf.ReceiptError(f"recorder run {index} artifact or metric differs")
        if identity != bundle["identity"] or actual["binary_sha256"] != bundle["binary_sha256"]:
            raise host_perf.ReceiptError("recorder source, host or binary changed")
        if expected_cadence is None:
            expected_cadence = actual["resource_cadence_ms"]
        elif actual["resource_cadence_ms"] != expected_cadence:
            raise host_perf.ReceiptError("recorder resource sampling cadence changed")
    host_aa.validate_study_windows(directory, runs, "recorder")
    return bundle


def acquire(
    scenario: Path,
    calibration: Path,
    directory: Path,
    pairs: int,
    features: tuple[str, ...] = (),
    *,
    timeout_seconds: float = 1800,
) -> dict[str, Any]:
    host_aa.validate_timeout(timeout_seconds)
    if pairs < 1 or pairs > 50:
        raise host_perf.ReceiptError("recorder pairs must be 1..=50")
    scenario_bytes = scenario.read_bytes()
    parsed = host_perf.parse_object(scenario_bytes, "recorder scenario")
    load = parsed.get("load")
    if not isinstance(load, dict) or load.get("snapshot_ms") != 0:
        raise host_perf.ReceiptError("recorder study requires Snapshot sampling off")
    directory.mkdir(parents=True, exist_ok=False)
    runs = []
    executions = []
    failures = []
    common_identity = None
    common_binary = None
    for index in range(pairs * 2):
        mode = mode_for(index)
        artifact = host_aa.paths(directory, index)
        if mode == "full":
            command = [
                sys.executable,
                str(Path(__file__).with_name("host_run.py")),
                str(scenario),
                str(artifact["raw"]),
                str(artifact["summary"]),
                str(calibration),
                *(part for feature in features for part in ("--feature", feature)),
            ]
        else:
            command = [
                sys.executable,
                str(Path(__file__).with_name("minimal_run.py")),
                str(scenario),
                str(artifact["raw"]),
                *(part for feature in features for part in ("--feature", feature)),
            ]
        terminal, reason = host_aa.execute_arm(directory, index, command, timeout_seconds)
        executions.append(terminal)
        if reason:
            failures.append({"index": index, "reason": reason})
            break
        try:
            row, identity = verified_run(directory, index, scenario_bytes, mode)
        except (OSError, host_perf.ReceiptError) as error:
            failures.append({"index": index, "reason": str(error)})
            break
        if common_identity is None:
            common_identity = identity
            common_binary = row["binary_sha256"]
        elif identity != common_identity or row["binary_sha256"] != common_binary:
            failures.append({"index": index, "reason": "recorder source, host or binary changed"})
            break
        runs.append(row)
    host_aa.account_unlaunched(directory, executions, failures, pairs * 2, timeout_seconds)
    bundle = {
        "schema_version": BUNDLE_VERSION,
        "kind": "recorder_full_minimal",
        "status": "diagnostic_complete" if not failures else "incomplete",
        "scenario_sha256": host_perf.sha256(scenario_bytes),
        "binary_sha256": common_binary,
        "identity": common_identity,
        "runs": runs,
        "failures": failures,
        "expected_runs": pairs * 2,
        "timeout_seconds": timeout_seconds,
        "executions": executions,
    }
    retain_control_bundle(
        directory / "recorder-bundle.json",
        bundle,
        lambda data: verify_bundle(data, directory, scenario_bytes),
        "recorder",
    )
    return bundle


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scenario", type=Path)
    parser.add_argument("calibration", type=Path)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--pairs", type=int, default=1)
    parser.add_argument("--feature", action="append", default=[])
    parser.add_argument("--timeout-seconds", type=float, default=1800)
    args = parser.parse_args()
    try:
        bundle = acquire(
            args.scenario,
            args.calibration,
            args.directory,
            args.pairs,
            tuple(args.feature),
            timeout_seconds=args.timeout_seconds,
        )
        print(
            f"RECORDER_DIAGNOSTIC performance=UNQUALIFIED runs={len(bundle['runs'])} "
            f"bundle={args.directory / 'recorder-bundle.json'}"
        )
        return 0
    except (OSError, host_perf.ReceiptError, ValueError) as error:
        print(f"recorder study rejected: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
