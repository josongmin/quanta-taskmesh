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
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CONFIG = REPO / "tools" / "bench" / "perf-gate.json"
RUNNER = "iai-callgrind-runner"

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
