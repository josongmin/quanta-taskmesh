#!/usr/bin/env python3
# ruff: noqa: UP045 -- pyproject supports Python 3.9, which lacks PEP 604 unions.
"""Acquire a diagnostic control with retained execution bytes.

The default entry point is generator-only. The minimal host recorder reuses
this exact artifact custody path with a different typed Rust raw validator.
"""

from __future__ import annotations

import argparse
import shutil
import sys
import tempfile
from pathlib import Path
from typing import Optional

import host_perf
from host_run import (
    add_timeout_arguments,
    build_runner,
    execution_artifacts,
    probe_failed,
    retain_executable,
    retain_rejection,
    write_new,
)
from process_resource import sample_subprocess
from scenario_rate import doubled_generator_scenario


def main(
    *,
    example_name: str = "host_generator_probe",
    raw_kind: str = "generator",
    runner_flag: Optional[str] = None,
) -> int:
    if (example_name, raw_kind, runner_flag) not in (
        ("host_generator_probe", "generator", None),
        ("host_load_probe", "minimal", "--recorder-minimal"),
    ):
        raise ValueError("unsupported diagnostic control")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scenario", type=Path)
    parser.add_argument("raw", type=Path)
    parser.add_argument("--feature", action="append", default=[])
    parser.add_argument("--rate-factor", type=int, choices=(1, 2), default=1)
    add_timeout_arguments(parser)
    args = parser.parse_args()
    provenance_path = args.raw.with_name(args.raw.name + ".provenance.json")
    executable_path = args.raw.with_name(args.raw.name + ".runner")
    topology_path = args.raw.with_name(args.raw.name + ".topology.json")
    resource_path = args.raw.with_name(args.raw.name + ".resources.json")
    scenario_artifact_path = args.raw.with_name(args.raw.name + ".scenario.json")
    rejection_path = args.raw.with_name(args.raw.name + ".rejection.json")
    build_execution = args.raw.with_name(args.raw.name + ".build")
    validator_execution = args.raw.with_name(args.raw.name + ".validation")
    probe_stderr_path = args.raw.with_name(args.raw.name + ".probe.stderr")
    owns_artifacts = False
    phase = "preflight"
    try:
        if raw_kind != "generator" and args.rate_factor != 1:
            raise host_perf.ReceiptError("rate factor applies only to generator control")
        if any(
            path.exists()
            for path in (
                args.raw,
                provenance_path,
                executable_path,
                topology_path,
                resource_path,
                scenario_artifact_path,
                rejection_path,
                *execution_artifacts(build_execution),
                *execution_artifacts(validator_execution),
                probe_stderr_path,
            )
        ):
            raise host_perf.ReceiptError("control artifact paths must be fresh")
        owns_artifacts = True
        scenario_bytes = args.scenario.read_bytes()
        scenario = host_perf.parse_object(scenario_bytes, "scenario")
        if args.rate_factor == 2:
            scenario = doubled_generator_scenario(scenario)
            scenario_bytes = host_perf.canonical(scenario) + b"\n"
        write_new(scenario_artifact_path, scenario_bytes)
        features = sorted(set(args.feature))
        build_start_identity = host_perf.local_identity(scenario, features)
        phase = "build"
        binary, artifact_features, build_command = build_runner(
            features,
            example_name,
            timeout_seconds=args.build_timeout_seconds,
            execution_path=build_execution,
        )
        if host_perf.local_identity(scenario, features) != build_start_identity:
            raise host_perf.ReceiptError("source, toolchain or host identity changed during build")
        with tempfile.TemporaryDirectory(prefix="taskmesh-control-binary-") as binary_dir:
            sealed_binary = Path(binary_dir) / binary.name
            sealed_scenario = Path(binary_dir) / "scenario.json"
            shutil.copy2(binary, sealed_binary)
            sealed_scenario.write_bytes(scenario_bytes)
            binary_digest = host_perf.sha256(sealed_binary.read_bytes())
            executable_bytes = retain_executable(sealed_binary, executable_path)
            if host_perf.sha256(executable_bytes) != binary_digest:
                raise host_perf.ReceiptError("retained control differs from sealed executable")
            start_identity = host_perf.local_identity(scenario, features)
            phase = "before_launch"
            if start_identity != build_start_identity:
                raise host_perf.ReceiptError(
                    "source, toolchain or host identity changed between build and launch"
                )
            runner_command = [
                str(sealed_binary),
                str(sealed_scenario),
                str(args.raw),
                "--topology-out",
                str(topology_path),
            ]
            if runner_flag is not None:
                runner_command.append(runner_flag)
            phase = "run"
            runner_exit_code, runner_pid, runner_stderr, resources = sample_subprocess(
                runner_command,
                cwd=host_perf.REPO,
                timeout_seconds=args.probe_timeout_seconds,
            )
            resource_bytes = host_perf.canonical(resources) + b"\n"
            write_new(probe_stderr_path, runner_stderr.encode())
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
            reason = "sealed or retained control changed during run"
        elif runner_exit_code != 0 or probe_failed(resources):
            reason = f"control exited {runner_exit_code}: {runner_stderr[-1000:]}"
        elif raw_bytes is None or topology_bytes is None:
            reason = "control did not write raw and topology artifacts"
        else:
            phase = "validation"
            try:
                host_perf.validate_with_rust(
                    scenario_bytes,
                    raw_bytes,
                    features,
                    topology_bytes,
                    raw_kind=raw_kind,
                    timeout_seconds=args.validator_timeout_seconds,
                    execution_prefix=validator_execution,
                )
            except host_perf.ReceiptError as error:
                reason = f"control raw validation failed: {error}"
        provenance = {
            "schema_version": host_perf.PROVENANCE_VERSION,
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
            "runner_mode": raw_kind,
            "runner_flags": [runner_flag] if runner_flag is not None else [],
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
                    example_name=example_name,
                    runner_mode=raw_kind,
                )
            except host_perf.ReceiptError as error:
                reason = f"control provenance validation failed: {error}"
                provenance["status"] = "invalid"
                provenance["reason"] = reason
        write_new(provenance_path, host_perf.canonical(provenance) + b"\n")
        if reason is not None:
            raise host_perf.ReceiptError(reason)
        print(
            f"STRUCTURALLY_VALID control={raw_kind} performance=UNQUALIFIED raw={args.raw} "
            f"provenance={provenance_path} runner={executable_path} "
            f"topology={topology_path} resources={resource_path} "
            f"scenario={scenario_artifact_path}"
        )
        return 0
    except (OSError, ValueError, host_perf.ReceiptError) as error:
        if owns_artifacts:
            retain_rejection(rejection_path, phase, error)
        print(f"{raw_kind} control rejected: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
