#!/usr/bin/env python3
"""Run cached, package-scoped generated mutations for local diagnostics only."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from campaign import (  # noqa: E402
    canonical_digest,
    create_isolated_campaign,
    execute,
    sanitized_campaign_environment,
    source_tree_digest,
    utc_now,
)
from run_generated_mutants import (  # noqa: E402
    classify_generated,
    parse_outcomes,
    raw_manifest,
    tool_identity,
)

REPO = Path(__file__).resolve().parents[2]
CACHE_ROOT = REPO / "target/verification/mutations/focused"


def checked_cache_root() -> Path:
    """Keep diagnostic cache reads and writes below the repository target."""
    target = REPO / "target"
    try:
        relative = CACHE_ROOT.relative_to(target)
    except ValueError as exc:
        raise ValueError("focused mutation cache must be below repository target") from exc
    if ".." in relative.parts:
        raise ValueError("focused mutation cache must be below repository target")
    if target.is_symlink():
        raise ValueError("focused mutation cache path contains a symlink")
    component = target
    for part in relative.parts:
        component /= part
        if component.is_symlink():
            raise ValueError("focused mutation cache path contains a symlink")
    if not CACHE_ROOT.resolve().is_relative_to(REPO.resolve()):
        raise ValueError("focused mutation cache escapes the repository")
    return CACHE_ROOT


def doc_example_build_inputs(source: Path) -> list[str]:
    """Read the build script's explicit repository-root Markdown inputs."""
    build_script = source / "tools/doc-examples/build.rs"
    declaration = re.search(
        r"const SOURCES: &\[&str\] = &\[(.*?)\];",
        build_script.read_text(encoding="utf-8"),
        re.DOTALL,
    )
    if declaration is None:
        raise ValueError("doc-examples build input declaration is missing")
    inputs: list[str] = []
    for line in declaration.group(1).splitlines():
        line = line.strip()
        if not line:
            continue
        literal = re.fullmatch(r'"([^"\\]+)",', line)
        if literal is None:
            raise ValueError("doc-examples build input declaration changed shape")
        relative = Path(literal.group(1))
        path = source / relative
        if (
            relative.is_absolute()
            or ".." in relative.parts
            or not path.is_file()
            or not path.resolve().is_relative_to(source.resolve())
        ):
            raise ValueError(f"doc-examples build input is missing or outside source: {relative}")
        inputs.append(relative.as_posix())
    if not inputs or len(inputs) != len(set(inputs)):
        raise ValueError("doc-examples build input declaration is empty or duplicated")
    return sorted(inputs)


def package_dependency_files(source: Path, package_names: list[str]) -> list[str]:
    metadata = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        cwd=source,
        capture_output=True,
        text=True,
        check=True,
    )
    document = json.loads(metadata.stdout)
    packages = document.get("packages", [])
    by_name = {
        item["name"]: item
        for item in packages
        if Path(item["manifest_path"]).is_relative_to(source)
    }
    missing = sorted(set(package_names) - set(by_name))
    if missing:
        raise ValueError(f"unknown workspace package(s): {', '.join(missing)}")
    by_id = {item["id"]: item for item in packages}
    dependencies = {
        node["id"]: {dep["pkg"] for dep in node.get("deps", [])}
        for node in document.get("resolve", {}).get("nodes", [])
    }
    pending = [by_name[name]["id"] for name in package_names]
    selected: set[str] = set()
    while pending:
        package_id = pending.pop()
        if package_id in selected:
            continue
        selected.add(package_id)
        pending.extend(dependencies.get(package_id, set()) - selected)

    files = {
        "Cargo.toml",
        "Cargo.lock",
        ".cargo/mutants.toml",
        ".cargo/config.toml",
        ".cargo/config",
        "rust-toolchain",
        "rust-toolchain.toml",
        "tools/verification/campaign.py",
        "tools/verification/run_generated_mutants.py",
        "tools/verification/run_focused_mutants.py",
        "tools/verification/mutation-gate.json",
        "tools/process_supervisor.py",
        "tools/qualification/evidence.py",
    }
    for package_id in selected:
        package = by_id[package_id]
        if package["name"] == "taskmesh-doc-examples":
            files.update(doc_example_build_inputs(source))
        package_root = Path(package["manifest_path"]).parent
        try:
            package_root.relative_to(source)
        except ValueError:
            continue
        for path in package_root.rglob("*"):
            if not path.is_file() or any(part in {"target", ".git"} for part in path.parts):
                continue
            try:
                files.add(path.relative_to(source).as_posix())
            except ValueError:
                # Registry dependencies do not participate in the local source digest;
                # Cargo.lock and toolchain identity cover their selected versions.
                continue
    return sorted(files)


def cache_fingerprint(
    source: Path,
    package_names: list[str],
    files: list[str],
    jobs: int,
    timeout: int | None,
    campaign_timeout: int,
    diff_sha256: str | None,
    test_target: str | None = None,
    test_filter: str | None = None,
) -> str:
    dependency_digest = source_tree_digest(source, package_dependency_files(source, package_names))
    return canonical_digest(
        {
            "package_dependency_digest": dependency_digest,
            "packages": sorted(package_names),
            "files": sorted(files),
            "diff_sha256": diff_sha256,
            "test_target": test_target,
            "test_filter": test_filter,
            "jobs": jobs,
            "timeout": timeout,
            "campaign_timeout": campaign_timeout,
            "tools": tool_identity(source),
            "cargo_home": os.environ.get("CARGO_HOME"),
            "rustup_home": os.environ.get("RUSTUP_HOME"),
            "cargo_user_config": cargo_user_config_identity(),
            "test_policy": "cargo-mutants selected Cargo test target/filter; baseline=run",
        }
    )


def cargo_user_config_identity() -> dict[str, str]:
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    identities = {}
    for name in ("config.toml", "config"):
        path = cargo_home / name
        if path.is_file():
            identities[name] = hashlib.sha256(path.read_bytes()).hexdigest()
    return identities


def command(
    packages: list[str],
    files: list[str],
    output: Path,
    jobs: int,
    timeout: int | None,
    diff_path: Path | None = None,
    test_target: str | None = None,
    test_filter: str | None = None,
) -> list[str]:
    argv = [
        "cargo",
        "mutants",
        "--baseline",
        "run",
        "--no-shuffle",
        "--colors",
        "never",
        "--output",
        str(output),
        "--jobs",
        str(jobs),
    ]
    if timeout is not None:
        argv.extend(["--timeout", str(timeout)])
    if diff_path is not None:
        argv.extend(["--in-diff", str(diff_path)])
    if test_target == "lib":
        argv.append("--cargo-arg=--lib")
    elif test_target is not None:
        argv.extend(["--cargo-arg=--test", f"--cargo-arg={test_target}"])
    if test_filter is not None:
        argv.extend(["--cargo-test-arg=--exact", f"--cargo-test-arg={test_filter}"])
    for package in packages:
        argv.extend(["--package", package])
    for file in files:
        argv.extend(["--file", file])
    return argv


def diagnostic_status(classified_status: str, complete: bool, exit_code: int | None) -> str:
    """A complete raw denominator cannot override a failed producer process."""
    return (
        "DIAGNOSTIC_PASS"
        if complete and classified_status == "PASS" and exit_code == 0
        else "DIAGNOSTIC_FAIL"
    )


def process_completed(
    exit_code: int | None,
    signal: int | None,
    timed_out: bool,
    interrupted_by_signal: int | None,
) -> bool:
    return (
        type(exit_code) is int
        and signal is None
        and timed_out is False
        and interrupted_by_signal is None
    )


def reusable_report(path: Path, key: str) -> dict[str, object] | None:
    if path.is_symlink() or path.parent.is_symlink() or not path.is_file():
        return None
    try:
        report = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError):
        return None
    if not isinstance(report, dict):
        return None
    if (
        report.get("schema_version") != 1
        or report.get("kind") != "focused-generated-cargo-mutants-diagnostic"
        or report.get("cache_key") != key
        or report.get("complete") is not True
        or report.get("report_path") != str(path)
        or report.get("status") not in {"DIAGNOSTIC_PASS", "DIAGNOSTIC_FAIL"}
        or report.get("source_unchanged") is not True
        or report.get("parse_error") is not None
    ):
        return None
    source = report.get("source")
    if not isinstance(source, dict):
        return None
    snapshot_digest = source.get("snapshot_sha256")
    if (
        not isinstance(source.get("source_head"), str)
        or re.fullmatch(r"[0-9a-f]{40}", source["source_head"]) is None
        or type(source.get("source_dirty")) is not bool
        or not isinstance(snapshot_digest, str)
        or re.fullmatch(r"[0-9a-f]{64}", snapshot_digest) is None
        or source.get("source_before_sha256") != snapshot_digest
    ):
        return None
    diff_sha256 = report.get("diff_sha256")
    diff_path = path.parent / "selection.diff"
    if diff_sha256 is not None:
        if not isinstance(diff_sha256, str) or not diff_path.is_file():
            return None
        if hashlib.sha256(diff_path.read_bytes()).hexdigest() != diff_sha256:
            return None
    elif diff_path.exists():
        return None
    try:
        raw = path.parent / "raw"
        if raw.is_symlink():
            return None
        if raw_manifest(raw) != report.get("raw_artifacts"):
            return None
        outcomes = parse_outcomes(raw)
        counts, classified_status, _ = classify_generated(outcomes, [])
    except (OSError, ValueError, json.JSONDecodeError):
        return None
    process = report.get("process")
    if (
        not isinstance(process, dict)
        or not {
            "exit_code",
            "signal",
            "timed_out",
            "interrupted_by_signal",
        }.issubset(process)
        or not process_completed(
            process.get("exit_code"),
            process.get("signal"),
            process.get("timed_out"),
            process.get("interrupted_by_signal"),
        )
    ):
        return None
    expected_status = diagnostic_status(classified_status, True, process.get("exit_code"))
    if (
        outcomes != report.get("outcomes")
        or counts != report.get("counts")
        or sum(counts.values()) != report.get("denominator")
        or expected_status != report.get("status")
    ):
        return None
    return report


def find_reusable_report(cache_entry: Path, key: str) -> dict[str, object] | None:
    if cache_entry.is_symlink():
        return None
    candidates = []
    try:
        for path in cache_entry.glob("*/focused-mutation-report.json"):
            try:
                mtime_ns = path.stat().st_mtime_ns
            except OSError:
                # Cache pruning may race with lookup; a vanished entry is a miss.
                continue
            report = reusable_report(path, key)
            if report is not None:
                candidates.append((mtime_ns, str(path), report))
    except OSError:
        # Directory enumeration can race with cache pruning too. Keep any
        # complete candidates already observed and otherwise report a miss.
        pass
    return max(
        candidates,
        key=lambda item: (item[0], item[1]),
        default=(0, "", None),
    )[2]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", action="append", required=True)
    parser.add_argument("--file", action="append", default=[])
    parser.add_argument(
        "--diff",
        type=Path,
        help="unified diff used to narrow mutations to changed source lines",
    )
    parser.add_argument(
        "--test-target",
        help="limit each mutation to a Cargo test target name, or 'lib' for unit tests",
    )
    parser.add_argument(
        "--test-filter",
        help="run only this exact Cargo test name as the mutation oracle",
    )
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--timeout", type=int)
    parser.add_argument("--campaign-timeout", type=int, default=86400)
    parser.add_argument(
        "--refresh", action="store_true", help="rerun even if an exact cache entry exists"
    )
    args = parser.parse_args(argv)
    if not 1 <= args.jobs <= 3:
        parser.error("focused generated mutations require --jobs from 1 through 3")
    if args.timeout is not None and args.timeout < 1:
        parser.error("--timeout must be positive")
    if args.campaign_timeout < 1:
        parser.error("--campaign-timeout must be positive")
    if args.test_target and args.test_target != "lib" and len(args.package) != 1:
        parser.error("an integration --test-target requires exactly one --package")
    if args.test_filter is not None and not args.test_filter.strip():
        parser.error("--test-filter must be a non-empty exact test name")
    try:
        diff_bytes = args.diff.read_bytes() if args.diff is not None else None
    except OSError as error:
        parser.error(f"cannot read --diff: {error}")
    if diff_bytes is not None and not diff_bytes.strip():
        parser.error("--diff must contain a non-empty unified diff")
    diff_sha256 = hashlib.sha256(diff_bytes).hexdigest() if diff_bytes is not None else None

    checked_cache_root()
    env = sanitized_campaign_environment(os.environ.copy())
    preflight_key = cache_fingerprint(
        REPO,
        args.package,
        args.file,
        args.jobs,
        args.timeout,
        args.campaign_timeout,
        diff_sha256,
        args.test_target,
        args.test_filter,
    )
    preflight_entry = CACHE_ROOT / preflight_key
    if not args.refresh:
        cached = find_reusable_report(preflight_entry, preflight_key)
        if cached is not None:
            # Rehash the selected dependency closure before trusting a cache hit;
            # a concurrently edited source tree must fall through to snapshotting.
            confirmed_key = cache_fingerprint(
                REPO,
                args.package,
                args.file,
                args.jobs,
                args.timeout,
                args.campaign_timeout,
                diff_sha256,
                args.test_target,
                args.test_filter,
            )
            if confirmed_key == preflight_key:
                print(
                    json.dumps(
                        {
                            "status": cached["status"],
                            "reused": True,
                            "report": cached["report_path"],
                            "cache_key": preflight_key,
                        }
                    )
                )
                return 0 if cached["status"] == "DIAGNOSTIC_PASS" else 1

    campaign = create_isolated_campaign(REPO)
    try:
        key = cache_fingerprint(
            campaign.source,
            args.package,
            args.file,
            args.jobs,
            args.timeout,
            args.campaign_timeout,
            diff_sha256,
            args.test_target,
            args.test_filter,
        )
        cache_entry = CACHE_ROOT / key
        if not args.refresh:
            cached = find_reusable_report(cache_entry, key)
            if cached is not None:
                print(
                    json.dumps(
                        {
                            "status": cached["status"],
                            "reused": True,
                            "report": cached["report_path"],
                            "cached_source_sha256": cached["source"]["snapshot_sha256"],
                            "current_source_sha256": campaign.snapshot_digest,
                        }
                    )
                )
                return 0 if cached["status"] == "DIAGNOSTIC_PASS" else 1

        output = cache_entry / campaign.campaign_id
        checked_cache_root()
        if (
            cache_entry.is_symlink()
            or output.is_symlink()
            or not output.resolve().is_relative_to(CACHE_ROOT.resolve())
        ):
            raise ValueError("focused mutation cache path escaped its target directory")
        output.mkdir(parents=True)
        report_path = output / "focused-mutation-report.json"
        raw_parent = campaign.root / "cargo-mutants-raw"
        diff_path = output / "selection.diff" if diff_bytes is not None else None
        if diff_path is not None:
            diff_path.write_bytes(diff_bytes)
        run_argv = command(
            args.package,
            args.file,
            raw_parent,
            args.jobs,
            args.timeout,
            diff_path,
            args.test_target,
            args.test_filter,
        )
        result = execute(
            run_argv,
            cwd=campaign.source,
            env=env,
            timeout_seconds=args.campaign_timeout,
        )
        raw_source = raw_parent / "mutants.out"
        if raw_source.is_dir():
            shutil.copytree(raw_source, output / "raw")

        parse_error = None
        outcomes: dict[str, list[str]] = {"caught": [], "missed": [], "unviable": [], "timeout": []}
        counts = {"caught": 0, "missed": 0, "unviable": 0, "timeout": 0, "equivalent": 0}
        classified_status = "FAIL"
        try:
            outcomes = parse_outcomes(output / "raw")
            counts, classified_status, _ = classify_generated(outcomes, [])
        except (ValueError, OSError, json.JSONDecodeError) as error:
            parse_error = str(error)

        unchanged = campaign.source_after(REPO) == campaign.source_before
        complete = (
            parse_error is None
            and unchanged
            and process_completed(
                result.exit_code,
                result.signal,
                result.timed_out,
                result.interrupted_by_signal,
            )
        )
        status = diagnostic_status(classified_status, complete, result.exit_code)
        limitations = [
            "diagnostic-only; not a workspace mutation qualification",
            "selected mutants run against the package test suite",
            "downstream consumer-test coverage requires a full campaign",
        ]
        if args.test_target is not None:
            limitations.append("tests outside the selected Cargo test target are not exercised")
        if args.test_filter is not None:
            limitations.append("tests other than the selected exact test name are not exercised")
        report = {
            "schema_version": 1,
            "kind": "focused-generated-cargo-mutants-diagnostic",
            "generated_at": utc_now(),
            "report_path": str(report_path),
            "cache_key": key,
            "complete": complete,
            "status": status,
            "scope": {
                "packages": sorted(args.package),
                "files": sorted(args.file),
                "test_target": args.test_target,
                "test_filter": args.test_filter,
            },
            "diff_sha256": diff_sha256,
            "reuse_policy": (
                "selected workspace packages, transitive local dependencies, tests, manifests, "
                "mutation config, tool versions, and options must match"
            ),
            "source": campaign.identity(),
            "source_unchanged": unchanged,
            "tools": tool_identity(campaign.source),
            "command": run_argv,
            "process": {
                "exit_code": result.exit_code,
                "signal": result.signal,
                "timed_out": result.timed_out,
                "interrupted_by_signal": result.interrupted_by_signal,
                "duration_s": result.duration_s,
            },
            "counts": counts,
            "denominator": sum(counts.values()),
            "outcomes": outcomes,
            "parse_error": parse_error,
            "limitations": limitations,
            "raw_artifacts": raw_manifest(output / "raw") if (output / "raw").is_dir() else [],
        }
        (output / "focused-mutation-report.json").write_text(
            json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        print(
            json.dumps(
                {"status": status, "reused": False, "counts": counts, "report": str(report_path)}
            )
        )
        return 0 if status == "DIAGNOSTIC_PASS" else 1
    finally:
        campaign.cleanup()


if __name__ == "__main__":
    raise SystemExit(main())
