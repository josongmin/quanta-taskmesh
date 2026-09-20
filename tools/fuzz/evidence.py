#!/usr/bin/env python3
"""Validate V03 fuzz anti-vacuity witnesses and emit V01 evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[2]
ARTIFACT_ROOT = REPO / "target/sep21/v03/fuzz"
if str(REPO) not in sys.path:
    sys.path.insert(0, str(REPO))

from tools.qualification.evidence import (  # noqa: E402
    canonical_bytes,
    command_digest,
    envelope_problems,
    file_identity,
    tool_identity_digest,
)
from tools.qualification.receipt import source_identity  # noqa: E402

RUNS = re.compile(r"^Done ([0-9]+) runs in ", re.MULTILINE)
CHECKPOINT = re.compile(
    r"^taskmesh-fuzz-checkpoint target=([a-z0-9_]+) checkpoint=([a-z0-9_]+)$",
    re.MULTILINE,
)


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise ValueError(f"{path}: root must be an object")
    return value


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def absolute_from_cwd(path: Path) -> Path:
    return path if path.is_absolute() else (Path.cwd() / path).resolve()


def safe_descendant(path: Path, root: Path, role: str) -> Path:
    """Resolve a named artifact path below root, rejecting root and escapes."""
    resolved_root = root.resolve()
    resolved = absolute_from_cwd(path).resolve()
    try:
        relative = resolved.relative_to(resolved_root)
    except ValueError as error:
        raise ValueError(f"{role} must be below {resolved_root}") from error
    if not relative.parts:
        raise ValueError(f"{role} must name a directory below {resolved_root}")
    return resolved


def validate_artifact_layout(
    *, output_dir: Path, logs_dir: Path, corpus_dir: Path, root: Path = ARTIFACT_ROOT
) -> tuple[Path, Path, Path]:
    """Resolve all writable directories and keep them in one V03 run directory."""
    output = safe_descendant(output_dir, root, "output directory")
    logs = safe_descendant(logs_dir, output, "logs directory")
    corpus = safe_descendant(corpus_dir, output, "corpus directory")
    if logs == corpus:
        raise ValueError("logs and corpus directories must be distinct")
    return output, logs, corpus


def source_drift_problems(before: dict[str, Any], after: dict[str, Any]) -> list[str]:
    problems = []
    for key in ("head", "paths_digest", "dirty"):
        if before.get(key) != after.get(key):
            problems.append(f"source changed during fuzz execution: {key}")
    return problems


def corpus_digest(targets: dict[str, object]) -> str:
    return hashlib.sha256(canonical_bytes(targets)).hexdigest()


def validate_manifests(
    producer: dict[str, Any], corpus: dict[str, Any], *, root: Path = REPO
) -> list[str]:
    problems: list[str] = []
    required = producer.get("required_targets")
    target_corpora = corpus.get("targets")
    if not isinstance(required, dict) or not required:
        return ["producer required_targets must be a non-empty object"]
    if not isinstance(target_corpora, dict):
        return ["corpus targets must be an object"]
    if set(required) != set(target_corpora):
        actual_names = sorted(target_corpora)
        required_names = sorted(required)
        problems.append(
            f"corpus target set {actual_names} does not equal required target set {required_names}"
        )
    if corpus.get("digest") != corpus_digest(target_corpora):
        problems.append("corpus manifest digest mismatch")
    for target, entries in target_corpora.items():
        if not isinstance(entries, list) or not entries:
            problems.append(f"{target}: corpus must contain at least one tracked seed")
            continue
        for index, entry in enumerate(entries):
            if not isinstance(entry, dict):
                problems.append(f"{target}: corpus entry {index} must be an object")
                continue
            rel = entry.get("path")
            expected = entry.get("sha256")
            if not isinstance(rel, str) or not rel.startswith(f"fuzz/seed-corpus/{target}/"):
                problems.append(f"{target}: invalid corpus path {rel!r}")
                continue
            path = root / rel
            if not path.is_file():
                problems.append(f"{target}: missing corpus seed {rel}")
            elif sha256(path) != expected:
                problems.append(f"{target}: corpus seed digest mismatch for {rel}")
    return problems


def validate_target_set(actual: list[str], required: dict[str, Any]) -> list[str]:
    if len(actual) != len(set(actual)):
        return ["cargo-fuzz listed duplicate targets"]
    if set(actual) != set(required):
        return [
            f"actual target set {sorted(actual)} does not equal required set {sorted(required)}"
        ]
    return []


def parse_target_log(
    target: str, text: str, required_checkpoints: list[str], duration_seconds: int
) -> tuple[dict[str, Any], list[str]]:
    problems: list[str] = []
    matches = RUNS.findall(text)
    runs = int(matches[-1]) if matches else 0
    if duration_seconds <= 0:
        problems.append(f"{target}: duration_seconds must be positive")
    if runs <= 0:
        problems.append(f"{target}: libFuzzer completed zero runs or omitted its run summary")
    seen_pairs = CHECKPOINT.findall(text)
    foreign = sorted({seen_target for seen_target, _ in seen_pairs if seen_target != target})
    if foreign:
        problems.append(f"{target}: log contains witnesses for other targets {foreign}")
    seen = [name for seen_target, name in seen_pairs if seen_target == target]
    missing = sorted(set(required_checkpoints) - set(seen))
    unknown = sorted(set(seen) - set(required_checkpoints))
    if missing:
        problems.append(f"{target}: missing semantic checkpoints {missing}")
    if unknown:
        problems.append(f"{target}: undeclared semantic checkpoints {unknown}")
    checkpoint_counts = {name: seen.count(name) for name in required_checkpoints}
    semantic = [name for name in required_checkpoints if name != "target_entry"]
    valid_inputs = min((checkpoint_counts[name] for name in semantic), default=0)
    if valid_inputs == 0:
        problems.append(f"{target}: no valid input reached every required semantic checkpoint")
    return (
        {
            "target": target,
            "duration_seconds": duration_seconds,
            "runs": runs,
            "valid_inputs": valid_inputs,
            "semantic_checkpoints": checkpoint_counts,
        },
        problems,
    )


def prepare_corpus(
    corpus: dict[str, Any],
    destination: Path,
    *,
    output_dir: Path,
    artifact_root: Path = ARTIFACT_ROOT,
) -> None:
    _, _, destination = validate_artifact_layout(
        output_dir=output_dir,
        logs_dir=output_dir / "raw",
        corpus_dir=destination,
        root=artifact_root,
    )
    if destination.exists():
        shutil.rmtree(destination)
    for target, entries in corpus["targets"].items():
        target_dir = destination / target
        target_dir.mkdir(parents=True)
        for entry in entries:
            source = REPO / entry["path"]
            shutil.copyfile(source, target_dir / Path(entry["path"]).name)


def exact_tool(name: str, command: list[str]) -> dict[str, str]:
    proc = subprocess.run(command, cwd=REPO, capture_output=True, text=True, check=False)
    version = (proc.stdout or proc.stderr).strip()
    if proc.returncode != 0 or not version:
        raise RuntimeError(f"cannot identify {name}: exit {proc.returncode}")
    tool = {"name": name, "version": version}
    tool["identity_sha256"] = tool_identity_digest(tool)
    return tool


def emit_evidence(
    *,
    producer_path: Path,
    corpus_path: Path,
    logs_dir: Path,
    output_dir: Path,
    source_before_path: Path,
    duration_seconds: int,
    started_at: str,
    finished_at: str,
    process_exit: int,
) -> int:
    output_dir, logs_dir, _ = validate_artifact_layout(
        output_dir=output_dir,
        logs_dir=logs_dir,
        corpus_dir=output_dir / "seed-corpus",
    )
    source_before_path = safe_descendant(source_before_path, output_dir, "source snapshot")
    source_before = load_json(source_before_path)
    producer, corpus = load_json(producer_path), load_json(corpus_path)
    problems = validate_manifests(producer, corpus)
    target_results: list[dict[str, Any]] = []
    required: dict[str, list[str]] = producer["required_targets"]
    for target in sorted(required):
        path = logs_dir / f"{target}.log"
        if not path.is_file():
            problems.append(f"{target}: raw log is missing")
            continue
        attempted = path.stat().st_size > 0
        if not attempted:
            problems.append(f"{target}: raw log is empty; target was not attempted")
        result, target_problems = parse_target_log(
            target, path.read_text(errors="replace"), required[target], duration_seconds
        )
        result["attempted"] = attempted
        target_source = REPO / "fuzz" / "fuzz_targets" / f"{target}.rs"
        result["source"] = {
            "path": target_source.relative_to(REPO).as_posix(),
            "sha256": sha256(target_source),
        }
        target_results.append(result)
        problems.extend(target_problems)
    if process_exit != 0:
        problems.append(f"fuzz subprocess exit was {process_exit}")
    source_after = source_identity(REPO)
    source_problems = source_drift_problems(source_before, source_after)
    problems.extend(source_problems)

    output_dir.mkdir(parents=True, exist_ok=True)
    receipt_path = output_dir / "fuzz-receipt.json"
    status = "PASS" if not problems else "FAIL"
    receipt = {
        "schema_version": 1,
        "kind": "taskmesh-fuzz-evidence",
        "producer_id": producer["producer_id"],
        "required_targets": sorted(required),
        "executed_targets": [item["target"] for item in target_results if item["attempted"]],
        "duration_seconds_per_target": duration_seconds,
        "corpus": {
            "manifest": corpus_path.relative_to(REPO).as_posix(),
            "digest": corpus["digest"],
        },
        "targets": target_results,
        "source_stable": not source_problems,
        "status": status,
        "problems": problems,
    }
    receipt_path.write_bytes(canonical_bytes(receipt) + b"\n")

    argv = ["bash", "tools/fuzz/run.sh"]
    environment = {"FUZZ_SECONDS": str(duration_seconds), "TASKMESH_FUZZ_WITNESS": "1"}
    command: dict[str, Any] = {"argv": argv, "cwd": ".", "environment": environment}
    command["sha256"] = command_digest(command)
    artifacts = []
    for target in sorted(required):
        path = logs_dir / f"{target}.log"
        if path.is_file() and path.stat().st_size > 0:
            artifacts.append({**file_identity(path, relative_to=REPO), "role": "raw"})
    artifacts.append({**file_identity(receipt_path, relative_to=REPO), "role": "summary"})
    workflow = REPO / ".github/workflows/ci.yml"
    envelope = {
        "schema_version": 1,
        "kind": "taskmesh-evidence-envelope",
        "producer": {"id": producer["producer_id"], "version": "1"},
        "source": {
            "head": source_before["head"],
            "tree_digest": source_before["paths_digest"],
            "dirty": source_before["dirty"],
        },
        "command": command,
        "tools": [
            exact_tool("cargo-fuzz", ["cargo", "+nightly", "fuzz", "--version"]),
            exact_tool("rustc-nightly", ["rustup", "run", "nightly", "rustc", "-Vv"]),
        ],
        "configs": [
            {"path": producer_path.relative_to(REPO).as_posix(), "sha256": sha256(producer_path)},
            {"path": corpus_path.relative_to(REPO).as_posix(), "sha256": sha256(corpus_path)},
        ],
        "action": {
            "workflow": workflow.relative_to(REPO).as_posix(),
            "workflow_sha256": sha256(workflow),
            "job": "fuzz",
            "event": "local"
            if not __import__("os").environ.get("GITHUB_EVENT_NAME")
            else __import__("os").environ["GITHUB_EVENT_NAME"],
            "actions": [],
        },
        "artifacts": artifacts,
        "result": {
            "status": status,
            "exit_code": 0 if status == "PASS" else 1,
            "started_at": started_at,
            "finished_at": finished_at,
            "selected_count": len(required),
            "executed_count": sum(item["attempted"] for item in target_results),
        },
    }
    envelope_path = output_dir / "evidence-envelope.json"
    envelope_path.write_bytes(canonical_bytes(envelope) + b"\n")
    envelope_issues = envelope_problems(envelope, artifact_root=REPO)
    if envelope_issues:
        print("fuzz evidence envelope invalid: " + "; ".join(envelope_issues), file=sys.stderr)
        return 1
    if problems:
        print("fuzz evidence rejected: " + "; ".join(problems), file=sys.stderr)
        return 1
    relative_receipt = receipt_path.relative_to(REPO)
    print(
        f"taskmesh-fuzz status=PASS targets={len(required)} "
        f"corpus_sha256={corpus['digest']} receipt={relative_receipt}"
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    verify = sub.add_parser("verify")
    verify.add_argument("--actual-target", action="append", default=[])
    prepare = sub.add_parser("prepare-corpus")
    prepare.add_argument("--destination", type=Path, required=True)
    prepare.add_argument("--output-dir", type=Path, required=True)
    paths = sub.add_parser("validate-paths")
    paths.add_argument("--output-dir", type=Path, required=True)
    paths.add_argument("--logs-dir", type=Path, required=True)
    paths.add_argument("--corpus-dir", type=Path, required=True)
    snapshot = sub.add_parser("capture-source")
    snapshot.add_argument("--output-dir", type=Path, required=True)
    snapshot.add_argument("--destination", type=Path, required=True)
    collect = sub.add_parser("collect")
    collect.add_argument("--logs-dir", type=Path, required=True)
    collect.add_argument("--output-dir", type=Path, required=True)
    collect.add_argument("--source-before", type=Path, required=True)
    collect.add_argument("--duration-seconds", type=int, required=True)
    collect.add_argument("--started-at", required=True)
    collect.add_argument("--finished-at", required=True)
    collect.add_argument("--process-exit", type=int, required=True)
    args = parser.parse_args()
    producer_path = REPO / "tools/fuzz/producer-manifest.json"
    corpus_path = REPO / "fuzz/corpus-manifest.json"
    producer, corpus = load_json(producer_path), load_json(corpus_path)
    problems = validate_manifests(producer, corpus)
    if args.command == "verify":
        problems.extend(validate_target_set(args.actual_target, producer["required_targets"]))
    elif args.command == "prepare-corpus":
        if not problems:
            try:
                prepare_corpus(
                    corpus,
                    args.destination,
                    output_dir=args.output_dir,
                )
            except ValueError as error:
                problems.append(str(error))
    elif args.command == "validate-paths":
        try:
            validate_artifact_layout(
                output_dir=args.output_dir,
                logs_dir=args.logs_dir,
                corpus_dir=args.corpus_dir,
            )
        except ValueError as error:
            problems.append(str(error))
    elif args.command == "capture-source":
        try:
            output = safe_descendant(args.output_dir, ARTIFACT_ROOT, "output directory")
            destination = safe_descendant(args.destination, output, "source snapshot")
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(canonical_bytes(source_identity(REPO)) + b"\n")
        except ValueError as error:
            problems.append(str(error))
    else:
        try:
            return emit_evidence(
                producer_path=producer_path,
                corpus_path=corpus_path,
                logs_dir=args.logs_dir,
                output_dir=args.output_dir,
                source_before_path=args.source_before,
                duration_seconds=args.duration_seconds,
                started_at=args.started_at,
                finished_at=args.finished_at,
                process_exit=args.process_exit,
            )
        except ValueError as error:
            problems.append(str(error))
    if problems:
        print("fuzz evidence rejected: " + "; ".join(problems), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
