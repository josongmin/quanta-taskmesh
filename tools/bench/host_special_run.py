#!/usr/bin/env python3
"""Acquire a source-bound structural receipt for non-open-loop host diagnostics."""

from __future__ import annotations

import argparse
import hashlib
import shutil
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

import host_perf
import host_run
from process_resource import sample_subprocess

RUNNERS = {
    "closed_loop": "host_closed_loop_probe",
    "local": "host_local_probe",
    "composite": "host_composite_probe",
}


def source_content_sha256() -> str:
    """Bind even a dirty diagnostic to the files present in this checkout."""
    names = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=host_perf.REPO,
        capture_output=True,
        check=True,
    ).stdout.split(b"\0")
    digest = hashlib.sha256()
    for name in sorted(set(names) - {b""}):
        path = host_perf.REPO / name.decode("utf-8", "surrogateescape")
        digest.update(len(name).to_bytes(4, "big"))
        digest.update(name)
        digest.update(
            stat.S_IMODE(path.lstat().st_mode).to_bytes(2, "big")
            if path.exists() or path.is_symlink()
            else b"\0\0"
        )
        if path.is_symlink():
            content = path.readlink().as_posix().encode("utf-8", "surrogateescape")
        elif path.is_file():
            content = path.read_bytes()
        else:
            content = b"<missing>"
        digest.update(hashlib.sha256(content).digest())
    return digest.hexdigest()


def verify_receipt(directory: Path, *, require_current_source: bool = False) -> dict:
    receipt = host_perf.parse_object((directory / "receipt.json").read_bytes(), "special receipt")
    expected = {
        "schema_version",
        "status",
        "reason",
        "performance_status",
        "mode",
        "scenario_sha256",
        "raw_sha256",
        "topology_sha256",
        "runner_sha256",
        "validator_sha256",
        "resources_sha256",
        "runner_pid",
        "runner_exit_code",
        "validator_exit_code",
        "build_commands",
        "features",
        "build_artifact_features",
        "start_identity",
        "end_identity",
        "source_content_sha256",
    }
    host_perf.exact_keys(receipt, expected, "special receipt")
    if type(receipt["schema_version"]) is not int or receipt["schema_version"] != 1:
        raise host_perf.ReceiptError("special receipt version differs")
    if not isinstance(receipt["mode"], str) or receipt["mode"] not in RUNNERS:
        raise host_perf.ReceiptError("special receipt version or mode differs")
    if receipt["status"] != "complete" or receipt["reason"] is not None:
        raise host_perf.ReceiptError(f"special receipt invalid: {receipt['reason']}")
    if receipt["performance_status"] != "UNQUALIFIED":
        raise host_perf.ReceiptError("special receipt cannot claim performance")
    if receipt["start_identity"] != receipt["end_identity"]:
        raise host_perf.ReceiptError("special source or host identity changed")
    host_perf.validate_identity(receipt["start_identity"])
    if receipt["features"] != receipt["start_identity"]["features"]:
        raise host_perf.ReceiptError("special feature identity differs")
    built_features = receipt["build_artifact_features"]
    if (
        not isinstance(built_features, list)
        or not all(isinstance(feature, str) and feature for feature in built_features)
        or built_features != sorted(set(built_features))
        or "default" not in built_features
        or not set(receipt["features"]).issubset(built_features)
    ):
        raise host_perf.ReceiptError("special build artifact features differ")
    source_digest = receipt["source_content_sha256"]
    if (
        not isinstance(source_digest, str)
        or len(source_digest) != 64
        or any(character not in "0123456789abcdef" for character in source_digest)
    ):
        raise host_perf.ReceiptError("special source content digest is invalid")
    if (
        type(receipt["runner_exit_code"]) is not int
        or type(receipt["validator_exit_code"]) is not int
        or receipt["runner_exit_code"] != 0
        or receipt["validator_exit_code"] != 0
    ):
        raise host_perf.ReceiptError("special runner or validator exit is not zero")
    for name in ("scenario", "raw", "topology", "runner", "validator", "resources"):
        actual = host_perf.sha256((directory / name).read_bytes())
        if actual != receipt[f"{name}_sha256"]:
            raise host_perf.ReceiptError(f"special {name} digest differs")
    host_perf.validate_resource_artifact(
        (directory / "resources").read_bytes(), receipt["runner_pid"]
    )
    if require_current_source:
        if source_content_sha256() != receipt["source_content_sha256"]:
            raise host_perf.ReceiptError("current checkout differs from acquired special source")
        source = receipt["end_identity"]
        current = host_perf.local_identity(
            host_perf.parse_object((directory / "scenario").read_bytes(), "scenario"),
            receipt["features"],
        )
        for field in ("source_head", "source_tree", "source_dirty", "lock_sha256", "rustc"):
            if current[field] != source[field]:
                raise host_perf.ReceiptError(f"current source/toolchain {field} differs")
    result = subprocess.run(
        [
            str(directory / "validator"),
            receipt["mode"],
            str(directory / "scenario"),
            str(directory / "raw"),
            str(directory / "topology"),
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise host_perf.ReceiptError(
            f"typed special raw/topology rejected: {result.stderr[-1000:]}"
        )
    return receipt


def acquire(mode: str, scenario_path: Path, directory: Path, features: list[str]) -> None:
    if directory.exists():
        raise host_perf.ReceiptError(f"refusing to overwrite {directory}")
    if directory.resolve().is_relative_to(host_perf.REPO):
        raise host_perf.ReceiptError("special output directory must be outside the source checkout")
    scenario_bytes = scenario_path.read_bytes()
    scenario = host_perf.parse_object(scenario_bytes, "special scenario")
    if not isinstance(scenario.get("topology"), dict):
        raise host_perf.ReceiptError("special scenario lacks topology")
    directory.mkdir(parents=True)
    host_run.write_new(directory / "scenario", scenario_bytes)
    baseline = host_perf.local_identity(scenario, features)
    source_digest = source_content_sha256()
    runner_name = RUNNERS[mode]
    runner, runner_features, runner_command = host_run.build_runner(features, runner_name)
    validator, validator_features, validator_command = host_run.build_runner(
        features, "host_special_validate"
    )
    if runner_features != validator_features:
        raise host_perf.ReceiptError("special runner and validator features differ")
    if (
        host_perf.local_identity(scenario, features) != baseline
        or source_content_sha256() != source_digest
    ):
        raise host_perf.ReceiptError("source or host identity changed during special build")
    with tempfile.TemporaryDirectory(prefix="taskmesh-special-binary-") as temporary:
        sealed_runner = Path(temporary) / "runner"
        sealed_validator = Path(temporary) / "validator"
        shutil.copy2(runner, sealed_runner)
        shutil.copy2(validator, sealed_validator)
        host_run.retain_executable(sealed_runner, directory / "runner")
        host_run.retain_executable(sealed_validator, directory / "validator")
        exit_code, pid, stderr, resources = sample_subprocess(
            [
                str(sealed_runner),
                str(directory / "scenario"),
                str(directory / "raw"),
                str(directory / "topology"),
            ],
            cwd=host_perf.REPO,
        )
        host_run.write_new(directory / "resources", host_perf.canonical(resources) + b"\n")
        validator_exit = None
        if exit_code == 0 and (directory / "raw").is_file() and (directory / "topology").is_file():
            checked = subprocess.run(
                [
                    str(sealed_validator),
                    mode,
                    str(directory / "scenario"),
                    str(directory / "raw"),
                    str(directory / "topology"),
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            validator_exit = checked.returncode
            if validator_exit:
                stderr += checked.stderr[-1000:]
        end = host_perf.local_identity(scenario, features)
        source_unchanged = source_content_sha256() == source_digest
        sealed_unchanged = all(
            host_perf.sha256(sealed.read_bytes())
            == host_perf.sha256((directory / name).read_bytes())
            for name, sealed in (("runner", sealed_runner), ("validator", sealed_validator))
        )
    reason = None
    if end != baseline or not source_unchanged:
        reason = "source or host identity changed during special run"
    elif not sealed_unchanged:
        reason = "special sealed binary changed"
    elif exit_code != 0:
        reason = f"special runner failed: {stderr[-1000:]}"
    elif validator_exit != 0:
        reason = f"special validator failed: {stderr[-1000:]}"
    else:
        try:
            host_perf.validate_resource_artifact((directory / "resources").read_bytes(), pid)
        except host_perf.ReceiptError as error:
            reason = f"special process resources invalid: {error}"
    receipt = {
        "schema_version": 1,
        "status": "invalid" if reason else "complete",
        "reason": reason,
        "performance_status": "UNQUALIFIED",
        "mode": mode,
        **{
            f"{name}_sha256": host_perf.sha256((directory / name).read_bytes())
            if (directory / name).is_file()
            else None
            for name in ("scenario", "raw", "topology", "runner", "validator", "resources")
        },
        "runner_pid": pid,
        "runner_exit_code": exit_code,
        "validator_exit_code": validator_exit,
        "build_commands": [runner_command, validator_command],
        "features": features,
        "build_artifact_features": runner_features,
        "start_identity": baseline,
        "end_identity": end,
        "source_content_sha256": source_digest,
    }
    host_run.write_new(directory / "receipt.json", host_perf.canonical(receipt) + b"\n")
    if reason:
        raise host_perf.ReceiptError(reason)
    verify_receipt(directory, require_current_source=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=RUNNERS)
    parser.add_argument("scenario", type=Path)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--feature", action="append", default=[])
    args = parser.parse_args()
    try:
        acquire(args.mode, args.scenario, args.directory, sorted(set(args.feature)))
    except (OSError, subprocess.SubprocessError, host_perf.ReceiptError) as error:
        if args.directory.is_dir() and not (args.directory / "receipt.json").exists():
            host_run.write_new(
                args.directory / "rejection.json",
                host_perf.canonical(
                    {
                        "schema_version": 1,
                        "status": "invalid",
                        "mode": args.mode,
                        "performance_status": "UNQUALIFIED",
                        "reason": str(error),
                    }
                )
                + b"\n",
            )
        print(f"special diagnostic rejected: {error}", file=sys.stderr)
        return 1
    print(
        f"SPECIAL_STRUCTURALLY_VALID performance=UNQUALIFIED mode={args.mode} "
        f"receipt={args.directory / 'receipt.json'}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
