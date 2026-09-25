#!/usr/bin/env python3
"""Acquire a diagnostic null-work producer control with retained execution bytes."""

from __future__ import annotations

import argparse
import shutil
import sys
import tempfile
from pathlib import Path

import host_perf
from host_run import build_runner, retain_executable, write_new
from process_resource import sample_subprocess


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scenario", type=Path)
    parser.add_argument("raw", type=Path)
    parser.add_argument("--feature", action="append", default=[])
    args = parser.parse_args()
    provenance_path = args.raw.with_name(args.raw.name + ".provenance.json")
    executable_path = args.raw.with_name(args.raw.name + ".runner")
    topology_path = args.raw.with_name(args.raw.name + ".topology.json")
    resource_path = args.raw.with_name(args.raw.name + ".resources.json")
    try:
        if any(
            path.exists()
            for path in (args.raw, provenance_path, executable_path, topology_path, resource_path)
        ):
            raise host_perf.ReceiptError("generator artifact paths must be fresh")
        scenario_bytes = args.scenario.read_bytes()
        scenario = host_perf.parse_object(scenario_bytes, "scenario")
        features = sorted(set(args.feature))
        build_start_identity = host_perf.local_identity(scenario, features)
        binary, artifact_features, build_command = build_runner(
            features, "host_generator_probe"
        )
        if host_perf.local_identity(scenario, features) != build_start_identity:
            raise host_perf.ReceiptError("source, toolchain or host identity changed during build")
        with tempfile.TemporaryDirectory(prefix="taskmesh-generator-binary-") as binary_dir:
            sealed_binary = Path(binary_dir) / binary.name
            sealed_scenario = Path(binary_dir) / "scenario.json"
            shutil.copy2(binary, sealed_binary)
            sealed_scenario.write_bytes(scenario_bytes)
            binary_digest = host_perf.sha256(sealed_binary.read_bytes())
            executable_bytes = retain_executable(sealed_binary, executable_path)
            if host_perf.sha256(executable_bytes) != binary_digest:
                raise host_perf.ReceiptError("retained generator differs from sealed executable")
            start_identity = host_perf.local_identity(scenario, features)
            runner_exit_code, runner_pid, runner_stderr, resources = sample_subprocess(
                [
                    str(sealed_binary),
                    str(sealed_scenario),
                    str(args.raw),
                    "--topology-out",
                    str(topology_path),
                ],
                cwd=host_perf.REPO,
            )
            resource_bytes = host_perf.canonical(resources) + b"\n"
            write_new(resource_path, resource_bytes)
            end_identity = host_perf.local_identity(scenario, features)
            binary_unchanged = host_perf.sha256(sealed_binary.read_bytes()) == binary_digest
        raw_bytes = args.raw.read_bytes() if args.raw.exists() else None
        topology_bytes = topology_path.read_bytes() if topology_path.exists() else None
        reason = None
        if start_identity != end_identity:
            reason = "source, toolchain or host identity changed during run"
        elif (
            not binary_unchanged or host_perf.sha256(executable_path.read_bytes()) != binary_digest
        ):
            reason = "sealed or retained generator changed during run"
        elif runner_exit_code != 0:
            reason = f"generator exited {runner_exit_code}: {runner_stderr[-1000:]}"
        elif raw_bytes is None or topology_bytes is None:
            reason = "generator did not write raw and topology artifacts"
        else:
            try:
                host_perf.validate_with_rust(
                    scenario_bytes,
                    raw_bytes,
                    features,
                    topology_bytes,
                    raw_kind="generator",
                )
            except host_perf.ReceiptError as error:
                reason = f"generator raw validation failed: {error}"
        provenance = {
            "schema_version": 3,
            "status": "invalid" if reason else "complete",
            "reason": reason,
            "scenario_sha256": host_perf.sha256(scenario_bytes),
            "raw_sha256": host_perf.sha256(raw_bytes) if raw_bytes is not None else None,
            "binary_sha256": binary_digest,
            "binary_artifact": executable_path.name,
            "topology_sha256": (
                host_perf.sha256(topology_bytes) if topology_bytes is not None else None
            ),
            "topology_artifact": topology_path.name,
            "resources_sha256": host_perf.sha256(resource_bytes),
            "resources_artifact": resource_path.name,
            "runner_pid": runner_pid,
            "build_command": build_command,
            "build_artifact_features": artifact_features,
            "start_identity": start_identity,
            "end_identity": end_identity,
            "runner_exit_code": runner_exit_code,
        }
        if reason is None:
            assert raw_bytes is not None and topology_bytes is not None
            try:
                host_perf.validate_execution_provenance(
                    host_perf.canonical(provenance) + b"\n",
                    raw_bytes,
                    scenario_bytes,
                    end_identity,
                    executable_bytes,
                    topology_bytes,
                    resource_bytes,
                    example_name="host_generator_probe",
                )
            except host_perf.ReceiptError as error:
                reason = f"generator provenance validation failed: {error}"
                provenance["status"] = "invalid"
                provenance["reason"] = reason
        write_new(provenance_path, host_perf.canonical(provenance) + b"\n")
        if reason is not None:
            raise host_perf.ReceiptError(reason)
        print(
            f"STRUCTURALLY_VALID control=UNQUALIFIED raw={args.raw} "
            f"provenance={provenance_path} runner={executable_path} "
            f"topology={topology_path} resources={resource_path}"
        )
        return 0
    except (OSError, host_perf.ReceiptError) as error:
        print(f"generator run rejected: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
