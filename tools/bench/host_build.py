#!/usr/bin/env python3
"""Retain a frozen Git archive and cold rebuild witnesses for host probes.

Local reproducibility evidence only; this is not an independent peer rerun.
Verification with rebuild=True executes Cargo again, never trusts a PASS flag.
"""

from __future__ import annotations

import argparse
import io
import os
import shutil
import subprocess
import sys
import tarfile
import time
from pathlib import Path
from typing import Any

import host_perf
from host_run import retain_executable, write_new

VERSION = 1
EXAMPLES = {
    "host_load_probe",
    "host_generator_probe",
    "host_closed_loop_probe",
    "host_local_probe",
    "host_composite_probe",
    "host_special_validate",
}
KEYS = {
    "schema_version",
    "source_head",
    "source_tree",
    "archive_sha256",
    "lock_sha256",
    "rustc",
    "cargo",
    "build_environment",
    "features",
    "example",
    "binary_sha256",
    "artifact_features",
    "build_command",
    "workspace_path",
    "logs_sha256",
    "cargo_config_sha256",
}


def command_output(command: list[str], cwd: Path = host_perf.REPO) -> bytes:
    return subprocess.run(command, cwd=cwd, check=True, capture_output=True).stdout


def cargo_config_sha256() -> dict[str, str]:
    home = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))).resolve()
    return {
        str(path): host_perf.sha256(path.read_bytes())
        for path in (home / "config", home / "config.toml")
        if path.is_file()
    }


def archive_members(data: bytes) -> dict[str, tuple[bytes, int]]:
    result = {}
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        for entry in archive.getmembers():
            path = Path(entry.name)
            if path.is_absolute() or ".." in path.parts:
                raise host_perf.ReceiptError("unsafe frozen source path")
            if entry.isdir():
                continue
            if not entry.isfile() or entry.name in result:
                raise host_perf.ReceiptError("frozen source requires unique regular files")
            stream = archive.extractfile(entry)
            if stream is None:
                raise host_perf.ReceiptError("frozen source member unavailable")
            result[entry.name] = (stream.read(), entry.mode)
    if "Cargo.lock" not in result:
        raise host_perf.ReceiptError("frozen source lacks Cargo.lock")
    return result


def check_source(source: Path, members: dict) -> None:
    files = {
        str(path.relative_to(source)): path
        for path in source.rglob("*")
        if path.is_file() or path.is_symlink()
    }
    if files.keys() != members.keys():
        raise host_perf.ReceiptError("frozen source file inventory changed")
    for name, (data, mode) in members.items():
        path = files[name]
        if (
            path.is_symlink()
            or path.read_bytes() != data
            or (path.stat().st_mode & 0o111) != (mode & 0o111)
        ):
            raise host_perf.ReceiptError(f"frozen source changed: {name}")


def recipe(features: list[str], example: str) -> list[str]:
    if (
        not isinstance(example, str)
        or example not in EXAMPLES
        or not isinstance(features, list)
        or any(not isinstance(f, str) or not f for f in features)
        or features != sorted(set(features))
    ):
        raise host_perf.ReceiptError("invalid frozen build target/features")
    return [
        "cargo",
        "build",
        "--locked",
        "-p",
        "taskmesh-bench",
        "--example",
        example,
        "--message-format=json",
        *(["--features", ",".join(features)] if features else []),
    ]


def cold_build(
    root: Path, features: list[str], example: str
) -> tuple[Path, list[str], bytes, bytes]:
    source = root / "source"
    target = root / "target"
    # target is exclusively owned by this fresh witness directory.
    if target.is_symlink():
        raise host_perf.ReceiptError("frozen build target cannot be a symlink")
    if target.exists():
        shutil.rmtree(target)
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(target)
    env["CARGO_INCREMENTAL"] = "0"
    process = subprocess.run(
        recipe(features, example), cwd=source, env=env, capture_output=True, check=False
    )
    if process.returncode:
        failed = root / f"failed-build-{time.time_ns()}"
        failed.mkdir()
        write_new(failed / "stdout", process.stdout)
        write_new(failed / "stderr", process.stderr)
        raise host_perf.ReceiptError(
            f"frozen build failed: {process.stderr[-2000:].decode(errors='replace')}"
        )
    artifacts = []
    for line in process.stdout.splitlines():
        item = host_perf.parse_object(line, "Cargo event")
        if (
            item.get("reason") == "compiler-artifact"
            and item.get("target", {}).get("name") == example
            and item.get("executable")
        ):
            artifacts.append(item)
    if len(artifacts) != 1:
        raise host_perf.ReceiptError("frozen build requires exactly one executable")
    binary = Path(artifacts[0]["executable"]).resolve()
    if not binary.is_relative_to(target.resolve()) or not binary.is_file():
        raise host_perf.ReceiptError("frozen build executable outside owned target")
    return binary, sorted(artifacts[0]["features"]), process.stdout, process.stderr


def cargo_profile(log: bytes, example: str) -> dict[str, Any]:
    profiles = []
    for line in log.splitlines():
        item = host_perf.parse_object(line, "frozen Cargo event")
        if (
            item.get("reason") == "compiler-artifact"
            and item.get("target", {}).get("name") == example
            and "example" in item.get("target", {}).get("kind", [])
        ):
            profiles.append(item.get("profile"))
    if len(profiles) != 1 or not isinstance(profiles[0], dict):
        raise host_perf.ReceiptError("frozen Cargo log has no unique build profile")
    profile = profiles[0]
    if (
        not isinstance(profile.get("opt_level"), str)
        or type(profile.get("debug_assertions")) is not bool
        or type(profile.get("test")) is not bool
        or type(profile.get("overflow_checks")) is not bool
    ):
        raise host_perf.ReceiptError("frozen Cargo profile is malformed")
    return profile


def verified_profile(root: Path, proof: dict) -> dict[str, Any]:
    profiles = []
    for index in range(2):
        name = f"build-{index}.stdout"
        data = (root / name).read_bytes()
        if host_perf.sha256(data) != proof["logs_sha256"][name]:
            raise host_perf.ReceiptError("frozen profile log digest changed")
        profiles.append(cargo_profile(data, proof["example"]))
    if profiles[0] != profiles[1]:
        raise host_perf.ReceiptError("cold rebuild profile changed")
    return profiles[0]


def acquire(root: Path, features: list[str], example: str) -> dict[str, Any]:
    root = root.resolve()
    features = sorted(set(features))
    recipe(features, example)
    if command_output(["git", "status", "--porcelain=v1"]).strip():
        raise host_perf.ReceiptError("frozen build requires a clean committed source")
    root.mkdir(parents=True, exist_ok=False)
    head = command_output(["git", "rev-parse", "HEAD"]).decode().strip()
    tree = command_output(["git", "rev-parse", "HEAD^{tree}"]).decode().strip()
    archive = command_output(["git", "archive", "--format=tar", head])
    members = archive_members(archive)
    write_new(root / "source.tar", archive)
    source = root / "source"
    source.mkdir()
    for name, (data, mode) in members.items():
        path = source / name
        path.parent.mkdir(parents=True, exist_ok=True)
        write_new(path, data)
        path.chmod(mode & 0o555)  # read-only source; no Cargo target in source.
    rustc = command_output(["rustc", "--version", "--verbose"]).decode()
    cargo = command_output(["cargo", "--version", "--verbose"]).decode()
    environment = host_perf.build_environment()
    configs = cargo_config_sha256()
    digests, artifact_features, logs = [], None, {}
    for index in range(2):
        check_source(source, members)
        binary, active_features, stdout, stderr = cold_build(root, features, example)
        digests.append(host_perf.sha256(retain_executable(binary, root / f"runner-{index}")))
        if artifact_features is not None and artifact_features != active_features:
            raise host_perf.ReceiptError("cold rebuild feature identity changed")
        artifact_features = active_features
        for name, data in ((f"build-{index}.stdout", stdout), (f"build-{index}.stderr", stderr)):
            write_new(root / name, data)
            logs[name] = host_perf.sha256(data)
        check_source(source, members)
    if digests[0] != digests[1]:
        raise host_perf.ReceiptError("cold rebuilds are not byte reproducible")
    if (
        environment != host_perf.build_environment()
        or configs != cargo_config_sha256()
        or rustc != command_output(["rustc", "--version", "--verbose"]).decode()
    ):
        raise host_perf.ReceiptError("build environment changed")
    proof = {
        "schema_version": VERSION,
        "source_head": head,
        "source_tree": tree,
        "archive_sha256": host_perf.sha256(archive),
        "lock_sha256": host_perf.sha256(members["Cargo.lock"][0]),
        "rustc": rustc,
        "cargo": cargo,
        "build_environment": environment,
        "features": features,
        "example": example,
        "binary_sha256": digests[0],
        "artifact_features": artifact_features,
        "build_command": recipe(features, example),
        "workspace_path": str(source),
        "logs_sha256": logs,
        "cargo_config_sha256": configs,
    }
    write_new(root / "build-witness.json", host_perf.canonical(proof) + b"\n")
    return proof


def verify(root: Path, *, rebuild: bool = False) -> dict[str, Any]:
    root = root.resolve()
    proof = host_perf.parse_object((root / "build-witness.json").read_bytes(), "build witness")
    host_perf.exact_keys(proof, KEYS, "build witness")
    if type(proof["schema_version"]) is not int or proof["schema_version"] != VERSION:
        raise host_perf.ReceiptError("unsupported frozen build version")
    if proof["workspace_path"] != str(root / "source"):
        raise host_perf.ReceiptError("frozen workspace moved; acquire a new witness")
    if proof["build_command"] != recipe(proof["features"], proof["example"]):
        raise host_perf.ReceiptError("frozen build command differs")
    archive = (root / "source.tar").read_bytes()
    if (
        host_perf.sha256(archive) != proof["archive_sha256"]
        or archive != command_output(["git", "archive", "--format=tar", proof["source_head"]])
        or command_output(["git", "rev-parse", proof["source_head"] + "^{tree}"]).decode().strip()
        != proof["source_tree"]
    ):
        raise host_perf.ReceiptError("frozen archive does not match declared Git source")
    members = archive_members(archive)
    if host_perf.sha256(members["Cargo.lock"][0]) != proof["lock_sha256"]:
        raise host_perf.ReceiptError("frozen lock differs")
    check_source(root / "source", members)
    expected_logs = {f"build-{i}.{kind}" for i in range(2) for kind in ("stdout", "stderr")}
    if not isinstance(proof["logs_sha256"], dict) or proof["logs_sha256"].keys() != expected_logs:
        raise host_perf.ReceiptError("frozen build logs incomplete")
    for name, digest in proof["logs_sha256"].items():
        if host_perf.sha256((root / name).read_bytes()) != digest:
            raise host_perf.ReceiptError("frozen build log changed")
    for index in range(2):
        if host_perf.sha256((root / f"runner-{index}").read_bytes()) != proof["binary_sha256"]:
            raise host_perf.ReceiptError("frozen executable changed")
    if (
        proof["rustc"] != command_output(["rustc", "--version", "--verbose"]).decode()
        or proof["cargo"] != command_output(["cargo", "--version", "--verbose"]).decode()
        or proof["build_environment"] != host_perf.build_environment()
        or proof["cargo_config_sha256"] != cargo_config_sha256()
    ):
        raise host_perf.ReceiptError("frozen toolchain/build environment differs")
    if rebuild:
        binary, features, stdout, _ = cold_build(root, proof["features"], proof["example"])
        check_source(root / "source", members)
        if (
            host_perf.sha256(binary.read_bytes()) != proof["binary_sha256"]
            or features != proof["artifact_features"]
        ):
            raise host_perf.ReceiptError(
                "independent cold rebuild differs from retained executable"
            )
        if cargo_profile(stdout, proof["example"]) != verified_profile(root, proof):
            raise host_perf.ReceiptError("independent cold rebuild profile differs")
    return proof


def runner(
    root: Path, identity: dict, features: list[str], example: str
) -> tuple[Path, list[str], list[str]]:
    proof = verify(root)
    if (
        identity["source_dirty"]
        or identity["source_head"] != proof["source_head"]
        or identity["source_tree"] != proof["source_tree"]
        or identity["lock_sha256"] != proof["lock_sha256"]
        or features != proof["features"]
        or example != proof["example"]
    ):
        raise host_perf.ReceiptError("run source/features do not match frozen build")
    return root.resolve() / "runner-0", proof["artifact_features"], proof["build_command"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("acquire", "verify"))
    parser.add_argument("directory", type=Path)
    parser.add_argument("--feature", action="append", default=[])
    parser.add_argument("--example", choices=sorted(EXAMPLES), default="host_load_probe")
    parser.add_argument("--rebuild", action="store_true")
    args = parser.parse_args()
    try:
        proof = (
            acquire(args.directory, args.feature, args.example)
            if args.mode == "acquire"
            else verify(args.directory, rebuild=args.rebuild)
        )
    except (OSError, ValueError, subprocess.SubprocessError, tarfile.TarError) as error:
        print(f"frozen build rejected: {error}", file=sys.stderr)
        return 1
    print(
        f"FROZEN_BUILD_VALID binary={proof['binary_sha256']} "
        f"rebuilt={args.mode == 'acquire' or args.rebuild} performance=UNQUALIFIED"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
