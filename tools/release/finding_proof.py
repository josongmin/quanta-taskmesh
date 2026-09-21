#!/usr/bin/env python3
"""Run one non-vacuous, source-bound negative/regression witness per SEP-21 finding.

This producer records execution, not human approval of witness adequacy. The
ordinary hosted receipt remains authoritative for the full required gate set.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SPEC = REPO / "tools/release/finding-proof-spec.json"
PLAN = REPO / "docs/bugbash/sep-21/tickets/plan.json"
IDS = [f"TM21-{number:03d}" for number in range(1, 24)]
RUST_TEST = re.compile(r"(?m)^\s*(?:async\s+)?fn\s+([A-Za-z_][A-Za-z_0-9]*)\s*\(")
PYTEST = re.compile(r"(?m)^def\s+(test_[A-Za-z_0-9]+)\s*\(")

if str(REPO) not in sys.path:
    sys.path.insert(0, str(REPO))

from tools.qualification.receipt import source_identity  # noqa: E402
from tools.release.semver import sha256  # noqa: E402


def expected_tickets(plan: object) -> dict[str, str]:
    if not isinstance(plan, dict) or not isinstance(plan.get("tickets"), list):
        raise ValueError("finding ticket plan missing")
    mapping: dict[str, str] = {}
    for ticket in plan["tickets"]:
        if not isinstance(ticket, dict) or not isinstance(ticket.get("findings"), list):
            raise ValueError("finding ticket plan malformed")
        for finding in ticket["findings"]:
            if not isinstance(finding, str) or finding in mapping:
                raise ValueError("duplicate or malformed finding in ticket plan")
            mapping[finding] = ticket.get("id")
    if sorted(mapping) != IDS:
        raise ValueError("ticket plan does not map exactly 23 findings")
    return mapping


def test_command(witness: dict) -> list[str]:
    path = witness["path"]
    name = witness["test"]
    if witness["kind"] == "rust":
        parts = Path(path).parts
        if parts[2] == "src":
            return [
                "cargo",
                "test",
                "--locked",
                "-p",
                parts[1],
                "--lib",
                witness["selector"],
                "--",
                "--exact",
                "--nocapture",
            ]
        return [
            "cargo",
            "test",
            "--locked",
            "-p",
            parts[1],
            "--test",
            Path(path).stem,
            name,
            "--",
            "--exact",
            "--nocapture",
        ]
    return ["uv", "run", "pytest", "-q", f"{path}::{name}"]


def spec_problems(root: Path, spec: object, plan: object, ordinary_gates: set[str]) -> list[str]:
    if not isinstance(spec, dict) or spec.get("schema_version") != 1:
        return ["finding proof spec missing or malformed"]
    try:
        mapping = expected_tickets(plan)
    except ValueError as exc:
        return [str(exc)]
    witnesses = spec.get("witnesses")
    if not isinstance(witnesses, list) or len(witnesses) != 23:
        return ["finding proof spec must contain exactly 23 witnesses"]
    reasons: list[str] = []
    ids = [row.get("id") for row in witnesses if isinstance(row, dict)]
    if ids != IDS or len(ids) != 23:
        reasons.append("finding proof spec IDs must be ordered and exactly TM21-001..023")
    for row in witnesses:
        if not isinstance(row, dict):
            reasons.append("finding proof spec row malformed")
            continue
        finding, kind, name = row.get("id"), row.get("kind"), row.get("test")
        if not isinstance(finding, str):
            reasons.append("finding proof spec ID malformed")
            continue
        path_name = row.get("path")
        if row.get("ticket") != mapping.get(finding):
            reasons.append(f"finding {finding} ticket mapping differs from plan")
        if row.get("required_gate") not in ordinary_gates:
            reasons.append(f"finding {finding} required ordinary gate is unknown")
        if not isinstance(row.get("negative_claim"), str) or len(row["negative_claim"]) < 24:
            reasons.append(f"finding {finding} negative claim is absent")
        if (
            kind not in ("rust", "pytest")
            or not isinstance(name, str)
            or not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*", name)
        ):
            reasons.append(f"finding {finding} test kind/name malformed")
            continue
        if not isinstance(path_name, str) or Path(path_name).is_absolute() or "\\" in path_name:
            reasons.append(f"finding {finding} test path malformed")
            continue
        path = Path(path_name)
        if not path.parts or any(part in ("", ".", "..") for part in path.parts):
            reasons.append(f"finding {finding} test path unsafe")
            continue
        if kind == "rust":
            integration = (
                len(path.parts) == 4
                and path.parts[0] == "crates"
                and path.parts[2] == "tests"
                and path.suffix == ".rs"
            )
            library = (
                len(path.parts) >= 4
                and path.parts[0] == "crates"
                and path.parts[2] == "src"
                and path.suffix == ".rs"
            )
            valid_layout = integration or library
            selector = row.get("selector")
            if library:
                module = "::".join((*path.parts[3:-1], path.stem))
                expected_selector = f"{module}::{name}"
                if (root / path).is_file() and "mod tests" in (root / path).read_text(
                    encoding="utf-8"
                ):
                    expected_selector = f"{module}::tests::{name}"
                if selector != expected_selector:
                    reasons.append(f"finding {finding} library test selector mismatch")
            elif selector is not None:
                reasons.append(f"finding {finding} integration test selector must be omitted")
        else:
            if row.get("selector") is not None:
                reasons.append(f"finding {finding} pytest selector must be omitted")
            valid_layout = (
                path.parts[0] == "tools" and "tests" in path.parts and path.suffix == ".py"
            )
        disk = root / path
        if (
            not valid_layout
            or not disk.is_file()
            or disk.is_symlink()
            or not disk.resolve().is_relative_to(root.resolve())
        ):
            reasons.append(f"finding {finding} test source missing or unsafe")
            continue
        pattern = RUST_TEST if kind == "rust" else PYTEST
        if name not in pattern.findall(disk.read_text(encoding="utf-8")):
            reasons.append(f"finding {finding} exact test does not exist in source")
    return reasons


def raw_identity(root: Path, path: Path) -> dict:
    return {
        "path": str(path.relative_to(root)),
        "sha256": sha256(path),
        "size": path.stat().st_size,
    }


def nonvacuous_output(row: dict, stdout: bytes, stderr: bytes) -> bool:
    combined = (stdout + stderr).decode("utf-8", errors="replace")
    if row["kind"] == "rust":
        selected = row.get("selector", row["test"])
        return bool(
            re.search(r"(?m)^running 1 test\s*$", combined)
            and re.search(rf"(?m)^test {re.escape(selected)} \.\.\. ok\s*$", combined)
            and re.search(r"(?m)^test result: ok\. 1 passed; 0 failed; 0 ignored;", combined)
        )
    return bool(re.search(r"(?m)^1 passed(?:, [0-9]+ warnings?)? in ", combined))


def run_one(root: Path, output: Path, row: dict) -> dict:
    argv = test_command(row)
    try:
        process = subprocess.run(argv, cwd=root, capture_output=True, check=False, timeout=900)
        code, stdout, stderr, timed_out = process.returncode, process.stdout, process.stderr, False
    except subprocess.TimeoutExpired as exc:
        code, stdout, stderr, timed_out = None, exc.stdout or b"", exc.stderr or b"", True
    except OSError as exc:
        code, stdout, stderr, timed_out = None, b"", str(exc).encode(), False
    prefix = output / row["id"]
    out_path, err_path = prefix.with_suffix(".stdout.log"), prefix.with_suffix(".stderr.log")
    out_path.write_bytes(stdout)
    err_path.write_bytes(stderr)
    nonvacuous = nonvacuous_output(row, stdout, stderr)
    source_path = root / row["path"]
    return {
        **row,
        "command": argv,
        "test_source": raw_identity(root, source_path),
        "exit_code": code,
        "timed_out": timed_out,
        "selected": 1,
        "executed": 1 if nonvacuous else 0,
        "status": "PASS" if code == 0 and not timed_out and nonvacuous else "FAIL",
        "stdout": raw_identity(root, out_path),
        "stderr": raw_identity(root, err_path),
    }


def produce(out: Path) -> int:
    spec = json.loads(SPEC.read_text(encoding="utf-8"))
    plan = json.loads(PLAN.read_text(encoding="utf-8"))
    ordinary = json.loads((REPO / "tools/gates/required.json").read_text(encoding="utf-8"))
    reasons = spec_problems(REPO, spec, plan, set(ordinary["required"]))
    if reasons:
        raise ValueError("; ".join(reasons))
    before = source_identity()
    if before["dirty"]:
        raise ValueError("finding proof requires a clean candidate source")
    if out.is_symlink() or out.exists() and not out.is_dir():
        raise ValueError("finding proof output must be a directory")
    out = out.resolve()
    if not out.is_relative_to((REPO / "target").resolve()):
        raise ValueError("finding proof output must be under target/")
    out.mkdir(parents=True, exist_ok=True)
    rows = [run_one(REPO, out, row) for row in spec["witnesses"]]
    after = source_identity()
    stable = before["paths_digest"] == after["paths_digest"] and after["dirty"] is False
    manifest = {
        "schema_version": 1,
        "producer": "taskmesh-finding-proof-v1",
        "status": "PASS" if stable and all(row["status"] == "PASS" for row in rows) else "FAIL",
        "source": before,
        "source_after": {"paths_digest": after["paths_digest"], "dirty": after["dirty"]},
        "spec": raw_identity(REPO, SPEC),
        "plan": raw_identity(REPO, PLAN),
        "results": rows,
    }
    dest = out / "finding-proof-manifest.json"
    dest.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"finding proof: {dest} {manifest['status']}")
    return 0 if manifest["status"] == "PASS" else 1


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=REPO / "target/release/finding-proof")
    args = parser.parse_args(argv)
    try:
        return produce(args.out)
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as exc:
        print(f"finding proof NOT_RUN: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
