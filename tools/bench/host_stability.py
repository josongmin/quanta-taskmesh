#!/usr/bin/env python3
"""Same-host recovery diagnostics; functional evidence never admits performance."""

from __future__ import annotations

import argparse
import math
import tempfile
from pathlib import Path

import host_perf
import host_run
from acquisition_process import run_acquisition
from host_special_run import source_content_sha256
from process_resource import sample_subprocess

from tools.inspection import read_regular_bytes

BASE_ARTIFACTS = {"manifest.json", "runner", "summary.json", "resources.json", "stderr.txt"}
RECEIPT_KEYS = {
    "schema_version",
    "status",
    "reason",
    "performance_status",
    "runner_pid",
    "runner_exit_code",
    "features",
    "build_artifact_features",
    "build_command",
    "start_identity",
    "end_identity",
    "source_content_sha256",
    "artifacts",
    "cadence_ms",
    "timeout_seconds",
}


def manifest_object(data: bytes) -> dict:
    manifest = host_perf.parse_object(data, "stability manifest")
    host_perf.exact_keys(
        manifest,
        {
            "schema_version",
            "scenario",
            "cycles",
            "min_duration_ms",
            "max_total_records",
        },
        "stability manifest",
    )
    for key in ("schema_version", "cycles", "min_duration_ms", "max_total_records"):
        host_perf.nat(manifest[key], key)
    if (
        manifest["schema_version"] != 1
        or not 1 <= manifest["cycles"] <= 10_000
        or manifest["min_duration_ms"] > 1_200_000
        or not 1 <= manifest["max_total_records"] <= 1_000_000
        or not isinstance(manifest["scenario"], dict)
    ):
        raise host_perf.ReceiptError("invalid stability manifest")
    return manifest


def cycle_names(count: int) -> set[str]:
    return {f"cycles/cycle-{index:06}.json" for index in range(count)}


def population(directory: Path, count: int) -> set[str]:
    cycles = directory / "cycles"
    if cycles.is_symlink() or not cycles.is_dir():
        raise host_perf.ReceiptError("regular cycle directory required")
    names = {f"cycles/{path.name}" for path in cycles.iterdir()}
    if names != cycle_names(count):
        raise host_perf.ReceiptError("missing or extra cycle artifacts")
    if {path.name for path in directory.iterdir()} != BASE_ARTIFACTS | {"cycles", "receipt.json"}:
        raise host_perf.ReceiptError("unexpected stability bundle population")
    return BASE_ARTIFACTS | names


def typed_validate(directory: Path, artifacts: dict[str, str]) -> None:
    # Read each regular descriptor once, hash those bytes, then execute and
    # validate private copies. Original paths are never executed after hashing.
    with tempfile.TemporaryDirectory(prefix="taskmesh-stability-validate-") as temporary:
        sealed = Path(temporary)
        (sealed / "cycles").mkdir()
        for name, digest in artifacts.items():
            data = read_regular_bytes(directory / name)
            if host_perf.sha256(data) != digest:
                raise host_perf.ReceiptError(f"stability artifact digest differs: {name}")
            host_run.write_new(sealed / name, data)
        _validate_private_bundle(sealed)


def _validate_private_bundle(directory: Path) -> None:
    """Replay only a private bundle whose bytes were checked during copying."""
    (directory / "runner").chmod(0o700)
    result = run_acquisition(
        [str(directory / "runner"), "validate", str(directory / "manifest.json"), str(directory)],
        cwd=host_perf.REPO,
        timeout_seconds=120,
    )
    if (
        result.returncode != 0
        or result.timed_out
        or result.interrupted_by_signal is not None
        or result.aborted_early
    ):
        raise host_perf.ReceiptError("typed stability validation failed: " + result.stderr[-2000:])


def _verify_sealed(directory: Path, *, require_current_source: bool = False) -> dict:
    receipt = host_perf.parse_object(
        read_regular_bytes(directory / "receipt.json"), "stability receipt"
    )
    host_perf.exact_keys(receipt, RECEIPT_KEYS, "stability receipt")
    if (
        type(receipt["schema_version"]) is not int
        or receipt["schema_version"] != 1
        or receipt["status"] != "complete"
        or receipt["reason"] is not None
        or receipt["performance_status"] != "UNQUALIFIED"
        or type(receipt["runner_exit_code"]) is not int
        or receipt["runner_exit_code"] != 0
    ):
        raise host_perf.ReceiptError("incomplete or invalid stability receipt")
    for name in ("start_identity", "end_identity"):
        host_perf.validate_identity(receipt[name])
    features = receipt["features"]
    if (
        features != []
        or receipt["build_artifact_features"] != ["default"]
        or receipt["start_identity"] != receipt["end_identity"]
        or receipt["start_identity"]["features"] != features
    ):
        raise host_perf.ReceiptError("stability identity or default feature scope differs")
    source = receipt["source_content_sha256"]
    if (
        not isinstance(source, str)
        or len(source) != 64
        or any(c not in "0123456789abcdef" for c in source)
    ):
        raise host_perf.ReceiptError("invalid stability source digest")
    if (
        type(receipt["cadence_ms"]) is not int
        or receipt["cadence_ms"] <= 0
        or type(receipt["timeout_seconds"]) not in (int, float)
        or not math.isfinite(receipt["timeout_seconds"])
        or receipt["timeout_seconds"] <= 0
    ):
        raise host_perf.ReceiptError("invalid stability controller budget")
    command = receipt["build_command"]
    if (
        not isinstance(command, list)
        or not all(isinstance(arg, str) for arg in command)
        or "host_stability_probe" not in command
    ):
        raise host_perf.ReceiptError("stability runner build command differs")
    manifest_data = read_regular_bytes(directory / "manifest.json")
    manifest = manifest_object(manifest_data)
    names = population(directory, manifest["cycles"])
    artifacts = receipt["artifacts"]
    if not isinstance(artifacts, dict) or set(artifacts) != names:
        raise host_perf.ReceiptError("stability artifact inventory differs")
    # verify_receipt checked every digest while creating this private bundle.
    scenario = manifest["scenario"]
    if receipt["start_identity"]["topology_fingerprint"] != host_perf.sha256(
        host_perf.canonical(scenario["topology"])
    ):
        raise host_perf.ReceiptError("stability declared topology differs")
    summary = host_perf.parse_object(
        read_regular_bytes(directory / "summary.json"), "stability summary"
    )
    pid = host_perf.nat(receipt["runner_pid"], "runner PID")
    if pid == 0 or summary.get("pid") != pid:
        raise host_perf.ReceiptError("stability runner PID differs")
    resources = host_perf.validate_resource_artifact(
        read_regular_bytes(directory / "resources.json"), pid
    )
    if resources["cadence_ms"] != receipt["cadence_ms"]:
        raise host_perf.ReceiptError("stability resource cadence differs")
    if require_current_source and (
        source_content_sha256() != source
        or host_perf.local_identity(scenario, []) != receipt["start_identity"]
    ):
        raise host_perf.ReceiptError("current stability source or host differs")
    _validate_private_bundle(directory)
    return receipt


def verify_receipt(directory: Path, *, require_current_source: bool = False) -> dict:
    receipt_data = read_regular_bytes(directory / "receipt.json")
    receipt = host_perf.parse_object(receipt_data, "stability receipt")
    host_perf.exact_keys(receipt, RECEIPT_KEYS, "stability receipt")
    manifest = manifest_object(read_regular_bytes(directory / "manifest.json"))
    names = population(directory, manifest["cycles"])
    artifacts = receipt["artifacts"]
    if not isinstance(artifacts, dict) or set(artifacts) != names:
        raise host_perf.ReceiptError("stability artifact inventory differs")
    with tempfile.TemporaryDirectory(prefix="taskmesh-stability-bundle-") as temporary:
        sealed = Path(temporary)
        (sealed / "cycles").mkdir()
        host_run.write_new(sealed / "receipt.json", receipt_data)
        for name in sorted(names):
            data = read_regular_bytes(directory / name)
            if host_perf.sha256(data) != artifacts[name]:
                raise host_perf.ReceiptError(f"stability artifact digest differs: {name}")
            host_run.write_new(sealed / name, data)
        return _verify_sealed(sealed, require_current_source=require_current_source)


def acquire(
    manifest_path: Path, directory: Path, *, cadence_ms: int = 250, timeout_seconds: float = 900
) -> dict:
    if (
        type(cadence_ms) is not int
        or cadence_ms <= 0
        or type(timeout_seconds) not in (int, float)
        or not math.isfinite(timeout_seconds)
        or timeout_seconds <= 0
    ):
        raise host_perf.ReceiptError("positive finite controller budgets required")
    if (
        directory.exists()
        or directory.is_symlink()
        or directory.resolve().is_relative_to(host_perf.REPO)
    ):
        raise host_perf.ReceiptError("new output directory outside checkout required")
    data = read_regular_bytes(manifest_path)
    manifest = manifest_object(data)
    directory.mkdir(parents=True)
    # Only a successful exclusive mkdir grants ownership of failure artifacts.
    # In particular, a rejected attempt must not mutate an existing bundle.
    try:
        return _acquire_owned(
            data, manifest, directory, cadence_ms=cadence_ms, timeout_seconds=timeout_seconds
        )
    except (OSError, ValueError, KeyError, host_perf.ReceiptError) as error:
        host_run.retain_rejection(directory / "rejection.json", "stability acquisition", error)
        raise


def _acquire_owned(
    data: bytes, manifest: dict, directory: Path, *, cadence_ms: int, timeout_seconds: float
) -> dict:
    scenario = manifest["scenario"]
    host_run.write_new(directory / "manifest.json", data)
    start = host_perf.local_identity(scenario, [])
    source = source_content_sha256()
    binary, features, command = host_run.build_runner([], "host_stability_probe")
    if (
        features != ["default"]
        or start != host_perf.local_identity(scenario, [])
        or source != source_content_sha256()
    ):
        raise host_perf.ReceiptError("stability source or build features changed")
    with tempfile.TemporaryDirectory(prefix="taskmesh-stability-run-") as temporary:
        sealed = Path(temporary) / "runner"
        binary_data = host_run.retain_executable(binary, directory / "runner")
        host_run.write_new(sealed, binary_data)
        sealed.chmod(0o700)
        sealed_manifest = Path(temporary) / "manifest.json"
        host_run.write_new(sealed_manifest, data)
        checked = run_acquisition(
            [str(sealed), "check", str(sealed_manifest)],
            cwd=host_perf.REPO,
            timeout_seconds=120,
        )
        if (
            checked.returncode != 0
            or checked.timed_out
            or checked.interrupted_by_signal is not None
            or checked.aborted_early
        ):
            raise host_perf.ReceiptError(
                "invalid typed stability manifest: " + checked.stderr[-2000:]
            )
        if start != host_perf.local_identity(scenario, []) or source != source_content_sha256():
            raise host_perf.ReceiptError("stability source changed before launch")
        code, pid, stderr, resources = sample_subprocess(
            [str(sealed), "run", str(sealed_manifest), str(directory)],
            cwd=host_perf.REPO,
            cadence_ms=cadence_ms,
            timeout_seconds=timeout_seconds,
        )
        sealed_unchanged = (
            read_regular_bytes(sealed) == binary_data
            and read_regular_bytes(directory / "runner") == binary_data
            and read_regular_bytes(directory / "manifest.json") == data
        )
    host_run.write_new(directory / "stderr.txt", stderr.encode())
    host_run.write_new(directory / "resources.json", host_perf.canonical(resources))
    end = host_perf.local_identity(scenario, [])
    reason = None
    if end != start or source_content_sha256() != source:
        reason = "source or host identity changed during stability execution"
    elif not sealed_unchanged:
        reason = "stability sealed runner changed"
    elif code != 0:
        reason = "stability execution failed: " + stderr[-2000:]
    names = {name for name in BASE_ARTIFACTS if (directory / name).exists()}
    if (directory / "cycles").is_dir():
        names |= {f"cycles/{path.name}" for path in (directory / "cycles").iterdir()}
    artifacts = {
        name: host_perf.sha256(read_regular_bytes(directory / name)) for name in sorted(names)
    }
    if reason is None:
        try:
            if names != BASE_ARTIFACTS | cycle_names(manifest["cycles"]):
                raise host_perf.ReceiptError("incomplete stability artifact population")
            summary = host_perf.parse_object(
                read_regular_bytes(directory / "summary.json"), "summary"
            )
            if summary.get("pid") != pid:
                raise host_perf.ReceiptError("stability PID differs")
            host_perf.validate_resource_artifact(
                read_regular_bytes(directory / "resources.json"), pid
            )
            typed_validate(directory, artifacts)
        except (OSError, ValueError, host_perf.ReceiptError) as error:
            reason = str(error)
    receipt = {
        "schema_version": 1,
        "status": "invalid" if reason else "complete",
        "reason": reason,
        "performance_status": "UNQUALIFIED",
        "runner_pid": pid,
        "runner_exit_code": code,
        "features": [],
        "build_artifact_features": features,
        "build_command": command,
        "start_identity": start,
        "end_identity": end,
        "source_content_sha256": source,
        "artifacts": artifacts,
        "cadence_ms": cadence_ms,
        "timeout_seconds": timeout_seconds,
    }
    host_run.write_new(directory / "receipt.json", host_perf.canonical(receipt))
    if reason:
        raise host_perf.ReceiptError(reason)
    return verify_receipt(directory, require_current_source=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    run = sub.add_parser("run")
    run.add_argument("manifest", type=Path)
    run.add_argument("directory", type=Path)
    run.add_argument("--cadence-ms", type=int, default=250)
    run.add_argument("--timeout-seconds", type=float, default=900)
    verify = sub.add_parser("verify")
    verify.add_argument("directory", type=Path)
    verify.add_argument("--require-current-source", action="store_true")
    args = parser.parse_args()
    try:
        if args.command == "run":
            receipt = acquire(
                args.manifest,
                args.directory,
                cadence_ms=args.cadence_ms,
                timeout_seconds=args.timeout_seconds,
            )
        else:
            receipt = verify_receipt(
                args.directory, require_current_source=args.require_current_source
            )
        print(
            host_perf.canonical(
                {"functional": "PASS", "performance": receipt["performance_status"]}
            ).decode()
        )
        return 0
    except (OSError, ValueError, KeyError, host_perf.ReceiptError) as error:
        print(f"stability rejected: {error}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
