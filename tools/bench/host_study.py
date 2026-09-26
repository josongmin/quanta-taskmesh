#!/usr/bin/env python3
"""Collect every predeclared host attempt, including failed executions.

The append-only event chain is collector accounting, not proof that nobody ran
additional experiments elsewhere. Interrupted/missing attempts cannot qualify.
"""

from __future__ import annotations

import argparse
import os
import re
import stat
import subprocess
import sys
from collections import Counter
from pathlib import Path
from typing import Any

import host_perf
from host_run import write_new

from tools.inspection import read_regular_bytes

if str(host_perf.REPO) not in sys.path:
    sys.path.insert(0, str(host_perf.REPO))
from acquisition_process import run_acquisition as run_process  # noqa: E402

VERSION = 2
ARTIFACTS = {
    "raw": "raw",
    "summary": "summary",
    "provenance": "raw.provenance.json",
    "binary": "raw.runner",
    "topology": "raw.topology.json",
    "resources": "raw.resources.json",
}


def parse_plan(data: bytes) -> dict:
    plan = host_perf.parse_object(data, "attempt plan")
    host_perf.exact_keys(plan, {"schema_version", "contract_sha256", "attempts"}, "attempt plan")
    if type(plan["schema_version"]) is not int or plan["schema_version"] != VERSION:
        raise host_perf.ReceiptError("unsupported attempt plan version")
    digest = plan["contract_sha256"]
    if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
        raise host_perf.ReceiptError("attempt plan requires frozen contract digest")
    if not isinstance(plan["attempts"], list) or not plan["attempts"]:
        raise host_perf.ReceiptError("attempt plan is empty")
    ids = set()
    for attempt in plan["attempts"]:
        if not isinstance(attempt, dict):
            raise host_perf.ReceiptError("attempt must be an object")
        host_perf.exact_keys(
            attempt,
            {
                "id",
                "rate_per_second",
                "pair_index",
                "arm",
                "scenario",
                "scenario_sha256",
                "calibration",
                "calibration_sha256",
                "features",
                "timeout_seconds",
            },
            "attempt",
        )
        name = attempt["id"]
        if (
            not isinstance(name, str)
            or re.fullmatch(r"[a-z0-9][a-z0-9_-]{0,63}", name) is None
            or name in ids
        ):
            raise host_perf.ReceiptError("attempt IDs must be unique safe names")
        ids.add(name)
        if host_perf.nat(attempt["rate_per_second"], "attempt rate") == 0 or attempt["arm"] not in (
            "baseline",
            "candidate",
        ):
            raise host_perf.ReceiptError("invalid attempt rate/arm")
        host_perf.nat(attempt["pair_index"], "attempt pair index")
        if host_perf.nat(attempt["timeout_seconds"], "attempt timeout_seconds") == 0:
            raise host_perf.ReceiptError("attempt timeout_seconds must be positive")
        for key in ("scenario_sha256", "calibration_sha256"):
            if (
                not isinstance(attempt[key], str)
                or re.fullmatch(r"[0-9a-f]{64}", attempt[key]) is None
            ):
                raise host_perf.ReceiptError("invalid attempt input digest")
        for key in ("scenario", "calibration"):
            if not isinstance(attempt[key], str) or not attempt[key]:
                raise host_perf.ReceiptError("attempt input path required")
            path = Path(attempt[key])
            if path.is_absolute() or ".." in path.parts:
                raise host_perf.ReceiptError("attempt input must remain under the plan directory")
        features = attempt["features"]
        if (
            not isinstance(features, list)
            or any(not isinstance(f, str) or not f for f in features)
            or features != sorted(set(features))
        ):
            raise host_perf.ReceiptError("attempt features must be a sorted unique list")
    roles = Counter((a["rate_per_second"], a["pair_index"], a["arm"]) for a in plan["attempts"])
    if any(count != 1 for count in roles.values()) or any(
        (r, i, other) not in roles for r, i, arm in roles for other in ("baseline", "candidate")
    ):
        raise host_perf.ReceiptError("attempt plan requires exactly one attempt per paired arm")
    for rate in {a["rate_per_second"] for a in plan["attempts"]}:
        ordered = [a for a in plan["attempts"] if a["rate_per_second"] == rate]
        for index in range(0, len(ordered), 2):
            first, second = ordered[index : index + 2]
            if (
                first["pair_index"] != index // 2
                or second["pair_index"] != index // 2
                or first["arm"] == second["arm"]
            ):
                raise host_perf.ReceiptError(
                    "planned pair indices/order do not match acquisition sequence"
                )
    return plan


def event(previous: str, sequence: int, kind: str, attempt: str, payload: dict) -> dict:
    return {
        "sequence": sequence,
        "previous_sha256": previous,
        "kind": kind,
        "attempt": attempt,
        "payload": payload,
    }


def append(path: Path, row: dict) -> str:
    body = host_perf.canonical(row) + b"\n"
    descriptor = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_NONBLOCK | os.O_NOFOLLOW)
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            raise OSError(f"regular ledger file required: {path}")
        with os.fdopen(descriptor, "ab") as stream:
            descriptor = -1
            stream.write(body)
            stream.flush()
            os.fsync(stream.fileno())
    finally:
        if descriptor >= 0:
            os.close(descriptor)
    return host_perf.sha256(body)


def collect(plan_bytes: bytes, plan_root: Path, root: Path) -> dict:
    plan = parse_plan(plan_bytes)
    root.mkdir(parents=True, exist_ok=False)
    write_new(root / "plan.json", plan_bytes)
    write_new(root / "events.jsonl", b"")
    previous, sequence = host_perf.sha256(plan_bytes), 0
    interrupted = None
    for attempt in plan["attempts"]:
        name = attempt["id"]
        directory = root / name
        directory.mkdir()
        # The started event precedes even input validation: these failures belong
        # to the planned population as well.
        previous = append(root / "events.jsonl", event(previous, sequence, "started", name, {}))
        sequence += 1
        result, reason, stdout, stderr = None, None, b"", b""
        try:
            if interrupted is not None:
                raise host_perf.ReceiptError(f"not launched after collector signal {interrupted}")
            for role in ("scenario", "calibration"):
                path = (plan_root / attempt[role]).resolve()
                if not path.is_relative_to(plan_root.resolve()):
                    raise host_perf.ReceiptError("attempt input escapes the plan directory")
                if not path.is_file():
                    raise host_perf.ReceiptError("attempt input must be a regular file")
                data = read_regular_bytes(path)
                if host_perf.sha256(data) != attempt[f"{role}_sha256"]:
                    raise host_perf.ReceiptError(f"{role} input differs from predeclared digest")
                write_new(directory / role, data)
            command = [
                sys.executable,
                str(host_perf.REPO / "tools/bench/host_run.py"),
                str((directory / "scenario").resolve()),
                str((directory / "raw").resolve()),
                str((directory / "summary").resolve()),
                str((directory / "calibration").resolve()),
            ]
            for feature in attempt["features"]:
                command.extend(["--feature", feature])
            process = run_process(
                command,
                cwd=host_perf.REPO,
                env=os.environ.copy(),
                timeout_seconds=attempt["timeout_seconds"],
            )
            result = process.returncode
            stdout, stderr = process.stdout.encode(), process.stderr.encode()
            interrupted = process.interrupted_by_signal
            if interrupted is not None:
                reason = f"collector interrupted by signal {interrupted}"
            elif process.timed_out:
                reason = f"runner timeout after {attempt['timeout_seconds']}s"
            elif result is None:
                reason = "runner exited without settled process group"
            elif result != 0:
                reason = f"runner exit {result}"
            elif any(not (directory / file).is_file() for file in ARTIFACTS.values()):
                reason = "runner did not retain all artifacts"
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            reason = str(error)
        write_new(directory / "stdout", stdout)
        write_new(directory / "stderr", stderr)
        files = {
            path.name: host_perf.sha256(read_regular_bytes(path))
            for path in directory.iterdir()
            if path.is_file()
        }
        previous = append(
            root / "events.jsonl",
            event(
                previous,
                sequence,
                "terminal",
                name,
                {
                    "status": "failed" if reason else "completed",
                    "exit_code": result,
                    "reason": reason,
                    "artifact_sha256": files,
                },
            ),
        )
        sequence += 1
    final = {
        "schema_version": VERSION,
        "plan_sha256": host_perf.sha256(plan_bytes),
        "events_sha256": host_perf.sha256(read_regular_bytes(root / "events.jsonl")),
        "last_event_sha256": previous,
        "event_count": sequence,
    }
    write_new(root / "ledger.json", host_perf.canonical(final) + b"\n")
    return verify(root)


def verify(root: Path) -> dict[str, Any]:
    if root.is_symlink() or not root.is_dir():
        raise host_perf.ReceiptError("attempt ledger requires an owned regular directory")
    for name in ("plan.json", "events.jsonl", "ledger.json"):
        path = root / name
        if path.is_symlink() or not path.is_file():
            raise host_perf.ReceiptError("attempt ledger metadata requires regular owned files")
    plan_bytes = read_regular_bytes(root / "plan.json")
    plan = parse_plan(plan_bytes)
    if {path.name for path in root.iterdir()} != {
        "plan.json",
        "events.jsonl",
        "ledger.json",
        *(a["id"] for a in plan["attempts"]),
    }:
        raise host_perf.ReceiptError("unplanned or missing study-root artifacts")
    final = host_perf.parse_object(read_regular_bytes(root / "ledger.json"), "attempt ledger")
    host_perf.exact_keys(
        final,
        {"schema_version", "plan_sha256", "events_sha256", "last_event_sha256", "event_count"},
        "attempt ledger",
    )
    events_bytes = read_regular_bytes(root / "events.jsonl")
    if (
        type(final["schema_version"]) is not int
        or final["schema_version"] != VERSION
        or final["plan_sha256"] != host_perf.sha256(plan_bytes)
        or final["events_sha256"] != host_perf.sha256(events_bytes)
    ):
        raise host_perf.ReceiptError("attempt ledger digest/version differs")
    if not events_bytes.endswith(b"\n"):
        raise host_perf.ReceiptError("truncated attempt event log")
    rows = events_bytes.splitlines(keepends=True)
    if (
        type(final["event_count"]) is not int
        or len(rows) != final["event_count"]
        or len(rows) != 2 * len(plan["attempts"])
    ):
        raise host_perf.ReceiptError("missing or extra planned attempt events")
    previous, terminals = host_perf.sha256(plan_bytes), []
    for index, body in enumerate(rows):
        row = host_perf.parse_object(body, "attempt event")
        host_perf.exact_keys(
            row, {"sequence", "previous_sha256", "kind", "attempt", "payload"}, "attempt event"
        )
        expected = plan["attempts"][index // 2]
        if (
            type(row["sequence"]) is not int
            or row["sequence"] != index
            or row["previous_sha256"] != previous
            or row["attempt"] != expected["id"]
            or row["kind"] != ("started" if index % 2 == 0 else "terminal")
        ):
            raise host_perf.ReceiptError("attempt order, population or hash chain differs")
        previous = host_perf.sha256(body)
        if index % 2 == 0:
            if row["payload"] != {}:
                raise host_perf.ReceiptError("invalid started event")
            continue
        payload = row["payload"]
        if not isinstance(payload, dict):
            raise host_perf.ReceiptError("terminal payload must be an object")
        host_perf.exact_keys(
            payload, {"status", "exit_code", "reason", "artifact_sha256"}, "attempt terminal"
        )
        if payload["status"] not in ("completed", "failed"):
            raise host_perf.ReceiptError("attempt exclusion or unknown terminal is not admitted")
        if payload["exit_code"] is not None and type(payload["exit_code"]) is not int:
            raise host_perf.ReceiptError("attempt exit code must be an integer or unavailable")
        if payload["status"] == "completed" and (
            type(payload["exit_code"]) is not int
            or payload["exit_code"] != 0
            or payload["reason"] is not None
        ):
            raise host_perf.ReceiptError("completed attempt has failure evidence")
        if payload["status"] == "failed" and (
            not isinstance(payload["reason"], str) or not payload["reason"]
        ):
            raise host_perf.ReceiptError("failed attempt requires retained reason")
        files = payload["artifact_sha256"]
        directory = root / expected["id"]
        if not isinstance(files, dict) or directory.is_symlink() or not directory.is_dir():
            raise host_perf.ReceiptError("attempt artifacts unavailable")
        actual = {}
        for path in directory.iterdir():
            if path.is_symlink() or not path.is_file():
                raise host_perf.ReceiptError("attempt artifacts require regular owned files")
            actual[path.name] = host_perf.sha256(read_regular_bytes(path))
        if actual != files or not {"stdout", "stderr"}.issubset(files):
            raise host_perf.ReceiptError("attempt artifact inventory or digest changed")
        if payload["status"] == "completed":
            if not set(ARTIFACTS.values()).issubset(files):
                raise host_perf.ReceiptError("completed attempt artifacts incomplete")
            for role in ("scenario", "calibration"):
                if files.get(role) != expected[f"{role}_sha256"]:
                    raise host_perf.ReceiptError("completed attempt input differs from frozen plan")
        terminals.append({**expected, **payload})
    if previous != final["last_event_sha256"]:
        raise host_perf.ReceiptError("attempt final chain digest differs")
    failures = [row["id"] for row in terminals if row["status"] == "failed"]
    return {
        "schema_version": VERSION,
        "status": "ACCOUNTED" if not failures else "ACCOUNTED_WITH_FAILURES",
        "performance": "UNQUALIFIED",
        "expected": len(plan["attempts"]),
        "attempted": len(terminals),
        "failed": failures,
        "excluded": [],
        "attempts": terminals,
        "plan_sha256": host_perf.sha256(plan_bytes),
        "ledger_sha256": host_perf.sha256(host_perf.canonical(final)),
        "contract_sha256": plan["contract_sha256"],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("collect", "verify"))
    parser.add_argument("directory", type=Path)
    parser.add_argument("--plan", type=Path)
    args = parser.parse_args()
    try:
        if args.mode == "collect":
            if args.plan is None:
                raise host_perf.ReceiptError("collection requires --plan")
            report = collect(read_regular_bytes(args.plan), args.plan.parent, args.directory)
        else:
            report = verify(args.directory)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"attempt ledger rejected: {error}", file=sys.stderr)
        return 1
    print(host_perf.canonical(report).decode())
    return 1 if report["failed"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
