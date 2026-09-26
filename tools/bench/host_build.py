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

if str(host_perf.REPO) not in sys.path:
    sys.path.insert(0, str(host_perf.REPO))
from tools.process_supervisor import run_process  # noqa: E402

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

VERSION = 2
BUILD_TIMEOUT_SECONDS = 1800
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
    "effective_build_environment",
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


def cargo_config_sha256(source: Path) -> dict[str, str]:
    # Cargo searches the invocation directory and every ancestor, including
    # outside the archived source. Retain both names even when one is shadowed.
    home = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))).resolve()
    directories = {home, *(path / ".cargo" for path in (source, *source.parents))}
    return {
        str(path): host_perf.sha256(path.read_bytes())
        for directory in sorted(directories)
        for path in (directory / "config", directory / "config.toml")
        if path.is_file()
    }


def effective_build_environment(root: Path) -> dict[str, str]:
    return {
        **host_perf.build_environment(),
        "CARGO_TARGET_DIR": str(root.resolve() / "target"),
        "CARGO_INCREMENTAL": "0",
    }


def require_standard_codegen(root: Path, proof: dict) -> None:
    """Cargo profile metadata cannot describe later rustflags/compiler overrides.

    Measured admission supports normal Cargo/rustc profile semantics only. A
    custom codegen/wrapper build can retain structural custody but needs its own
    admission contract and correctness oracle.
    """
    environment = proof["build_environment"]
    forbidden = {
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTC",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "RUSTC_BOOTSTRAP",
    }
    if any(
        value and (key in forbidden or key.endswith(("_RUSTFLAGS", "_RUSTC", "_WRAPPER")))
        for key, value in environment.items()
    ):
        raise host_perf.ReceiptError(
            "measured admission does not support custom compiler flags/wrappers"
        )
    configs = cargo_config_sha256(root.resolve() / "source")
    if configs != proof["cargo_config_sha256"]:
        raise host_perf.ReceiptError("frozen Cargo configuration changed")
    for name, digest in configs.items():
        data = Path(name).read_bytes()
        if host_perf.sha256(data) != digest:
            raise host_perf.ReceiptError("Cargo configuration changed during codegen check")
        config = tomllib.loads(data.decode())
        build = config.get("build", {})
        if any(
            build.get(key)
            for key in ("rustflags", "rustc", "rustc-wrapper", "rustc-workspace-wrapper")
        ):
            raise host_perf.ReceiptError(
                "measured admission does not support custom compiler flags/wrappers"
            )
        if any(target.get("rustflags") for target in config.get("target", {}).values()):
            raise host_perf.ReceiptError("measured admission does not support target rustflags")
        # [env] can inject flags into rustc or build scripts after our environment
        # snapshot. Reject this scoped path rather than infer effective settings.
        if config.get("env"):
            raise host_perf.ReceiptError(
                "measured admission does not support Cargo config env overrides"
            )


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
    env.update(effective_build_environment(root))
    process = run_process(
        recipe(features, example), cwd=source, env=env, timeout_seconds=BUILD_TIMEOUT_SECONDS
    )
    stdout, stderr = process.stdout.encode(), process.stderr.encode()
    if process.returncode != 0 or process.timed_out or process.interrupted_by_signal is not None:
        failed = root / f"failed-build-{time.time_ns()}"
        failed.mkdir()
        write_new(failed / "stdout", stdout)
        write_new(failed / "stderr", stderr)
        write_new(
            failed / "execution.json",
            host_perf.canonical(
                {
                    "returncode": process.returncode,
                    "timed_out": process.timed_out,
                    "interrupted_by_signal": process.interrupted_by_signal,
                    "timeout_seconds": BUILD_TIMEOUT_SECONDS,
                }
            )
            + b"\n",
        )
        raise host_perf.ReceiptError(
            f"frozen build failed (timeout={process.timed_out}, "
            f"signal={process.interrupted_by_signal}): {process.stderr[-2000:]}"
        )
    artifacts = []
    for line in stdout.splitlines():
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
    return binary, sorted(artifacts[0]["features"]), stdout, stderr


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


def require_matching_library_profiles(root: Path, proof: dict, profile: dict) -> None:
    """The probe's profile does not prove the governed libraries share it.

    Cargo per-package overrides can optimize the example while leaving the
    engine unoptimized, or change its assertion/overflow semantics. The oracle
    uses one profile, so this admission lane requires all four library profiles
    to match those semantics in both cold-build logs.
    """
    names = {"taskmesh", "taskmesh_contract", "taskmesh_engine", "taskmesh_bench"}
    fields = ("opt_level", "debug_assertions", "overflow_checks", "test")
    for index in range(2):
        name = f"build-{index}.stdout"
        data = (root / name).read_bytes()
        if host_perf.sha256(data) != proof["logs_sha256"][name]:
            raise host_perf.ReceiptError("frozen library profile log digest changed")
        seen = set()
        for line in data.splitlines():
            item = host_perf.parse_object(line, "frozen library Cargo event")
            target = item.get("target", {})
            if (
                item.get("reason") != "compiler-artifact"
                or target.get("name") not in names
                or "lib" not in target.get("kind", [])
            ):
                continue
            library = target["name"]
            actual = item.get("profile")
            if (
                library in seen
                or not isinstance(actual, dict)
                or any(
                    type(actual.get(field)) is not type(profile[field])
                    or actual.get(field) != profile[field]
                    for field in fields
                )
            ):
                raise host_perf.ReceiptError(
                    "frozen governed library profile differs from probe/oracle"
                )
            seen.add(library)
        if seen != names:
            raise host_perf.ReceiptError("frozen governed library profile population incomplete")


def require_oracle_profiles(data: bytes, profile: dict) -> None:
    names = {
        "taskmesh",
        "taskmesh_contract",
        "taskmesh_engine",
        "hardening_mixed_overload",
        "hardening_deadline_custody",
        "hardening_drain",
        "hardening_executor_protocol",
    }
    seen = set()
    for line in data.splitlines():
        # Cargo JSON events and the test harness's normal stdout share a stream.
        if not line.startswith(b"{"):
            continue
        item = host_perf.parse_object(line, "oracle Cargo event")
        target = item.get("target", {})
        name = target.get("name")
        if item.get("reason") != "compiler-artifact" or name not in names:
            continue
        actual = item.get("profile")
        expected_test = name.startswith("hardening_")
        if (
            name in seen
            or not isinstance(actual, dict)
            or target.get("kind") != (["test"] if expected_test else ["lib"])
            or type(actual.get("test")) is not bool
            or actual.get("test") != expected_test
            or any(
                type(actual.get(field)) is not type(profile[field])
                or actual.get(field) != profile[field]
                for field in ("opt_level", "debug_assertions", "overflow_checks")
            )
        ):
            raise host_perf.ReceiptError("actual oracle profile differs from measured build")
        seen.add(name)
    if seen != names:
        raise host_perf.ReceiptError("actual oracle profile population incomplete")


def acquire(root: Path, features: list[str], example: str) -> dict[str, Any]:
    root = root.resolve()
    features = sorted(set(features))
    recipe(features, example)
    identity = host_perf.source_identity()
    if identity["source_dirty"]:
        raise host_perf.ReceiptError("frozen build requires a clean committed source")
    root.mkdir(parents=True, exist_ok=False)
    head = identity["source_head"]
    tree = identity["source_tree"]
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
    configs = cargo_config_sha256(source)
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
        or configs != cargo_config_sha256(source)
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
        "effective_build_environment": effective_build_environment(root),
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
        or proof["effective_build_environment"] != effective_build_environment(root)
        or proof["cargo_config_sha256"] != cargo_config_sha256(root / "source")
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
