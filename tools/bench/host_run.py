#!/usr/bin/env python3
"""Build and run the diagnostic host probe with execution-time provenance."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from collections.abc import Callable
from pathlib import Path
from typing import Any

import host_perf
from process_resource import sample_subprocess


def build_runner(
    features: list[str], example_name: str = "host_load_probe"
) -> tuple[Path, list[str], list[str]]:
    if example_name not in (
        "host_load_probe",
        "host_generator_probe",
        "host_closed_loop_probe",
        "host_local_probe",
        "host_composite_probe",
        "host_special_validate",
    ):
        raise host_perf.ReceiptError("unsupported benchmark runner")
    witness_home = os.environ.get("TASKMESH_BENCH_BUILD_WITNESSES")
    if witness_home:
        import host_build

        identity = {
            "source_dirty": bool(host_perf._git("status", "--porcelain=v1")),
            "source_head": host_perf._git("rev-parse", "HEAD"),
            "source_tree": host_perf._git("rev-parse", "HEAD^{tree}"),
            "lock_sha256": host_perf.sha256((host_perf.REPO / "Cargo.lock").read_bytes()),
        }
        return host_build.runner(
            Path(witness_home) / example_name, identity, features, example_name
        )
    command = [
        "cargo",
        "build",
        "--locked",
        "-p",
        "taskmesh-bench",
        "--example",
        example_name,
        "--message-format=json",
    ]
    if features:
        command.extend(["--features", ",".join(features)])
    result = subprocess.run(
        command, cwd=host_perf.REPO, capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        raise host_perf.ReceiptError(f"host runner build failed: {result.stderr[-2000:]}")
    artifacts: list[dict[str, Any]] = []
    for line in result.stdout.splitlines():
        try:
            item = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (
            item.get("reason") == "compiler-artifact"
            and item.get("target", {}).get("name") == example_name
            and "example" in item.get("target", {}).get("kind", [])
            and item.get("executable")
        ):
            artifacts.append(item)
    if len(artifacts) != 1:
        raise host_perf.ReceiptError("cargo did not identify exactly one host runner binary")
    binary = Path(artifacts[0]["executable"])
    if not binary.is_file():
        raise host_perf.ReceiptError("built host runner binary is missing")
    return binary, sorted(artifacts[0]["features"]), command


def write_new(path: Path, data: bytes) -> None:
    if path.exists():
        raise host_perf.ReceiptError(f"refusing to overwrite {path}")
    descriptor, temporary_name = tempfile.mkstemp(prefix=f"{path.name}.tmp-", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(data)
        # Linking publishes complete bytes atomically and never replaces an
        # artifact created by another writer after the preflight check.
        try:
            os.link(temporary, path)
        except FileExistsError as error:
            raise host_perf.ReceiptError(f"refusing to overwrite {path}") from error
    finally:
        temporary.unlink(missing_ok=True)


def retain_executable(source: Path, destination: Path) -> bytes:
    if destination.exists():
        raise host_perf.ReceiptError(f"refusing to overwrite {destination}")
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f"{destination.name}.tmp-", dir=destination.parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as output_stream:
            with source.open("rb") as input_stream:
                shutil.copyfileobj(input_stream, output_stream)
        temporary.chmod(source.stat().st_mode & 0o777)
        data = temporary.read_bytes()
        try:
            os.link(temporary, destination)
        except FileExistsError as error:
            raise host_perf.ReceiptError(f"refusing to overwrite {destination}") from error
        return data
    finally:
        temporary.unlink(missing_ok=True)


def retain_control_bundle(
    path: Path, bundle: dict[str, Any], verify: Callable[[bytes], Any], label: str
) -> None:
    """Publish a completed bundle only after final cross-run verification."""
    if not bundle["failures"]:
        try:
            verify(host_perf.canonical(bundle) + b"\n")
        except (OSError, host_perf.ReceiptError) as error:
            bundle["failures"].append(
                {
                    "index": len(bundle["runs"]),
                    "phase": "bundle_validation",
                    "reason": str(error),
                }
            )
    bundle["status"] = "incomplete" if bundle["failures"] else "diagnostic_complete"
    write_new(path, host_perf.canonical(bundle) + b"\n")
    if bundle["failures"]:
        raise host_perf.ReceiptError(f"{label} acquisition incomplete: {bundle['failures'][0]}")


def retain_rejection(path: Path, phase: str, error: Exception) -> None:
    """Keep acquisition failures distinct from complete execution provenance."""
    try:
        write_new(
            path,
            host_perf.canonical(
                {
                    "schema_version": 1,
                    "status": "rejected",
                    "performance": "UNQUALIFIED",
                    "phase": phase,
                    "reason": str(error),
                }
            )
            + b"\n",
        )
    except (OSError, host_perf.ReceiptError) as retention_error:
        print(f"acquisition rejection could not be retained: {retention_error}", file=sys.stderr)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scenario", type=Path)
    parser.add_argument("raw", type=Path)
    parser.add_argument("summary", type=Path)
    parser.add_argument("calibration", type=Path)
    parser.add_argument("--feature", action="append", default=[])
    parser.add_argument("--no-resource-sampling", action="store_true")
    args = parser.parse_args()
    provenance_path = args.raw.with_name(args.raw.name + ".provenance.json")
    executable_path = args.raw.with_name(args.raw.name + ".runner")
    topology_path = args.raw.with_name(args.raw.name + ".topology.json")
    resource_path = args.raw.with_name(args.raw.name + ".resources.json")
    rejection_path = args.raw.with_name(args.raw.name + ".rejection.json")
    owns_artifacts = False
    phase = "preflight"
    try:
        if any(
            path.exists()
            for path in (
                args.raw,
                args.summary,
                provenance_path,
                executable_path,
                topology_path,
                resource_path,
                rejection_path,
            )
        ):
            raise host_perf.ReceiptError(
                "raw, summary, provenance, runner, topology and resource paths must be fresh"
            )
        owns_artifacts = True
        scenario_bytes = args.scenario.read_bytes()
        scenario = host_perf.parse_object(scenario_bytes, "scenario")
        calibration = host_perf.parse_object(args.calibration.read_bytes(), "calibration")
        features = sorted(set(args.feature))
        build_start_identity = host_perf.local_identity(scenario, features)
        phase = "build"
        binary, artifact_features, build_command = build_runner(features)
        if host_perf.local_identity(scenario, features) != build_start_identity:
            raise host_perf.ReceiptError("source, toolchain or host identity changed during build")
        # Run a private copy so another Cargo build cannot replace the measured
        # executable between hashing it and process creation.
        with tempfile.TemporaryDirectory(prefix="taskmesh-host-binary-") as binary_dir:
            sealed_binary = Path(binary_dir) / binary.name
            sealed_scenario = Path(binary_dir) / "scenario.json"
            shutil.copy2(binary, sealed_binary)
            sealed_scenario.write_bytes(scenario_bytes)
            binary_digest = host_perf.sha256(sealed_binary.read_bytes())
            executable_bytes = retain_executable(sealed_binary, executable_path)
            if host_perf.sha256(executable_bytes) != binary_digest:
                raise host_perf.ReceiptError("retained runner differs from sealed executable")
            start_identity = host_perf.local_identity(scenario, features)
            phase = "before_launch"
            if start_identity != build_start_identity:
                raise host_perf.ReceiptError(
                    "source, toolchain or host identity changed between build and launch"
                )
            phase = "run"
            runner_exit_code, runner_pid, runner_stderr, resources = sample_subprocess(
                [
                    str(sealed_binary),
                    str(sealed_scenario),
                    str(args.raw),
                    "--topology-out",
                    str(topology_path),
                ],
                cwd=host_perf.REPO,
                sample_resources=not args.no_resource_sampling,
            )
            resource_bytes = host_perf.canonical(resources) + b"\n"
            write_new(resource_path, resource_bytes)
            end_identity = host_perf.local_identity(scenario, features)
            binary_unchanged = host_perf.sha256(sealed_binary.read_bytes()) == binary_digest
        raw_bytes = args.raw.read_bytes() if args.raw.exists() else None
        topology_bytes = topology_path.read_bytes() if topology_path.exists() else None
        raw_valid = False
        raw_problem = None
        if raw_bytes is not None:
            try:
                raw = host_perf.parse_object(raw_bytes, "raw")
                raw_valid = raw.get("status") == {"kind": "complete"}
            except host_perf.ReceiptError as error:
                raw_problem = str(error)
        reason = None
        if start_identity != end_identity:
            reason = "source, toolchain or host identity changed during run"
        elif (
            not binary_unchanged or host_perf.sha256(executable_path.read_bytes()) != binary_digest
        ):
            reason = "sealed or retained executable changed during run"
        elif runner_exit_code != 0:
            reason = f"runner exited {runner_exit_code}: {runner_stderr[-1000:]}"
        elif raw_bytes is None:
            reason = "runner did not write raw artifact"
        elif topology_bytes is None:
            reason = "runner did not write resolved topology artifact"
        elif not raw_valid:
            reason = f"runner wrote invalid raw artifact: {raw_problem or 'status is not complete'}"
        provenance = {
            "schema_version": 4,
            "status": "invalid" if reason else "complete",
            "reason": reason,
            "scenario_sha256": host_perf.sha256(scenario_bytes),
            "raw_sha256": host_perf.sha256(raw_bytes) if raw_bytes is not None else None,
            "binary_sha256": binary_digest,
            "binary_artifact": executable_path.name,
            "topology_sha256": host_perf.sha256(topology_bytes) if topology_bytes else None,
            "topology_artifact": topology_path.name,
            "resources_sha256": host_perf.sha256(resource_bytes),
            "resources_artifact": resource_path.name,
            "runner_pid": runner_pid,
            "runner_mode": "full",
            "runner_flags": [],
            "build_command": build_command,
            "build_artifact_features": artifact_features,
            "start_identity": start_identity,
            "end_identity": end_identity,
            "runner_exit_code": runner_exit_code,
        }
        if reason is None:
            assert raw_bytes is not None
            provisional = host_perf.canonical(provenance) + b"\n"
            try:
                summary = host_perf.make_summary(
                    raw_bytes,
                    scenario_bytes,
                    end_identity,
                    calibration,
                    10_000,
                    provisional,
                    executable_bytes,
                    topology_bytes,
                    resource_bytes,
                )
            except host_perf.ReceiptError as error:
                provenance["status"] = "invalid"
                provenance["reason"] = f"raw analysis rejected: {error}"
            if provenance["status"] == "complete":
                if host_perf.local_identity(scenario, features) != end_identity:
                    provenance["status"] = "invalid"
                    provenance["reason"] = (
                        "source, toolchain or host identity changed during analysis"
                    )
                else:
                    write_new(provenance_path, provisional)
                    write_new(
                        args.summary, json.dumps(summary, indent=2, sort_keys=True).encode() + b"\n"
                    )
                    print(
                        f"STRUCTURALLY_VALID performance=UNQUALIFIED raw={args.raw} "
                        f"provenance={provenance_path} runner={executable_path} "
                        f"topology={topology_path} "
                        f"resources={resource_path} "
                        f"summary={args.summary}"
                    )
                    return 0
        write_new(provenance_path, host_perf.canonical(provenance) + b"\n")
        raise host_perf.ReceiptError(provenance["reason"] or "host run was invalid")
    except (OSError, host_perf.ReceiptError) as error:
        if owns_artifacts:
            retain_rejection(rejection_path, phase, error)
        print(f"host run rejected: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
