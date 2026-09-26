#!/usr/bin/env python3
"""Acquire and verify repeated same-binary public-host A/A diagnostic runs."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Any

import host_perf
from bench_process import run_control
from host_run import retain_control_bundle

BUNDLE_VERSION = 1
RUN_KEYS = {
    "index",
    "pair",
    "member",
    "raw_sha256",
    "summary_sha256",
    "provenance_sha256",
    "binary_sha256",
    "topology_sha256",
    "resources_sha256",
    "metrics",
}


def paths(directory: Path, index: int) -> dict[str, Path]:
    stem = f"run-{index:03d}.json"
    raw = directory / stem
    return {
        "raw": raw,
        "summary": directory / f"run-{index:03d}.summary.json",
        "provenance": directory / f"{stem}.provenance.json",
        "binary": directory / f"{stem}.runner",
        "topology": directory / f"{stem}.topology.json",
        "resources": directory / f"{stem}.resources.json",
    }


def verified_run(directory: Path, index: int, scenario_bytes: bytes) -> tuple[dict, dict]:
    artifacts = {name: path.read_bytes() for name, path in paths(directory, index).items()}
    summary = host_perf.verify_receipt(
        artifacts["raw"],
        scenario_bytes,
        artifacts["summary"],
        provenance_bytes=artifacts["provenance"],
        binary_bytes=artifacts["binary"],
        topology_bytes=artifacts["topology"],
        resource_bytes=artifacts["resources"],
    )

    return (
        {
            "index": index,
            "pair": index // 2,
            "member": "A1" if index % 2 == 0 else "A2",
            "raw_sha256": host_perf.sha256(artifacts["raw"]),
            "summary_sha256": host_perf.sha256(artifacts["summary"]),
            "provenance_sha256": host_perf.sha256(artifacts["provenance"]),
            "binary_sha256": host_perf.sha256(artifacts["binary"]),
            "topology_sha256": host_perf.sha256(artifacts["topology"]),
            "resources_sha256": host_perf.sha256(artifacts["resources"]),
            "metrics": {
                "counts": summary["metrics"]["counts"],
                "cohort_goodput": summary["metrics"]["cohort_goodput"],
                "success_latency_by_class_path": summary["metrics"][
                    "success_latency_by_class_path"
                ],
                "max_producer_lag_ns": summary["metrics"]["max_producer_lag_ns"],
            },
        },
        summary["identity"],
    )


def validate_study_windows(directory: Path, runs: list[dict], label: str) -> None:
    """Bind indexed members to serial process windows, with digest rechecks."""
    previous_end = 0
    boots = set()
    cadences = set()
    for index, run in enumerate(runs):
        artifact_paths = paths(directory, index)
        artifacts = {}
        for name in ("provenance", "resources"):
            data = artifact_paths[name].read_bytes()
            if host_perf.sha256(data) != run[f"{name}_sha256"]:
                raise host_perf.ReceiptError(f"{label} {name} changed during verification")
            artifacts[name] = data
        provenance = host_perf.parse_object(artifacts["provenance"], f"{label} provenance")
        resources = host_perf.validate_resource_artifact(
            artifacts["resources"], provenance["runner_pid"]
        )
        start = resources["sampling_started_epoch_ns"]
        end = resources["sampling_ended_epoch_ns"]
        if start == 0 or end <= start:
            raise host_perf.ReceiptError(f"{label} process window is invalid")
        if previous_end > start:
            raise host_perf.ReceiptError(f"{label} windows overlap or indexed order is reversed")
        previous_end = end
        boots.add(resources["boot_time_ns"])
        cadences.add(resources["cadence_ms"])
    if len(boots) != 1 or len(cadences) != 1:
        raise host_perf.ReceiptError(f"{label} host boot or resource cadence changed")


def verify_bundle(bundle_bytes: bytes, directory: Path, scenario_bytes: bytes) -> dict[str, Any]:
    bundle = host_perf.parse_object(bundle_bytes, "A/A bundle")
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
        },
        "A/A bundle",
    )
    if bundle["schema_version"] != BUNDLE_VERSION or bundle["kind"] != "same_binary_aa":
        raise host_perf.ReceiptError("unsupported A/A bundle")
    if bundle["status"] != "diagnostic_complete" or bundle["failures"] != []:
        raise host_perf.ReceiptError("A/A bundle is incomplete")
    if bundle["scenario_sha256"] != host_perf.sha256(scenario_bytes):
        raise host_perf.ReceiptError("A/A scenario digest differs")
    runs = bundle["runs"]
    if not isinstance(runs, list) or len(runs) < 2 or len(runs) % 2:
        raise host_perf.ReceiptError("A/A run population must contain complete pairs")
    host_perf.validate_identity(bundle["identity"])
    for index, recorded in enumerate(runs):
        if not isinstance(recorded, dict):
            raise host_perf.ReceiptError("A/A run must be an object")
        host_perf.exact_keys(recorded, RUN_KEYS, f"A/A run {index}")
        actual, identity = verified_run(directory, index, scenario_bytes)
        if recorded != actual:
            raise host_perf.ReceiptError(f"A/A run {index} artifact or metric differs")
        if identity != bundle["identity"] or actual["binary_sha256"] != bundle["binary_sha256"]:
            raise host_perf.ReceiptError("A/A source, host or binary changed")
    validate_study_windows(directory, runs, "A/A")
    return bundle


def acquire(
    scenario: Path,
    calibration: Path,
    directory: Path,
    pairs: int,
    features: tuple[str, ...] = (),
) -> dict[str, Any]:
    if pairs < 1 or pairs > 50:
        raise host_perf.ReceiptError("A/A pairs must be 1..=50")
    directory.mkdir(parents=True, exist_ok=False)
    scenario_bytes = scenario.read_bytes()
    host_perf.parse_object(scenario_bytes, "scenario")
    runs = []
    failures = []
    common_identity = None
    common_binary = None
    for index in range(pairs * 2):
        artifact = paths(directory, index)
        result = run_control(
            [
                sys.executable,
                str(Path(__file__).with_name("host_run.py")),
                str(scenario),
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
            row, identity = verified_run(directory, index, scenario_bytes)
        except (OSError, host_perf.ReceiptError) as error:
            failures.append({"index": index, "reason": str(error)})
            break
        if common_identity is None:
            common_identity = identity
            common_binary = row["binary_sha256"]
        elif identity != common_identity or row["binary_sha256"] != common_binary:
            failures.append({"index": index, "reason": "A/A source, host or binary changed"})
            break
        runs.append(row)
    bundle = {
        "schema_version": BUNDLE_VERSION,
        "kind": "same_binary_aa",
        "status": "diagnostic_complete" if not failures else "incomplete",
        "scenario_sha256": host_perf.sha256(scenario_bytes),
        "binary_sha256": common_binary,
        "identity": common_identity,
        "runs": runs,
        "failures": failures,
    }
    retain_control_bundle(
        directory / "aa-bundle.json",
        bundle,
        lambda data: verify_bundle(data, directory, scenario_bytes),
        "A/A",
    )
    return bundle


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scenario", type=Path)
    parser.add_argument("calibration", type=Path)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--pairs", type=int, default=1)
    parser.add_argument("--feature", action="append", default=[])
    args = parser.parse_args()
    try:
        bundle = acquire(
            args.scenario, args.calibration, args.directory, args.pairs, tuple(args.feature)
        )
        print(
            f"AA_DIAGNOSTIC performance=UNQUALIFIED runs={len(bundle['runs'])} "
            f"bundle={args.directory / 'aa-bundle.json'}"
        )
        return 0
    except (OSError, host_perf.ReceiptError, ValueError) as error:
        print(f"A/A rejected: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
