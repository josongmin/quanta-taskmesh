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
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CONFIG = REPO / "tools" / "bench" / "perf-gate.json"
RUNNER = "iai-callgrind-runner"
BASELINE_MANIFEST_VERSION = 1
BASELINE_MANIFEST = "baseline-manifest.json"
COMPARISON_MANIFEST = "comparison-manifest.json"

# The full version, pre-release/build suffix included, so `0.14.2-rc.1` is
# reported as itself and never accepted as `0.14.2`.
_INSTALL_LIST_LINE = re.compile(
    r"^iai-callgrind-runner v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?(?:\+[0-9A-Za-z.]+)?)(?: \([^)]*\))?:"
)
# The runner's own complaint when invoked without a library version, e.g.
# `... but iai-callgrind-runner (0.14.2) is >= '0.3.0'`.
_SELF_REPORT = re.compile(
    r"iai-callgrind-runner \((\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?(?:\+[0-9A-Za-z.]+)?)\)"
)


def gate_config() -> dict:
    return json.loads(CONFIG.read_text(encoding="utf-8"))["instruction_count_gate"]


def runner_version_from_install_list(text: str) -> str | None:
    """Parse `cargo install --list` output; `None` when the runner is absent."""
    for line in text.splitlines():
        match = _INSTALL_LIST_LINE.match(line.strip())
        if match:
            return match.group(1)
    return None


def runner_version_from_self_report(text: str) -> str | None:
    """Parse the version the runner names in its own usage error."""
    match = _SELF_REPORT.search(text)
    return match.group(1) if match else None


def detect_runner_version() -> tuple[str | None, str]:
    """Returns (version, explanation). `version` is None when it could not be
    determined; the explanation always says what was tried."""
    tried: list[str] = []
    try:
        listing = subprocess.run(
            ["cargo", "install", "--list"], capture_output=True, text=True, check=False
        )
        version = runner_version_from_install_list(listing.stdout)
        if version:
            return version, f"from `cargo install --list` ({RUNNER} v{version})"
        tried.append("`cargo install --list` does not list it")
    except OSError as error:
        tried.append(f"`cargo install --list` failed: {error}")
    try:
        # The runner has no --version flag; invoked bare it reports its own
        # version inside an error message, which is the only self-report it has.
        probe = subprocess.run([RUNNER], capture_output=True, text=True, check=False)
        version = runner_version_from_self_report(probe.stdout + probe.stderr)
        if version:
            return version, f"from the runner's own report ({RUNNER} {version})"
        tried.append(f"bare `{RUNNER}` did not name its version")
    except OSError as error:
        tried.append(f"`{RUNNER}` could not be executed: {error}")
    return None, "; ".join(tried)


def fingerprint(inputs: list[Path], schema: object, runner: str, valgrind: str, rustc: str) -> str:
    """Hash of everything that defines whether two baselines are comparable:
    the measured definition and its dependency graph (the input files), the
    measurement schema, the runner, valgrind, and the compiler."""
    if not inputs:
        raise ValueError("fingerprint needs at least one input file")
    if not valgrind.strip() or not rustc.strip():
        raise ValueError("fingerprint needs non-empty valgrind and rustc descriptions")
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
        if not relative or named.is_absolute() or ".." in named.parts:
            return None
        artifact = root / named
        if artifact.is_symlink() or not artifact.resolve().is_relative_to(root.resolve()):
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
    root: Path, *, fingerprint_value: str, runner: str, valgrind: str, rustc: str
) -> list[str]:
    path = root / BASELINE_MANIFEST
    if path.is_symlink() or not path.is_file():
        return ["baseline manifest is missing"]
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return [f"baseline manifest is unreadable: {error}"]
    problems: list[str] = []
    expected = {
        "schema_version": BASELINE_MANIFEST_VERSION,
        "kind": "iai-callgrind-baseline",
        "state": "BASELINE_READY",
        "fingerprint": fingerprint_value,
        "runner": runner,
        "valgrind": valgrind.strip(),
        "rustc": rustc.strip(),
        "config_sha256": _sha256(CONFIG),
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


def _summary_comparison_complete(summary: object) -> bool:
    if not isinstance(summary, dict):
        return False
    callgrind = summary.get("callgrind_summary")
    if not isinstance(callgrind, dict):
        return False
    run = callgrind.get("callgrind_run")
    total = run.get("total") if isinstance(run, dict) else None
    metrics = total.get("summary") if isinstance(total, dict) else None
    if not isinstance(metrics, dict) or not metrics:
        return False
    for metric in metrics.values():
        values = metric.get("metrics") if isinstance(metric, dict) else None
        both = values.get("Both") if isinstance(values, dict) else None
        if not isinstance(both, list) or len(both) != 2:
            return False
    return True


def inspect_summaries(root: Path) -> dict[str, object]:
    paths = sorted(root.rglob("summary.json"))
    invalid: list[str] = []
    compared = 0
    for path in paths:
        relative = path.relative_to(root).as_posix()
        if confined_artifact(root, relative) is None:
            invalid.append(relative)
            continue
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            invalid.append(relative)
            continue
        if not isinstance(payload, dict) or not payload.get("callgrind_summary"):
            invalid.append(relative)
            continue
        if _summary_comparison_complete(payload):
            compared += 1
    return {
        "selected_count": len(paths),
        "executed_count": len(paths) - len(invalid),
        "comparison_count": compared,
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
    expected_comparison: bool,
) -> tuple[str, list[str]]:
    inspection = inspect_summaries(root)
    selected = inspection["selected_count"]
    executed = inspection["executed_count"]
    compared = inspection["comparison_count"]
    problems: list[str] = []
    if not isinstance(selected, int) or selected <= 0:
        problems.append("runner produced no summary.json artifacts")
    if executed != selected:
        problems.append("one or more summary artifacts are invalid")
    if expected_comparison and compared != selected:
        problems.append("verified old-vs-new comparison is incomplete")
    if not expected_comparison and compared:
        problems.append("fresh-baseline run unexpectedly claims an old-vs-new comparison")
    status = "NOT_RUN" if problems else ("QUALIFIED" if expected_comparison else "BASELINE_CREATED")
    raw_log = root / "benchmark-output.log"
    safe_raw_log = confined_artifact(root, "benchmark-output.log")
    artifacts = baseline_artifacts(root)
    if safe_raw_log is None or not raw_log.is_file() or raw_log.stat().st_size == 0:
        problems.append("raw benchmark output is missing")
        status = "NOT_RUN"
    comparison = {
        "schema_version": 1,
        "kind": "iai-callgrind-comparison",
        "status": status,
        "fingerprint": fingerprint_value,
        **inspection,
        "summary_artifacts": [
            _artifact(root / path, root)
            for path in inspection["summaries"]
            if confined_artifact(root, path) is not None and (root / path).is_file()
        ],
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
        "config_sha256": _sha256(CONFIG),
        "artifacts": artifacts,
    }
    (root / BASELINE_MANIFEST).write_text(json.dumps(baseline, indent=2) + "\n", encoding="utf-8")
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
    if args.command == "validate-baseline":
        problems = baseline_manifest_problems(
            args.root,
            fingerprint_value=args.fingerprint,
            runner=args.runner,
            valgrind=args.valgrind,
            rustc=args.rustc_file.read_text(encoding="utf-8"),
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
            )
        )
    except ValueError as error:
        print(f"bench-iai: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
