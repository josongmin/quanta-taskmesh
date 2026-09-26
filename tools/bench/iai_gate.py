#!/usr/bin/env python3
"""Helpers behind `tools/bench-iai.sh`: runner-version detection and the
baseline-compatibility fingerprint.

Kept in Python (and unit-tested) so the shell script stays a thin sequence of
steps, and so the one thing that used to fail silently — asking
`iai-callgrind-runner --version`, which the runner does not support and answers
with a non-zero exit and empty stdout — is now a function with a tested answer
for every input shape.

Commands:
    iai_gate.py runner-version    # installed runner version, or exit 2 with a reason
    iai_gate.py fingerprint --runner X --valgrind V --rustc-file F   # prints it
    iai_gate.py config KEY                     # prints one instruction_count_gate value
    iai_gate.py config-fields                  # prints the four shell gate fields once
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from decimal import Decimal, InvalidOperation
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CONFIG = REPO / "tools" / "bench" / "perf-gate.json"
RUNNER = "iai-callgrind-runner"
BASELINE_MANIFEST_VERSION = 3
COMPARISON_MANIFEST_VERSION = 3
BASELINE_MANIFEST = "baseline-manifest.json"
COMPARISON_MANIFEST = "comparison-manifest.json"

# The full version, pre-release/build suffix included, so `0.14.2-rc.1` is
# reported as itself and never accepted as `0.14.2`.
_INSTALL_LIST_LINE = re.compile(
    r"^iai-callgrind-runner v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?(?:\+[0-9A-Za-z.]+)?)(?: \([^)]*\))?:"
)
# The runner's own complaint when invoked without a library version, e.g.
# `... but iai-callgrind-runner (0.14.2) is >= '0.3.0'`.
_VERSION = r"(\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?(?:\+[0-9A-Za-z.]+)?)"
_SELF_REPORT = re.compile(rf"iai-callgrind-runner \({_VERSION}\)")
_BARE_REPORT = re.compile(rf"Detected version of iai-callgrind-runner is {_VERSION}")
_IR_LIMIT = re.compile(r"Ir=(0|[1-9][0-9]*)(?:\.([0-9]+))?\Z")
_U64_MAX = (1 << 64) - 1


def gate_config() -> dict:
    return json.loads(CONFIG.read_text(encoding="utf-8"))["instruction_count_gate"]


def shell_config_fields() -> tuple[str, str, str, str]:
    """Return the shell gate's scalar fields as safe, line-delimited values."""
    config = gate_config()
    fields = ("bench", "iai_callgrind_runner", "regression")
    values = []
    for field in fields:
        value = config[field]
        if not isinstance(value, str) or not value.strip() or not value.isprintable():
            raise ValueError(f"{field} must be a nonempty printable string")
        values.append(value)
    schema = config["measurement_schema"]
    if type(schema) is not int or schema <= 0:
        raise ValueError("measurement_schema must be a positive integer")
    return (*values, str(schema))


def runner_version_from_install_list(text: str) -> str | None:
    """Parse `cargo install --list` output; `None` when the runner is absent."""
    for line in text.splitlines():
        match = _INSTALL_LIST_LINE.match(line.strip())
        if match:
            return match.group(1)
    return None


def runner_version_from_self_report(text: str) -> str | None:
    """Parse the version the runner names in its own usage error."""
    match = _SELF_REPORT.search(text) or _BARE_REPORT.search(text)
    return match.group(1) if match else None


def detect_runner_version() -> tuple[str | None, str]:
    """Returns (version, explanation). `version` is None when it could not be
    determined; the explanation always says what was tried."""
    tried: list[str] = []
    # The executable on PATH is what iai-callgrind actually executes. Cargo's
    # installation registry may describe a different, shadowed executable.
    try:
        probe = subprocess.run([RUNNER], capture_output=True, text=True, check=False)
        path_version = runner_version_from_self_report(probe.stdout + probe.stderr)
        if path_version is None:
            return None, "the runner on PATH did not report its version"
    except OSError as error:
        return None, f"the runner on PATH could not be executed: {error}"
    try:
        listing = subprocess.run(
            ["cargo", "install", "--list"], capture_output=True, text=True, check=False
        )
        installed_version = runner_version_from_install_list(listing.stdout)
        if installed_version and installed_version != path_version:
            return (
                None,
                f"PATH runner {path_version} differs from cargo install {installed_version}",
            )
        if installed_version:
            tried.append(f"cargo install agrees ({installed_version})")
        else:
            tried.append("runner is not listed by cargo install")
    except OSError as error:
        tried.append(f"`cargo install --list` failed: {error}")
    return path_version, "; ".join(tried)


def cargo_build_context(root: Path, runner_executable: Path | None = None) -> dict[str, object]:
    """Capture build inputs that can change codegen without changing rustc -Vv."""
    names = {
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTC",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "RUSTC_BOOTSTRAP",
        "CARGO_INCREMENTAL",
        "CARGO_HOME",
        "CARGO_BUILD_RUSTFLAGS",
        "CARGO_BUILD_TARGET",
        "CARGO_BUILD_RUSTC",
        "CARGO_BUILD_RUSTC_WRAPPER",
        "CC",
        "CXX",
        "CFLAGS",
        "CXXFLAGS",
        "AR",
        "RANLIB",
    }
    environment = {
        key: value
        for key, value in sorted(os.environ.items())
        if key in names
        or key.startswith(("CARGO_PROFILE_BENCH_", "CARGO_PROFILE_RELEASE_"))
        or (key.startswith("CARGO_TARGET_") and key.endswith(("_RUSTFLAGS", "_LINKER", "_RUNNER")))
    }
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    directories = [cargo_home]
    directories.extend(parent / ".cargo" for parent in (root.resolve(), *root.resolve().parents))
    configs: dict[str, str] = {}
    for directory in directories:
        for name in ("config", "config.toml"):
            path = directory / name
            if path.is_file():
                configs[str(path.resolve())] = hashlib.sha256(path.read_bytes()).hexdigest()
    if runner_executable is None:
        located = shutil.which(RUNNER)
        runner_executable = Path(located) if located else None
    runner_sha256 = (
        hashlib.sha256(runner_executable.read_bytes()).hexdigest()
        if runner_executable is not None and runner_executable.is_file()
        else None
    )
    return {
        "environment": environment,
        "cargo_configs": dict(sorted(configs.items())),
        "runner_sha256": runner_sha256,
    }


def build_context_valid(value: object) -> bool:
    if not isinstance(value, dict) or set(value) != {
        "environment",
        "cargo_configs",
        "runner_sha256",
    }:
        return False
    environment = value["environment"]
    configs = value["cargo_configs"]
    return (
        isinstance(value["runner_sha256"], str)
        and re.fullmatch(r"[0-9a-f]{64}", value["runner_sha256"]) is not None
        and isinstance(environment, dict)
        and all(isinstance(key, str) and isinstance(item, str) for key, item in environment.items())
        and isinstance(configs, dict)
        and all(
            isinstance(key, str)
            and isinstance(item, str)
            and re.fullmatch(r"[0-9a-f]{64}", item) is not None
            for key, item in configs.items()
        )
    )


def fingerprint(
    inputs: list[Path],
    schema: object,
    runner: str,
    valgrind: str,
    rustc: str,
    *,
    build_context: dict[str, object],
) -> str:
    """Hash of everything that defines whether two baselines are comparable:
    the measured definition and its dependency graph (the input files), the
    measurement schema, the runner, valgrind, and the compiler."""
    if not inputs:
        raise ValueError("fingerprint needs at least one input file")
    if not valgrind.strip() or not rustc.strip():
        raise ValueError("fingerprint needs non-empty valgrind and rustc descriptions")
    if not build_context_valid(build_context):
        raise ValueError("fingerprint needs a complete Cargo build context")
    digest = hashlib.sha256()
    for path in inputs:
        digest.update(f"{path.as_posix()}\n".encode())
        digest.update(hashlib.sha256(path.read_bytes()).hexdigest().encode())
        digest.update(b"\n")
    digest.update(f"schema={schema}\n".encode())
    digest.update(f"runner={runner}\n".encode())
    digest.update(f"valgrind={valgrind.strip()}\n".encode())
    digest.update(rustc.strip().encode())
    digest.update(b"\n")
    digest.update(json.dumps(build_context, sort_keys=True, separators=(",", ":")).encode())
    digest.update(b"\n")
    return digest.hexdigest()


def _sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _artifact(path: Path, root: Path) -> dict[str, object]:
    return {
        "path": path.relative_to(root).as_posix(),
        "sha256": _sha256(path),
        "size": path.stat().st_size,
    }


def confined_artifact(root: Path, relative: str) -> Path | None:
    """Resolve a manifest artifact only when it stays inside the baseline store."""
    try:
        named = Path(relative)
        if not relative or named.is_absolute() or ".." in named.parts or root.is_symlink():
            return None
        artifact = root / named
        component = root
        for part in named.parts:
            component /= part
            if component.is_symlink():
                return None
        if not artifact.resolve().is_relative_to(root.resolve()):
            return None
    except (OSError, ValueError):
        return None
    return artifact


def baseline_artifacts(root: Path) -> list[dict[str, object]]:
    """Raw runner outputs that make a baseline real rather than stamp-only."""
    excluded = {BASELINE_MANIFEST, COMPARISON_MANIFEST, "benchmark-output.log"}
    paths = [
        path
        for path in root.rglob("*")
        if path.is_file()
        and not path.is_symlink()
        and path.resolve().is_relative_to(root.resolve())
        and path.name not in excluded
        and path.suffix in {".out", ".log"}
        and path.stat().st_size > 0
    ]
    return [_artifact(path, root) for path in sorted(paths)]


def baseline_manifest_problems(
    root: Path,
    *,
    fingerprint_value: str,
    runner: str,
    valgrind: str,
    rustc: str,
    build_context: dict[str, object],
    config_path: Path = CONFIG,
) -> list[str]:
    path = root / BASELINE_MANIFEST
    if path.is_symlink() or not path.is_file():
        return ["baseline manifest is missing"]
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        return [f"baseline manifest is unreadable: {error}"]
    if not isinstance(manifest, dict):
        return ["baseline manifest is malformed"]
    problems: list[str] = []
    expected = {
        "schema_version": BASELINE_MANIFEST_VERSION,
        "kind": "iai-callgrind-baseline",
        "state": "BASELINE_READY",
        "fingerprint": fingerprint_value,
        "runner": runner,
        "valgrind": valgrind.strip(),
        "rustc": rustc.strip(),
        "build_context": build_context,
        "config_sha256": _sha256(config_path),
    }
    for key, value in expected.items():
        if manifest.get(key) != value:
            problems.append(f"baseline {key} mismatch")
    artifacts = manifest.get("artifacts")
    if not isinstance(artifacts, list) or not artifacts:
        return [*problems, "baseline artifact list is missing or empty"]
    if not any(
        isinstance(item, dict) and str(item.get("path", "")).endswith(".out") for item in artifacts
    ):
        problems.append("baseline has no raw callgrind .out artifact")
    for item in artifacts:
        if not isinstance(item, dict) or not isinstance(item.get("path"), str):
            problems.append("baseline artifact entry is malformed")
            continue
        artifact = confined_artifact(root, item["path"])
        if artifact is None:
            problems.append(f"baseline artifact {item['path']!r} escapes the baseline store")
            continue
        if not artifact.is_file():
            problems.append(f"baseline artifact {item['path']!r} is missing")
            continue
        if artifact.stat().st_size != item.get("size"):
            problems.append(f"baseline artifact {item['path']!r} size mismatch")
        if _sha256(artifact) != item.get("sha256"):
            problems.append(f"baseline artifact {item['path']!r} digest mismatch")
    return problems


def _total_metrics(summary: object) -> dict | None:
    if not isinstance(summary, dict):
        return None
    callgrind = summary.get("callgrind_summary")
    if not isinstance(callgrind, dict):
        return None
    run = callgrind.get("callgrind_run")
    total = run.get("total") if isinstance(run, dict) else None
    metrics = total.get("summary") if isinstance(total, dict) else None
    return metrics if isinstance(metrics, dict) and metrics else None


def _instruction_counts_present(summary: object) -> bool:
    metrics = _total_metrics(summary)
    if metrics is None:
        return False
    ir = metrics.get("Ir")
    values = ir.get("metrics") if isinstance(ir, dict) else None
    if not isinstance(values, dict):
        return False
    # iai-callgrind 0.14.2 serializes EitherOrBoth<u64> as Left or Both.
    # A measured governance operation must execute at least one instruction.
    if set(values) == {"Left"}:
        return type(values["Left"]) is int and values["Left"] > 0
    if set(values) == {"Both"}:
        both = values["Both"]
        return (
            isinstance(both, list)
            and len(both) == 2
            and all(type(value) is int and value > 0 for value in both)
        )
    return False


def _summary_comparison_complete(summary: object) -> bool:
    metrics = _total_metrics(summary)
    if metrics is None or not _instruction_counts_present(summary):
        return False
    ir = metrics["Ir"]["metrics"].get("Both")
    if not isinstance(ir, list) or len(ir) != 2:
        return False
    for metric in metrics.values():
        values = metric.get("metrics") if isinstance(metric, dict) else None
        both = values.get("Both") if isinstance(values, dict) else None
        if (
            not isinstance(both, list)
            or len(both) != 2
            or not all(type(value) is int and value >= 0 for value in both)
        ):
            return False
    return True


def _case_identity(summary: object) -> str | None:
    if not isinstance(summary, dict) or summary.get("kind") != "LibraryBenchmark":
        return None
    module_path = summary.get("module_path")
    case_id = summary.get("id")
    function_name = summary.get("function_name")
    if (
        not isinstance(module_path, str)
        or not module_path
        or not isinstance(case_id, str)
        or not case_id
        or not isinstance(function_name, str)
        or module_path.rsplit("::", 1)[-1] != function_name
    ):
        return None
    return f"{module_path}#{case_id}"


def _raw_output_paths(summary: object, root: Path) -> list[str] | None:
    callgrind = summary.get("callgrind_summary") if isinstance(summary, dict) else None
    out_paths = callgrind.get("out_paths") if isinstance(callgrind, dict) else None
    if not isinstance(out_paths, list) or not out_paths:
        return None
    checked: list[str] = []
    for named in out_paths:
        if not isinstance(named, str) or not named:
            return None
        path = Path(named)
        try:
            relative = path.relative_to(root.resolve()) if path.is_absolute() else path
        except ValueError:
            return None
        artifact = confined_artifact(root, relative.as_posix())
        if (
            artifact is None
            or artifact.suffix != ".out"
            or not artifact.is_file()
            or artifact.stat().st_size == 0
        ):
            return None
        checked.append(relative.as_posix())
    return checked


def _u64(value: object) -> bool:
    return type(value) is int and 0 <= value <= _U64_MAX


def _raw_ir(path: Path) -> int | None:
    """Mirror iai-callgrind 0.14.2 SummaryParser's totals/summary precedence."""
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeError):
        return None
    events: list[str] | None = None
    summary: list[str] | None = None
    totals: list[str] | None = None
    version = False
    for line in lines:
        if line.startswith("version:"):
            version = line.split(":", 1)[1].strip() == "1"
        elif line.startswith("events:"):
            if events is not None:
                return None
            events = line.split(":", 1)[1].split()
        elif line.startswith("summary:"):
            summary = line.split(":", 1)[1].split()
        elif line.startswith("totals:"):
            totals = line.split(":", 1)[1].split()
            break
    if not version or events is None or events.count("Ir") != 1:
        return None
    values = totals if totals is not None else summary
    if values is None or len(values) > len(events):
        return None
    try:
        counts = [int(value) for value in values]
    except ValueError:
        return None
    if any(not _u64(value) for value in counts):
        return None
    index = events.index("Ir")
    return counts[index] if index < len(counts) else 0


def _old_output_paths(summary: dict, root: Path) -> list[str] | None:
    run = summary["callgrind_summary"].get("callgrind_run")
    segments = run.get("segments") if isinstance(run, dict) else None
    if not isinstance(segments, list) or not segments:
        return None
    result: list[str] = []
    for segment in segments:
        baseline = segment.get("baseline") if isinstance(segment, dict) else None
        if baseline is None:
            continue
        named = baseline.get("path") if isinstance(baseline, dict) else None
        if not isinstance(named, str):
            return None
        path = Path(named)
        try:
            relative = path.relative_to(root.resolve()) if path.is_absolute() else path
        except ValueError:
            return None
        artifact = confined_artifact(root, relative.as_posix())
        if (
            artifact is None
            or not relative.as_posix().endswith(".out.old")
            or not artifact.is_file()
        ):
            return None
        result.append(relative.as_posix())
    return result or None


def _ir_limit() -> Decimal:
    value = gate_config().get("regression")
    if not isinstance(value, str) or _IR_LIMIT.fullmatch(value) is None:
        raise ValueError("IAI regression must be a non-negative Ir percentage")
    try:
        return Decimal(value.split("=", 1)[1])
    except InvalidOperation as exc:
        raise ValueError("IAI regression percentage is invalid") from exc


def _ir_semantic_problems(
    summary: dict,
    root: Path,
    current_paths: list[str],
    *,
    comparison: bool,
) -> tuple[list[str], list[str]]:
    problems: list[str] = []
    old_paths = _old_output_paths(summary, root) if comparison else []
    if comparison and old_paths is None:
        return [], ["old Callgrind output paths are missing or unsafe"]
    assert old_paths is not None
    if len(current_paths) != len(set(current_paths)) or len(old_paths) != len(set(old_paths)):
        problems.append("duplicate raw Callgrind output path")
    sums: list[int] = []
    for label, paths in (("current", current_paths), ("old", old_paths)):
        if not paths:
            if label == "old" and not comparison:
                continue
            problems.append(f"{label} Callgrind output is missing")
            continue
        counts = [_raw_ir(root / path) for path in paths]
        if any(count is None for count in counts):
            problems.append(f"{label} Callgrind Ir is unreadable")
            continue
        total = sum(count for count in counts if count is not None)
        if not _u64(total):
            problems.append(f"{label} Callgrind Ir total overflows u64")
        sums.append(total)
    metrics = _total_metrics(summary)
    ir = metrics.get("Ir") if isinstance(metrics, dict) else None
    values = ir.get("metrics") if isinstance(ir, dict) else None
    expected_key = "Both" if comparison else "Left"
    pair = values.get(expected_key) if isinstance(values, dict) else None
    expected = (
        pair
        if comparison and isinstance(pair, list) and len(pair) == 2
        else [pair]
        if not comparison
        else None
    )
    if not isinstance(expected, list) or any(not _u64(value) for value in expected):
        problems.append("summary Ir metric is missing or malformed")
    elif len(sums) == len(expected) and sums != expected:
        problems.append("summary Ir differs from raw Callgrind totals")
    if (
        comparison
        and isinstance(expected, list)
        and len(expected) == 2
        and all(_u64(v) for v in expected)
    ):
        new, old = expected
        if old == 0 or Decimal(new) * 100 > Decimal(old) * (100 + _ir_limit()):
            problems.append("Ir regression exceeds reviewed threshold")
    return old_paths, problems


def inspect_summaries(root: Path, *, expected_comparison: bool = False) -> dict[str, object]:
    paths = sorted(root.rglob("summary.json"))
    invalid: list[str] = []
    compared = 0
    cases: list[str] = []
    raw_outputs: list[str] = []
    old_outputs: list[str] = []
    semantic_problems: list[str] = []
    for path in paths:
        relative = path.relative_to(root).as_posix()
        if confined_artifact(root, relative) is None:
            invalid.append(relative)
            continue
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, json.JSONDecodeError):
            invalid.append(relative)
            continue
        case = _case_identity(payload)
        outputs = _raw_output_paths(payload, root)
        if case is None or not _instruction_counts_present(payload) or outputs is None:
            invalid.append(relative)
            continue
        cases.append(case)
        raw_outputs.extend(outputs)
        old, problems = _ir_semantic_problems(
            payload, root, outputs, comparison=expected_comparison
        )
        old_outputs.extend(old)
        semantic_problems.extend(f"{relative}: {problem}" for problem in problems)
        if _summary_comparison_complete(payload):
            compared += 1
    return {
        "selected_count": len(paths),
        "executed_count": len(paths) - len(invalid),
        "comparison_count": compared,
        "cases": cases,
        "raw_outputs": raw_outputs,
        "old_outputs": old_outputs,
        "semantic_problems": semantic_problems,
        "invalid_summaries": invalid,
        "summaries": [path.relative_to(root).as_posix() for path in paths],
    }


def finalize_run(
    root: Path,
    *,
    fingerprint_value: str,
    runner: str,
    valgrind: str,
    rustc: str,
    build_context: dict[str, object],
    expected_comparison: bool,
) -> tuple[str, list[str]]:
    if not build_context_valid(build_context):
        return "NOT_RUN", ["Cargo build context is malformed"]
    inspection = inspect_summaries(root, expected_comparison=expected_comparison)
    selected = inspection["selected_count"]
    executed = inspection["executed_count"]
    compared = inspection["comparison_count"]
    problems: list[str] = []
    problems.extend(inspection["semantic_problems"])
    if not isinstance(selected, int) or selected <= 0:
        problems.append("runner produced no summary.json artifacts")
    if executed != selected:
        problems.append("one or more summary artifacts are invalid")
    if expected_comparison and compared != selected:
        problems.append("verified old-vs-new comparison is incomplete")
    if not expected_comparison and compared:
        problems.append("fresh-baseline run unexpectedly claims an old-vs-new comparison")
    expected_cases = gate_config().get("expected_cases")
    if (
        not isinstance(expected_cases, list)
        or not expected_cases
        or any(not isinstance(case, str) or not case for case in expected_cases)
        or len(set(expected_cases)) != len(expected_cases)
        or sorted(inspection["cases"]) != sorted(expected_cases)
    ):
        problems.append("benchmark case inventory mismatch")
    raw_outputs = inspection["raw_outputs"]
    if len(raw_outputs) != len(set(raw_outputs)):
        problems.append("raw callgrind output is shared across benchmark cases")
    old_outputs = inspection["old_outputs"]
    if len(old_outputs) != len(set(old_outputs)):
        problems.append("old callgrind output is shared across benchmark cases")
    artifacts = baseline_artifacts(root)
    if not any(str(item["path"]).endswith(".out") for item in artifacts):
        problems.append("raw callgrind .out artifact is missing")
    status = "NOT_RUN" if problems else ("QUALIFIED" if expected_comparison else "BASELINE_CREATED")
    raw_log = root / "benchmark-output.log"
    safe_raw_log = confined_artifact(root, "benchmark-output.log")
    if safe_raw_log is None or not raw_log.is_file() or raw_log.stat().st_size == 0:
        problems.append("raw benchmark output is missing")
        status = "NOT_RUN"
    comparison = {
        "schema_version": COMPARISON_MANIFEST_VERSION,
        "kind": "iai-callgrind-comparison",
        "status": status,
        "fingerprint": fingerprint_value,
        "build_context": build_context,
        **inspection,
        "summary_artifacts": [
            _artifact(root / path, root)
            for path in inspection["summaries"]
            if confined_artifact(root, path) is not None and (root / path).is_file()
        ],
        "old_artifacts": [_artifact(root / path, root) for path in inspection["old_outputs"]],
        "raw_output": _artifact(raw_log, root)
        if safe_raw_log is not None and raw_log.is_file() and raw_log.stat().st_size
        else None,
        "problems": problems,
    }
    (root / COMPARISON_MANIFEST).write_text(
        json.dumps(comparison, indent=2) + "\n", encoding="utf-8"
    )
    # A successful benchmark always becomes the next baseline, but never
    # upgrades BASELINE_CREATED into comparison proof.
    baseline = {
        "schema_version": BASELINE_MANIFEST_VERSION,
        "kind": "iai-callgrind-baseline",
        "state": "BASELINE_READY",
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "fingerprint": fingerprint_value,
        "runner": runner,
        "valgrind": valgrind.strip(),
        "rustc": rustc.strip(),
        "build_context": build_context,
        "config_sha256": _sha256(CONFIG),
        "artifacts": artifacts,
    }
    if status != "NOT_RUN":
        (root / BASELINE_MANIFEST).write_text(
            json.dumps(baseline, indent=2) + "\n", encoding="utf-8"
        )
    return status, problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("runner-version")
    fp = sub.add_parser("fingerprint")
    fp.add_argument("--runner", required=True)
    fp.add_argument("--valgrind", required=True)
    fp.add_argument("--rustc-file", required=True, type=Path)
    fp.add_argument("--root", type=Path, default=REPO)
    cfg = sub.add_parser("config")
    cfg.add_argument("key")
    sub.add_parser("config-fields")
    baseline = sub.add_parser("validate-baseline")
    baseline.add_argument("--root", required=True, type=Path)
    baseline.add_argument("--fingerprint", required=True)
    baseline.add_argument("--runner", required=True)
    baseline.add_argument("--valgrind", required=True)
    baseline.add_argument("--rustc-file", required=True, type=Path)
    finalize = sub.add_parser("finalize")
    finalize.add_argument("--root", required=True, type=Path)
    finalize.add_argument("--fingerprint", required=True)
    finalize.add_argument("--runner", required=True)
    finalize.add_argument("--valgrind", required=True)
    finalize.add_argument("--rustc-file", required=True, type=Path)
    finalize.add_argument("--expected-comparison", choices=("yes", "no"), required=True)
    args = parser.parse_args(argv)

    if args.command == "config":
        value = gate_config()[args.key]
        print(" ".join(value) if isinstance(value, list) else value)
        return 0
    if args.command == "config-fields":
        try:
            print("\n".join(shell_config_fields()))
        except (OSError, KeyError, ValueError, TypeError) as error:
            print(f"bench-iai: invalid gate config: {error}", file=sys.stderr)
            return 2
        return 0
    if args.command == "runner-version":
        version, explanation = detect_runner_version()
        if version is None:
            print(
                f"bench-iai: cannot determine the {RUNNER} version: {explanation}",
                file=sys.stderr,
            )
            return 2
        print(version)
        return 0
    if args.command in {"validate-baseline", "finalize"}:
        try:
            config = gate_config()
            context = cargo_build_context(REPO)
            actual_fingerprint = fingerprint(
                [REPO / path for path in config["fingerprint_inputs"]],
                config["measurement_schema"],
                args.runner,
                args.valgrind,
                args.rustc_file.read_text(encoding="utf-8"),
                build_context=context,
            )
        except (OSError, KeyError, TypeError, ValueError) as error:
            print(f"bench-iai: cannot verify current fingerprint: {error}", file=sys.stderr)
            return 2
        if actual_fingerprint != args.fingerprint:
            print(
                "bench-iai: source, build context, or runner changed during the run",
                file=sys.stderr,
            )
            return 2
    if args.command == "validate-baseline":
        problems = baseline_manifest_problems(
            args.root,
            fingerprint_value=args.fingerprint,
            runner=args.runner,
            valgrind=args.valgrind,
            rustc=args.rustc_file.read_text(encoding="utf-8"),
            build_context=context,
        )
        if problems:
            print("; ".join(problems), file=sys.stderr)
            return 2
        return 0
    if args.command == "finalize":
        status, problems = finalize_run(
            args.root,
            fingerprint_value=args.fingerprint,
            runner=args.runner,
            valgrind=args.valgrind,
            rustc=args.rustc_file.read_text(encoding="utf-8"),
            build_context=context,
            expected_comparison=args.expected_comparison == "yes",
        )
        print(status)
        if problems:
            print("; ".join(problems), file=sys.stderr)
            return 2
        return 0
    config = gate_config()
    inputs = [args.root / p for p in config["fingerprint_inputs"]]
    missing = [str(p) for p in inputs if not p.is_file()]
    if missing:
        print(f"bench-iai: fingerprint input(s) missing: {missing}", file=sys.stderr)
        return 2
    try:
        print(
            fingerprint(
                inputs,
                config["measurement_schema"],
                args.runner,
                args.valgrind,
                args.rustc_file.read_text(encoding="utf-8"),
                build_context=cargo_build_context(args.root),
            )
        )
    except ValueError as error:
        print(f"bench-iai: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
