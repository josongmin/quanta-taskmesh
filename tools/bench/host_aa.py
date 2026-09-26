#!/usr/bin/env python3
"""Acquire and verify repeated same-binary public-host A/A diagnostic runs."""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path
from typing import Any

import host_perf
from acquisition_process import deadline, run_acquisition
from host_run import retain_control_bundle, write_new

BUNDLE_VERSION = 2
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


def validate_timeout(timeout_seconds: float) -> None:
    try:
        deadline(timeout_seconds)
    except ValueError as error:
        raise host_perf.ReceiptError(
            "control timeout_seconds must be positive and finite"
        ) from error


def execution_paths(directory: Path, index: int) -> dict[str, Path]:
    return {
        name: directory / f"run-{index:03d}.{name}"
        for name in ("stdout", "stderr", "execution.json")
    }


def execute_arm(
    directory: Path,
    index: int,
    command: list[str],
    timeout_seconds: float,
    *,
    not_launched: str | None = None,
) -> tuple[dict, str | None]:
    """Retain every planned terminal, including the arms stopped after failure."""
    stdout, stderr, code, timed_out, signum, aborted = "", "", None, False, None, False
    reason = not_launched
    if reason is None:
        try:
            process = run_acquisition(command, cwd=host_perf.REPO, timeout_seconds=timeout_seconds)
            stdout, stderr = process.stdout, process.stderr
            code, timed_out = process.returncode, process.timed_out
            signum, aborted = process.interrupted_by_signal, process.aborted_early
            if timed_out:
                reason = f"control timeout after {timeout_seconds}s"
            elif signum is not None:
                reason = f"control interrupted by signal {signum}"
            elif aborted or code != 0:
                reason = (
                    f"control execution failed: exit={code}, aborted={aborted}: {stderr[-1000:]}"
                )
        except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
            reason = f"control launch failed: {error}"
    status = "not_launched" if not_launched else "failed" if reason else "completed"
    paths = execution_paths(directory, index)
    out, err = stdout.encode(), stderr.encode()
    write_new(paths["stdout"], out)
    write_new(paths["stderr"], err)
    body = (
        host_perf.canonical(
            {
                "schema_version": 1,
                "index": index,
                "status": status,
                "reason": reason,
                "command": command,
                "timeout_seconds": timeout_seconds,
                "returncode": code,
                "timed_out": timed_out,
                "interrupted_by_signal": signum,
                "aborted_early": aborted,
                "stdout_sha256": host_perf.sha256(out),
                "stderr_sha256": host_perf.sha256(err),
            }
        )
        + b"\n"
    )
    write_new(paths["execution.json"], body)
    return {"index": index, "status": status, "execution_sha256": host_perf.sha256(body)}, reason


def validate_executions(bundle: dict, directory: Path, runs: list) -> None:
    validate_timeout(bundle["timeout_seconds"])
    expected = bundle["expected_runs"]
    if type(expected) is not int or expected != len(runs):
        raise host_perf.ReceiptError("control execution population differs from planned runs")
    executions = bundle["executions"]
    if not isinstance(executions, list) or len(executions) != expected:
        raise host_perf.ReceiptError("control execution population is incomplete")
    for index, recorded in enumerate(executions):
        if not isinstance(recorded, dict):
            raise host_perf.ReceiptError("control execution reference must be an object")
        host_perf.exact_keys(
            recorded, {"index", "status", "execution_sha256"}, "execution reference"
        )
        paths = execution_paths(directory, index)
        if any(path.is_symlink() or not path.is_file() for path in paths.values()):
            raise host_perf.ReceiptError("control execution evidence requires regular owned files")
        data = paths["execution.json"].read_bytes()
        row = host_perf.parse_object(data, "control execution")
        host_perf.exact_keys(
            row,
            {
                "schema_version",
                "index",
                "status",
                "reason",
                "command",
                "timeout_seconds",
                "returncode",
                "timed_out",
                "interrupted_by_signal",
                "aborted_early",
                "stdout_sha256",
                "stderr_sha256",
            },
            "control execution",
        )
        validate_timeout(row["timeout_seconds"])
        if (
            type(recorded["index"]) is not int
            or recorded
            != {"index": index, "status": "completed", "execution_sha256": host_perf.sha256(data)}
            or type(row["schema_version"]) is not int
            or row["schema_version"] != 1
            or type(row["index"]) is not int
            or row["index"] != index
            or row["status"] != "completed"
            or row["reason"] is not None
            or type(row["returncode"]) is not int
            or row["returncode"] != 0
            or row["timed_out"] is not False
            or row["aborted_early"] is not False
            or row["interrupted_by_signal"] is not None
            or row["timeout_seconds"] != bundle["timeout_seconds"]
        ):
            raise host_perf.ReceiptError("control execution terminal differs or is unsuccessful")
        command = row["command"]
        if (
            not isinstance(command, list)
            or len(command) < 4
            or any(not isinstance(arg, str) or not arg for arg in command)
            or Path(command[1]).name
            != ("minimal_run.py" if runs[index].get("mode") == "minimal" else "host_run.py")
            or Path(command[3]).resolve() != paths_for_raw(directory, index).resolve()
        ):
            raise host_perf.ReceiptError("control execution command differs from indexed arm")
        for name in ("stdout", "stderr"):
            if host_perf.sha256(paths[name].read_bytes()) != row[f"{name}_sha256"]:
                raise host_perf.ReceiptError("control execution capture changed")


def paths_for_raw(directory: Path, index: int) -> Path:
    return paths(directory, index)["raw"]


def account_unlaunched(
    directory: Path, executions: list, failures: list, expected: int, timeout_seconds: float
) -> None:
    for index in range(len(executions), expected):
        reason = f"not launched after failed arm {failures[0]['index']}"
        terminal, _ = execute_arm(directory, index, [], timeout_seconds, not_launched=reason)
        executions.append(terminal)
        failures.append({"index": index, "reason": reason})


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
            "expected_runs",
            "timeout_seconds",
            "executions",
        },
        "A/A bundle",
    )
    if (
        type(bundle["schema_version"]) is not int
        or bundle["schema_version"] != BUNDLE_VERSION
        or bundle["kind"] != "same_binary_aa"
    ):
        raise host_perf.ReceiptError("unsupported A/A bundle")
    if bundle["status"] != "diagnostic_complete" or bundle["failures"] != []:
        raise host_perf.ReceiptError("A/A bundle is incomplete")
    if bundle["scenario_sha256"] != host_perf.sha256(scenario_bytes):
        raise host_perf.ReceiptError("A/A scenario digest differs")
    runs = bundle["runs"]
    if not isinstance(runs, list) or len(runs) < 2 or len(runs) % 2:
        raise host_perf.ReceiptError("A/A run population must contain complete pairs")
    validate_executions(bundle, directory, runs)
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
    *,
    timeout_seconds: float = 1800,
) -> dict[str, Any]:
    validate_timeout(timeout_seconds)
    if pairs < 1 or pairs > 50:
        raise host_perf.ReceiptError("A/A pairs must be 1..=50")
    directory.mkdir(parents=True, exist_ok=False)
    scenario_bytes = scenario.read_bytes()
    host_perf.parse_object(scenario_bytes, "scenario")
    runs = []
    executions = []
    failures = []
    common_identity = None
    common_binary = None
    for index in range(pairs * 2):
        artifact = paths(directory, index)
        terminal, reason = execute_arm(
            directory,
            index,
            [
                sys.executable,
                str(Path(__file__).with_name("host_run.py")),
                str(scenario),
                str(artifact["raw"]),
                str(artifact["summary"]),
                str(calibration),
                *(part for feature in features for part in ("--feature", feature)),
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
            failures.append({"index": index, "reason": "A/A source, host or binary changed"})
            break
        runs.append(row)
    account_unlaunched(directory, executions, failures, pairs * 2, timeout_seconds)
    bundle = {
        "schema_version": BUNDLE_VERSION,
        "kind": "same_binary_aa",
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
            f"AA_DIAGNOSTIC performance=UNQUALIFIED runs={len(bundle['runs'])} "
            f"bundle={args.directory / 'aa-bundle.json'}"
        )
        return 0
    except (OSError, host_perf.ReceiptError, ValueError) as error:
        print(f"A/A rejected: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
