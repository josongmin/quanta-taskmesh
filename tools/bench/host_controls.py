#!/usr/bin/env python3
# ruff: noqa: UP045 -- pyproject supports Python 3.9, which lacks PEP 604 unions.
"""Bind existing host controls into one revalidated diagnostic artifact.

Every constituent verifier owns its raw semantics. This file owns cross-control
identity, role, freshness and nonoverlap only. It does not set performance budgets.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Any, Optional

import host_aa
import host_observer
import host_perf
import host_recorder
from host_run import write_new

SCHEMA_VERSION = 1


def sidecars(raw: Path, *, summary: Optional[Path] = None) -> dict[str, Path]:
    result = {
        "raw": raw,
        "provenance": raw.with_name(raw.name + ".provenance.json"),
        "binary": raw.with_name(raw.name + ".runner"),
        "topology": raw.with_name(raw.name + ".topology.json"),
        "resources": raw.with_name(raw.name + ".resources.json"),
    }
    if summary is not None:
        result["summary"] = summary
    return result


def read_artifacts(paths: dict[str, Path]) -> dict[str, bytes]:
    return {name: path.read_bytes() for name, path in paths.items()}


def digest_map(artifacts: dict[str, bytes]) -> dict[str, str]:
    return {name: host_perf.sha256(data) for name, data in artifacts.items()}


def complete_resources(data: bytes, pid: int, label: str) -> tuple[int, int, int, int]:
    resource = host_perf.validate_resource_artifact(data, pid)
    if resource["status"] != "complete":
        raise host_perf.ReceiptError(f"{label}: resource sampling is unavailable")
    start = host_perf.nat(resource["sampling_started_epoch_ns"], f"{label}.start_epoch")
    end = host_perf.nat(resource["sampling_ended_epoch_ns"], f"{label}.end_epoch")
    cadence = host_perf.nat(resource["cadence_ms"], f"{label}.cadence")
    boot = host_perf.nat(resource["boot_time_ns"], f"{label}.boot_time")
    if start == 0 or end <= start or cadence == 0:
        raise host_perf.ReceiptError(f"{label}: invalid sampling window")
    return start, end, cadence, boot


def constituent_resource_windows(
    directory: Path, run_count: int, label: str
) -> list[tuple[str, bytes, bytes]]:
    result = []
    for index in range(run_count):
        paths = host_aa.paths(directory, index)
        result.append(
            (
                f"{label}/{index}",
                paths["provenance"].read_bytes(),
                paths["resources"].read_bytes(),
            )
        )
    return result


def make_bundle(
    scenario_path: Path,
    host_raw_path: Path,
    host_summary_path: Path,
    generator_raw_path: Path,
    aa_directory: Path,
    snapshot_directory: Path,
    recorder_directory: Path,
    max_span_ns: int,
) -> dict[str, Any]:
    if type(max_span_ns) is not int or max_span_ns <= 0:
        raise host_perf.ReceiptError("control max_span_ns must be positive")
    scenario_bytes = scenario_path.read_bytes()
    scenario = host_perf.parse_object(scenario_bytes, "control scenario")
    if not isinstance(scenario.get("load"), dict) or scenario["load"].get("snapshot_ms") != 0:
        raise host_perf.ReceiptError("control scenario requires Snapshot sampling off")
    host = read_artifacts(sidecars(host_raw_path, summary=host_summary_path))
    host_summary = host_perf.verify_receipt(
        host["raw"],
        scenario_bytes,
        host["summary"],
        provenance_bytes=host["provenance"],
        binary_bytes=host["binary"],
        topology_bytes=host["topology"],
        resource_bytes=host["resources"],
    )
    identity = host_summary["identity"]
    host_provenance = host_perf.parse_object(host["provenance"], "host provenance")
    if host_provenance["runner_mode"] != "full":
        raise host_perf.ReceiptError("control target must use full recorder")
    generator = read_artifacts(sidecars(generator_raw_path))
    host_perf.validate_with_rust(
        scenario_bytes,
        generator["raw"],
        identity["features"],
        generator["topology"],
        raw_kind="generator",
    )
    host_perf.validate_execution_provenance(
        generator["provenance"],
        generator["raw"],
        scenario_bytes,
        identity,
        generator["binary"],
        generator["topology"],
        generator["resources"],
        example_name="host_generator_probe",
        runner_mode="generator",
    )
    if generator["topology"] != host["topology"]:
        raise host_perf.ReceiptError("generator and host resolved topologies differ")
    generator_raw = host_perf.parse_object(generator["raw"], "generator raw")
    aa_bytes = (aa_directory / "aa-bundle.json").read_bytes()
    aa = host_aa.verify_bundle(aa_bytes, aa_directory, scenario_bytes)
    snapshot_bytes = (snapshot_directory / "snapshot-bundle.json").read_bytes()
    snapshot = host_observer.verify_bundle(snapshot_bytes, snapshot_directory)
    expected_off, expected_on = host_observer.scenario_pair(scenario_bytes, snapshot["snapshot_ms"])
    if snapshot["off_scenario_sha256"] != host_perf.sha256(expected_off) or snapshot[
        "on_scenario_sha256"
    ] != host_perf.sha256(expected_on):
        raise host_perf.ReceiptError("Snapshot study differs from control workload")
    recorder_bytes = (recorder_directory / "recorder-bundle.json").read_bytes()
    recorder = host_recorder.verify_bundle(recorder_bytes, recorder_directory, scenario_bytes)
    host_binary_sha256 = host_perf.sha256(host["binary"])
    for label, control in (("A/A", aa), ("Snapshot", snapshot), ("recorder", recorder)):
        if control["identity"] != identity or control["binary_sha256"] != host_binary_sha256:
            raise host_perf.ReceiptError(f"{label} source, host or binary differs from target")
    if aa["scenario_sha256"] != host_perf.sha256(scenario_bytes) or recorder[
        "scenario_sha256"
    ] != host_perf.sha256(scenario_bytes):
        raise host_perf.ReceiptError("A/A or recorder workload differs from target")
    resource_artifacts = [
        ("target", host["provenance"], host["resources"]),
        ("generator", generator["provenance"], generator["resources"]),
    ]
    for label, directory, control in (
        ("aa", aa_directory, aa),
        ("snapshot", snapshot_directory, snapshot),
        ("recorder", recorder_directory, recorder),
    ):
        resource_artifacts.extend(
            constituent_resource_windows(directory, len(control["runs"]), label)
        )
    windows = []
    provenance_digests = set()
    cadences = set()
    boots = set()
    for label, provenance_bytes, resource_bytes in resource_artifacts:
        digest = host_perf.sha256(provenance_bytes)
        if digest in provenance_digests:
            raise host_perf.ReceiptError("control process artifact reused across roles")
        provenance_digests.add(digest)
        provenance = host_perf.parse_object(provenance_bytes, f"{label} provenance")
        start, end, cadence, boot = complete_resources(
            resource_bytes, provenance["runner_pid"], label
        )
        windows.append((start, end, label))
        cadences.add(cadence)
        boots.add(boot)
    if len(cadences) != 1:
        raise host_perf.ReceiptError("control resource sampling cadence differs")
    if len(boots) != 1:
        raise host_perf.ReceiptError("control processes span different host boots")
    windows.sort()
    for previous, current in zip(windows, windows[1:]):
        if current[0] < previous[1]:
            raise host_perf.ReceiptError(
                f"control process windows overlap: {previous[2]} and {current[2]}"
            )
    observed_span_ns = windows[-1][1] - windows[0][0]
    if observed_span_ns > max_span_ns:
        raise host_perf.ReceiptError("control acquisition exceeds declared span")
    generator_lags = [
        row["scheduled_lag_ns"]
        for row in generator_raw["records"]
        if row["scheduled_lag_ns"] is not None
    ]
    return {
        "schema_version": SCHEMA_VERSION,
        "status": "diagnostic_complete",
        "performance": "UNQUALIFIED",
        "scenario_sha256": host_perf.sha256(scenario_bytes),
        "identity": identity,
        "host_binary_sha256": host_binary_sha256,
        "generator_binary_sha256": host_perf.sha256(generator["binary"]),
        "max_span_ns": max_span_ns,
        "observed_span_ns": observed_span_ns,
        "resource_cadence_ms": cadences.pop(),
        "boot_time_ns": boots.pop(),
        "process_count": len(windows),
        "generator_not_submitted": generator_raw["not_submitted"],
        "generator_max_lag_ns": max(generator_lags),
        "host_not_submitted": host_summary["metrics"]["counts"]["not_submitted"],
        "host_max_lag_ns": host_summary["metrics"]["max_producer_lag_ns"],
        "artifact_sha256": {
            "target": digest_map(host),
            "generator": digest_map(generator),
            "aa_bundle": host_perf.sha256(aa_bytes),
            "snapshot_bundle": host_perf.sha256(snapshot_bytes),
            "recorder_bundle": host_perf.sha256(recorder_bytes),
        },
    }


def verify_bundle(bundle_bytes: bytes, *args: Any) -> dict[str, Any]:
    recorded = host_perf.parse_object(bundle_bytes, "control bundle")
    host_perf.exact_keys(
        recorded,
        {
            "schema_version",
            "status",
            "performance",
            "scenario_sha256",
            "identity",
            "host_binary_sha256",
            "generator_binary_sha256",
            "max_span_ns",
            "observed_span_ns",
            "resource_cadence_ms",
            "boot_time_ns",
            "process_count",
            "generator_not_submitted",
            "generator_max_lag_ns",
            "host_not_submitted",
            "host_max_lag_ns",
            "artifact_sha256",
        },
        "control bundle",
    )
    if recorded["schema_version"] != SCHEMA_VERSION:
        raise host_perf.ReceiptError("unsupported control bundle version")
    expected = make_bundle(*args, max_span_ns=recorded["max_span_ns"])
    if recorded != expected:
        raise host_perf.ReceiptError("control bundle differs from revalidated artifacts")
    return recorded


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scenario", type=Path)
    parser.add_argument("host_raw", type=Path)
    parser.add_argument("host_summary", type=Path)
    parser.add_argument("generator_raw", type=Path)
    parser.add_argument("aa_directory", type=Path)
    parser.add_argument("snapshot_directory", type=Path)
    parser.add_argument("recorder_directory", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--max-span-seconds", type=int, required=True)
    args = parser.parse_args()
    try:
        bundle = make_bundle(
            args.scenario,
            args.host_raw,
            args.host_summary,
            args.generator_raw,
            args.aa_directory,
            args.snapshot_directory,
            args.recorder_directory,
            args.max_span_seconds * 1_000_000_000,
        )
        bundle_bytes = host_perf.canonical(bundle) + b"\n"
        write_new(args.output, bundle_bytes)
        verify_bundle(
            bundle_bytes,
            args.scenario,
            args.host_raw,
            args.host_summary,
            args.generator_raw,
            args.aa_directory,
            args.snapshot_directory,
            args.recorder_directory,
        )
        print(f"CONTROL_DIAGNOSTIC performance=UNQUALIFIED bundle={args.output}")
        return 0
    except (OSError, host_perf.ReceiptError, ValueError) as error:
        print(f"control bundle rejected: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
