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
from pathlib import Path
from typing import Any

import host_perf


def build_runner(features: list[str]) -> tuple[Path, list[str], list[str]]:
    command = [
        "cargo",
        "build",
        "--locked",
        "-p",
        "taskmesh-bench",
        "--example",
        "host_load_probe",
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
            and item.get("target", {}).get("name") == "host_load_probe"
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
    temporary = path.with_name(f"{path.name}.tmp-{os.getpid()}")
    try:
        with temporary.open("xb") as stream:
            stream.write(data)
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("scenario", type=Path)
    parser.add_argument("raw", type=Path)
    parser.add_argument("summary", type=Path)
    parser.add_argument("calibration", type=Path)
    parser.add_argument("--feature", action="append", default=[])
    args = parser.parse_args()
    provenance_path = args.raw.with_name(args.raw.name + ".provenance.json")
    try:
        if any(path.exists() for path in (args.raw, args.summary, provenance_path)):
            raise host_perf.ReceiptError("raw, summary and provenance paths must be fresh")
        scenario_bytes = args.scenario.read_bytes()
        scenario = host_perf.parse_object(scenario_bytes, "scenario")
        calibration = host_perf.parse_object(args.calibration.read_bytes(), "calibration")
        features = sorted(set(args.feature))
        build_start_identity = host_perf.local_identity(scenario, features)
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
            start_identity = host_perf.local_identity(scenario, features)
            runner = subprocess.run(
                [str(sealed_binary), str(sealed_scenario), str(args.raw)],
                cwd=host_perf.REPO,
                capture_output=True,
                text=True,
                check=False,
            )
            end_identity = host_perf.local_identity(scenario, features)
        raw_bytes = args.raw.read_bytes() if args.raw.exists() else None
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
        elif runner.returncode != 0:
            reason = f"runner exited {runner.returncode}: {runner.stderr[-1000:]}"
        elif raw_bytes is None:
            reason = "runner did not write raw artifact"
        elif not raw_valid:
            reason = f"runner wrote invalid raw artifact: {raw_problem or 'status is not complete'}"
        provenance = {
            "schema_version": 1,
            "status": "invalid" if reason else "complete",
            "reason": reason,
            "scenario_sha256": host_perf.sha256(scenario_bytes),
            "raw_sha256": host_perf.sha256(raw_bytes) if raw_bytes is not None else None,
            "binary_sha256": binary_digest,
            "build_command": build_command,
            "build_artifact_features": artifact_features,
            "start_identity": start_identity,
            "end_identity": end_identity,
            "runner_exit_code": runner.returncode,
        }
        if reason is None:
            assert raw_bytes is not None
            provisional = host_perf.canonical(provenance) + b"\n"
            try:
                summary = host_perf.make_summary(
                    raw_bytes, scenario_bytes, end_identity, calibration, 10_000, provisional
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
                        f"provenance={provenance_path} summary={args.summary}"
                    )
                    return 0
        write_new(provenance_path, host_perf.canonical(provenance) + b"\n")
        raise host_perf.ReceiptError(provenance["reason"] or "host run was invalid")
    except (OSError, host_perf.ReceiptError) as error:
        print(f"host run rejected: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
