#!/usr/bin/env python3
"""Fail-closed release decision over an exact-source ordinary receipt.

The ordinary CI verdict is not weakened here. A generated mutation report must
retain every planned identity. Compile-unviable mutations are explicit,
unscored limitations; missed, timeout, and equivalent outcomes cannot be
relabeled as quality PASS.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
POLICY = REPO / "tools/release/release-policy.json"
REQUIRED = REPO / "tools/release/release-required.json"
ADJUDICATION = REPO / "tools/release/adjudication.json"
PLAN = REPO / "docs/bugbash/sep-21/tickets/plan.json"
ORDINARY_REQUIRED = REPO / "tools/gates/required.json"
METRICS = ("lines", "regions", "functions", "instantiations", "branches", "mcdc")
COVERAGE_COMMAND = (
    "cargo llvm-cov --workspace --exclude taskmesh-bench "
    "--exclude taskmesh-doc-examples --all-targets --json --summary-only"
)
ADJUDICATION_IDS = {
    "C01-provenance-rust",
    "C01-provenance-wire",
    "C01-child-builder",
    "C01-child-wire",
    "E01-capability-handle",
    "E03-memory-return",
    "E04-nested-cycle",
    "H01-stack-preflight",
    "H02-acquisition-precedence",
    "H03-physical-domains",
    "H03-executor-contract",
    "H03-rayon-constructor",
    "facade-reexports",
}
LOCAL_RELEASE_CHECKS = {
    "not_github_actions",
    "workspace_matches",
    "source_clean",
    "action_is_local",
    "action_source_matches",
}

if str(REPO) not in sys.path:
    sys.path.insert(0, str(REPO))

from tools.bench import iai_gate  # noqa: E402
from tools.qualification import receipt as ordinary  # noqa: E402
from tools.qualification.evidence import runtime_action  # noqa: E402
from tools.release import finding_proof  # noqa: E402
from tools.release.semver import command, policy_problems, sha256, workspace_version  # noqa: E402


def read_json(path: Path) -> object | None:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError):
        return None


def safe_artifact(root: Path, name: object) -> Path | None:
    if not isinstance(name, str) or not name or "\\" in name:
        return None
    rel = Path(name)
    if rel.is_absolute() or any(part in ("", ".", "..") for part in rel.parts):
        return None
    path = root / rel
    if not path.is_file() or path.is_symlink() or not path.resolve().is_relative_to(root.resolve()):
        return None
    if any(
        part.is_symlink() for part in path.parents if part != root and part.is_relative_to(root)
    ):
        return None
    return path


def identity(root: Path, name: object) -> dict | None:
    path = safe_artifact(root, name)
    if path is None:
        return None
    return {
        "path": str(path.relative_to(root)),
        "sha256": sha256(path),
        "size": path.stat().st_size,
    }


def check_identity(root: Path, value: object, label: str, reasons: list[str]) -> object | None:
    if not isinstance(value, dict):
        reasons.append(f"{label} artifact identity missing")
        return None
    actual = identity(root, value.get("path"))
    if actual is None or actual != value:
        reasons.append(f"{label} artifact missing, unsafe, or digest/size mismatch")
        return None
    return read_json(root / actual["path"])


def iai_raw_problems(
    root: Path,
    baseline: object,
    comparison: object,
    output_ref: object,
    gate_status_line: object,
) -> list[str]:
    """Re-derive the Linux IAI verdict from the saved raw producer artifacts."""
    problems: list[str] = []
    if not isinstance(baseline, dict):
        problems.append("IAI baseline manifest is missing or malformed")
        return problems
    if not isinstance(comparison, dict):
        problems.append("IAI comparison manifest is missing or malformed")
        return problems
    if comparison.get("status") != "QUALIFIED":
        problems.append("IAI comparison status is not QUALIFIED")
    if (
        comparison.get("schema_version") != 1
        or comparison.get("kind") != "iai-callgrind-comparison"
    ):
        problems.append("IAI comparison manifest schema/kind mismatch")
    if comparison.get("problems") != []:
        problems.append("IAI comparison manifest reports problems")

    config_path = safe_artifact(root, "tools/bench/perf-gate.json")
    config = read_json(config_path) if config_path is not None else None
    gate = config.get("instruction_count_gate") if isinstance(config, dict) else None
    if not isinstance(gate, dict):
        problems.append("IAI gate config is missing or malformed")
        return problems
    expected_cases = gate.get("expected_cases")
    if (
        not isinstance(expected_cases, list)
        or not expected_cases
        or any(not isinstance(case, str) or not case for case in expected_cases)
        or len(expected_cases) != len(set(expected_cases))
    ):
        problems.append("IAI expected case inventory is malformed")
        return problems
    inputs = gate.get("fingerprint_inputs")
    if (
        not isinstance(inputs, list)
        or not inputs
        or any(not isinstance(path, str) for path in inputs)
    ):
        problems.append("IAI fingerprint input inventory is malformed")
        return problems
    input_paths = [safe_artifact(root, name) for name in inputs]
    if any(path is None for path in input_paths):
        problems.append("IAI fingerprint source input is missing or unsafe")
        return problems
    runner, valgrind, rustc = (baseline.get(key) for key in ("runner", "valgrind", "rustc"))
    if (
        not isinstance(runner, str)
        or runner != gate.get("iai_callgrind_runner")
        or not isinstance(valgrind, str)
        or not valgrind.strip()
        or not isinstance(rustc, str)
        or not rustc.strip()
    ):
        problems.append("IAI baseline toolchain identity is missing or incompatible")
        return problems
    fingerprint = iai_gate.fingerprint(
        input_paths, gate.get("measurement_schema"), runner, valgrind, rustc
    )
    if baseline.get("fingerprint") != fingerprint or comparison.get("fingerprint") != fingerprint:
        problems.append("IAI baseline/comparison fingerprint differs from current source")
    status_tokens = gate_status_line.split() if isinstance(gate_status_line, str) else []
    expected_tokens = (
        "status=QUALIFIED",
        f"fingerprint={fingerprint}",
        f"regression={gate.get('regression')}",
        f"schema={gate.get('measurement_schema')}",
    )
    if (
        not isinstance(gate_status_line, str)
        or not gate_status_line.startswith("taskmesh-iai-gate ")
        or any(status_tokens.count(token) != 1 for token in expected_tokens)
        or len(status_tokens) != len(expected_tokens) + 1
    ):
        problems.append("IAI raw verdict differs from ordinary bench-iai status line")

    store = root / "target/iai"
    if (
        store.is_symlink()
        or not store.is_dir()
        or not store.resolve().is_relative_to(root.resolve())
    ):
        problems.append("IAI raw store is missing or unsafe")
        return problems
    problems.extend(
        f"IAI {problem}"
        for problem in iai_gate.baseline_manifest_problems(
            store,
            fingerprint_value=fingerprint,
            runner=runner,
            valgrind=valgrind,
            rustc=rustc,
            config_path=config_path,
        )
    )
    if baseline.get("artifacts") != iai_gate.baseline_artifacts(store):
        problems.append("IAI baseline raw artifact inventory differs from disk")
    inspection = iai_gate.inspect_summaries(store)
    if (
        inspection["selected_count"] != len(expected_cases)
        or inspection["executed_count"] != len(expected_cases)
        or inspection["comparison_count"] != len(expected_cases)
        or sorted(inspection["cases"]) != sorted(expected_cases)
        or len(inspection["raw_outputs"]) != len(set(inspection["raw_outputs"]))
        or inspection["invalid_summaries"]
    ):
        problems.append("IAI summary case/comparison/raw-output inventory is incomplete")
    for key, expected in inspection.items():
        if comparison.get(key) != expected:
            problems.append(f"IAI comparison {key} differs from raw summaries")
    summaries = [identity(store, path) for path in inspection["summaries"]]
    if None in summaries or comparison.get("summary_artifacts") != summaries:
        problems.append("IAI comparison summary artifact identities differ from disk")
    output = identity(store, "benchmark-output.log")
    release_output = identity(root, "target/iai/benchmark-output.log")
    if (
        output is None
        or output["size"] == 0
        or comparison.get("raw_output") != output
        or output_ref != release_output
    ):
        problems.append("IAI benchmark output identity differs from raw log")
    return problems


def metric_report(raw: object, reasons: list[str]) -> dict:
    try:
        assert isinstance(raw, dict)
        totals = raw["data"][0]["totals"]
        assert isinstance(totals, dict)
    except (AssertionError, KeyError, IndexError, TypeError):
        reasons.append("coverage summary has no data[0].totals")
        return {}
    report = {}
    for name in METRICS:
        value = totals.get(name)
        if not isinstance(value, dict) or type(value.get("count")) is not int:
            reasons.append(f"coverage {name} count missing")
            continue
        count = value["count"]
        if count < 0:
            reasons.append(f"coverage {name} count negative")
            continue
        percent = value.get("percent")
        if count and (type(percent) not in (float, int) or not 0 <= percent <= 100):
            reasons.append(f"coverage {name} percent missing or invalid")
            continue
        report[name] = {
            "status": "COLLECTED" if count else "NOT_COLLECTED",
            "count": count,
            "percent": percent if count else None,
        }
    return report


def closure_report(root: Path, plan: object, reasons: list[str]) -> list[dict]:
    if not isinstance(plan, dict) or not isinstance(plan.get("tickets"), list):
        reasons.append("ticket plan missing")
        return []
    all_findings: list[str] = []
    closure = []
    for ticket in plan["tickets"]:
        if not isinstance(ticket, dict):
            reasons.append("ticket plan has malformed ticket")
            continue
        ticket_id, name, findings = ticket.get("id"), ticket.get("file"), ticket.get("findings")
        if (
            not isinstance(ticket_id, str)
            or not isinstance(name, str)
            or not isinstance(findings, list)
            or not all(isinstance(finding, str) for finding in findings)
        ):
            reasons.append("ticket plan has malformed identity/findings")
            continue
        all_findings.extend(findings)
        path = f"docs/bugbash/sep-21/tickets/{name}"
        file_id = identity(root, path)
        text = (root / path).read_text(encoding="utf-8") if file_id else ""
        match = re.search(r"^- 상태: ([A-Z_]+)$", text, re.MULTILINE)
        status = match.group(1) if match else "MISSING"
        evidence = any(
            heading in text
            for heading in ("## Closure evidence", "## Phase B evidence", "## Producer evidence")
        )
        if ticket_id != "SEP21-R01" and status not in ("LOCALLY_VERIFIED", "HOSTED_QUALIFIED"):
            reasons.append(f"upstream ticket {ticket_id} is {status}, not verified")
        if ticket_id != "SEP21-R01" and not evidence:
            reasons.append(f"upstream ticket {ticket_id} has no closure/producer evidence section")
        closure.append(
            {
                "ticket": ticket_id,
                "findings": findings,
                "status": status,
                "ticket_artifact": file_id,
                "evidence_section": evidence,
            }
        )
    if sorted(all_findings) != [f"TM21-{number:03d}" for number in range(1, 24)]:
        reasons.append("ticket plan does not map exactly all 23 findings once")
    if len(closure) != 12:
        reasons.append("ticket plan does not contain exactly 12 tickets")
    return closure


def finding_proof_problems(
    root: Path,
    manifest: object,
    spec: object,
    plan: object,
    ordinary_receipt: object,
    source: dict,
) -> list[str]:
    if not isinstance(manifest, dict):
        return ["finding proof manifest missing"]
    reasons: list[str] = []
    gate_object = ordinary_receipt.get("gates") if isinstance(ordinary_receipt, dict) else None
    ordinary_gates = gate_object.get("results", []) if isinstance(gate_object, dict) else []
    if not isinstance(ordinary_gates, list):
        ordinary_gates = []
    statuses = {
        row.get("id"): row.get("status")
        for row in ordinary_gates
        if isinstance(row, dict) and isinstance(row.get("id"), str)
    }
    required = read_json(root / "tools/gates/required.json")
    required_ids = set(required.get("required", [])) if isinstance(required, dict) else set()
    reasons.extend(finding_proof.spec_problems(root, spec, plan, required_ids))
    if reasons:
        return reasons
    if (
        manifest.get("schema_version") != 1
        or manifest.get("producer") != "taskmesh-finding-proof-v1"
    ):
        reasons.append("finding proof producer/schema mismatch")
    if manifest.get("status") != "PASS":
        reasons.append("finding proof producer did not pass")
    recorded_source = manifest.get("source")
    if not isinstance(recorded_source, dict) or any(
        recorded_source.get(key) != source.get(key)
        for key in ("head", "tree", "paths_digest", "dirty")
    ):
        reasons.append("finding proof source differs from release source")
    if manifest.get("source_after") != {
        field: source.get(field) for field in ("head", "tree", "paths_digest", "dirty")
    }:
        reasons.append("finding proof source changed during execution")
    for field, path in (
        ("spec", "tools/release/finding-proof-spec.json"),
        ("plan", "docs/bugbash/sep-21/tickets/plan.json"),
    ):
        if manifest.get(field) != identity(root, path):
            reasons.append(f"finding proof {field} digest differs from tracked input")
    rows = manifest.get("results")
    witnesses = spec["witnesses"]
    if (
        not isinstance(rows, list)
        or len(rows) != 23
        or [row.get("id") for row in rows if isinstance(row, dict)] != finding_proof.IDS
    ):
        return [*reasons, "finding proof results must be exactly 23 ordered IDs"]
    for expected, row in zip(witnesses, rows):
        finding = expected["id"]
        if not isinstance(row, dict):
            reasons.append(f"finding {finding} result malformed")
            continue
        if any(row.get(key) != expected.get(key) for key in expected):
            reasons.append(f"finding {finding} result differs from tracked witness spec")
        if row.get("command") != finding_proof.test_command(expected):
            reasons.append(f"finding {finding} test command differs from spec")
        if row.get("test_source") != identity(root, expected["path"]):
            reasons.append(f"finding {finding} test source digest mismatch")
        if statuses.get(expected["required_gate"]) != "PASS":
            reasons.append(f"finding {finding} required ordinary gate did not pass")
        if (
            row.get("status") != "PASS"
            or type(row.get("exit_code")) is not int
            or row["exit_code"] != 0
            or row.get("timed_out") is not False
            or type(row.get("selected")) is not int
            or row["selected"] != 1
            or type(row.get("executed")) is not int
            or row["executed"] != 1
        ):
            reasons.append(f"finding {finding} test did not execute exactly once and pass")
        outputs = []
        for stream in ("stdout", "stderr"):
            expected_path = f"target/release/finding-proof/{finding}.{stream}.log"
            ref = row.get(stream)
            if (
                not isinstance(ref, dict)
                or ref.get("path") != expected_path
                or ref != identity(root, expected_path)
            ):
                reasons.append(f"finding {finding} {stream} raw digest/path mismatch")
                outputs.append(b"")
            else:
                outputs.append((root / expected_path).read_bytes())
        if not finding_proof.nonvacuous_output(expected, *outputs):
            reasons.append(f"finding {finding} raw output has zero or wrong test count")
    return reasons


def adjudication_problems(
    root: Path,
    value: object,
    source_sha: object,
    baseline: object,
    semver_manifest: object = None,
    template: object = None,
    actor: object = None,
) -> list[str]:
    if not isinstance(value, dict):
        return ["human adjudication manifest missing"]
    reasons = []
    if value.get("schema_version") != 1 or value.get("baseline_sha") != baseline:
        reasons.append("human adjudication schema/baseline mismatch")
    if value.get("source_sha") != source_sha:
        reasons.append("human adjudication is not bound to candidate SHA")
    if value.get("decision") != "APPROVED" or not value.get("reviewer"):
        reasons.append("human adjudication approval/reviewer missing")
    if actor is not None and value.get("reviewer") != actor:
        reasons.append("human adjudication reviewer differs from hosted release actor")
    if value.get("all_tool_findings_reviewed") is not True:
        reasons.append("human reviewer did not attest complete semver finding review")
    items = value.get("items")
    item_ids = (
        [i.get("id") for i in items if isinstance(i, dict)] if isinstance(items, list) else []
    )
    if (
        not isinstance(items, list)
        or not all(isinstance(item_id, str) for item_id in item_ids)
        or set(item_ids) != ADJUDICATION_IDS
        or len(items) != len(ADJUDICATION_IDS)
    ):
        reasons.append("human adjudication item set differs from required blind spots")
        return reasons
    if template is not None:
        template_items = template.get("items") if isinstance(template, dict) else None
        if not isinstance(template_items, list) or [
            (item.get("id"), item.get("surface"), item.get("evidence")) for item in items
        ] != [
            (item.get("id"), item.get("surface"), item.get("evidence"))
            for item in template_items
            if isinstance(item, dict)
        ]:
            reasons.append("human adjudication differs from tracked blind-spot template")
    changelog = (root / "CHANGELOG.md").read_text(encoding="utf-8")
    for item in items:
        if not isinstance(item, dict):
            reasons.append("malformed human adjudication item")
            continue
        name = item["id"]
        if item.get("surface") not in ("rust-api", "serde-wire", "behavior", "facade"):
            reasons.append(f"adjudication {name} has invalid surface")
        if item.get("decision") not in ("APPROVED_BREAK", "NO_BREAK"):
            reasons.append(f"adjudication {name} decision pending")
        if identity(root, item.get("evidence")) is None:
            reasons.append(f"adjudication {name} evidence path missing")
        anchor = item.get("changelog_anchor")
        if item.get("decision") == "APPROVED_BREAK" and (
            not isinstance(anchor, str) or anchor not in changelog
        ):
            reasons.append(f"adjudication {name} lacks a concrete CHANGELOG anchor")
    reviews = value.get("semver_outputs")
    results = semver_manifest.get("results") if isinstance(semver_manifest, dict) else None
    if not isinstance(reviews, list) or not isinstance(results, list) or len(reviews) != 4:
        reasons.append("human semver raw review is incomplete")
        return reasons
    if [r.get("crate") for r in reviews if isinstance(r, dict)] != [
        r.get("crate") for r in results if isinstance(r, dict)
    ]:
        reasons.append("human semver review crate set/order differs from tool run")
        return reasons
    for review, result in zip(reviews, results):
        if not isinstance(review, dict) or not isinstance(result, dict):
            reasons.append("malformed human semver crate review")
            continue
        crate = result.get("crate")
        stdout_ref, stderr_ref = result.get("stdout"), result.get("stderr")
        if not isinstance(stdout_ref, dict) or not isinstance(stderr_ref, dict):
            reasons.append(f"semver {crate} lacks raw identities for human review")
            continue
        if (
            review.get("stdout_sha256") != stdout_ref.get("sha256")
            or review.get("stderr_sha256") != stderr_ref.get("sha256")
            or review.get("all_findings_reviewed") is not True
        ):
            reasons.append(f"human semver {crate} review is not raw-bound/complete")
        findings = review.get("findings")
        if not isinstance(findings, list):
            reasons.append(f"human semver {crate} findings are missing")
            continue
        if result.get("status") == "FINDINGS" and not findings:
            reasons.append(f"human semver {crate} has unmapped deny-level findings")
        if result.get("status") == "CLEAN" and findings:
            reasons.append(f"human semver {crate} lists findings despite clean tool result")
        raw_paths = (stdout_ref.get("path"), stderr_ref.get("path"))
        raw = b"".join(
            path.read_bytes()
            for name in raw_paths
            if (path := safe_artifact(root, name)) is not None
        ).decode("utf-8", errors="replace")
        seen: set[tuple[str, int]] = set()
        for finding in findings:
            if not isinstance(finding, dict):
                reasons.append(f"human semver {crate} finding is malformed")
                continue
            locator = finding.get("raw_locator")
            occurrence = finding.get("occurrence")
            item_id = finding.get("adjudication_item_id")
            if (
                not isinstance(locator, str)
                or len(locator) < 20
                or type(occurrence) is not int
                or occurrence < 1
                or raw.count(locator) < occurrence
            ):
                reasons.append(f"human semver {crate} finding locator absent from raw output")
            elif (locator, occurrence) in seen:
                reasons.append(f"human semver {crate} finding locator duplicated")
            else:
                seen.add((locator, occurrence))
            item = next((i for i in items if i.get("id") == item_id), None)
            if item is None or item.get("decision") != "APPROVED_BREAK":
                reasons.append(f"human semver {crate} finding has no approved break mapping")
    return reasons


def semver_problems(root: Path, manifest: object, source: dict, policy: dict) -> list[str]:
    if not isinstance(manifest, dict):
        return ["semver manifest missing"]
    reasons = []
    expected = {
        "schema_version": 1,
        "producer": "taskmesh-semver-release-v1",
        "baseline_sha": policy.get("baseline_sha"),
        "baseline_version": policy.get("baseline_version"),
        "policy_sha256": sha256(root / "tools/release/release-policy.json"),
        "tool": policy.get("semver_tool"),
        "features": policy.get("features"),
        "release_type": policy.get("release_type"),
        "candidate_version": policy.get("candidate_version"),
    }
    for key, value in expected.items():
        if manifest.get(key) != value:
            reasons.append(f"semver {key} mismatch")
    if manifest.get("status") != "REPORTED":
        reasons.append("semver audit did not complete as REPORTED")
    ms = manifest.get("source")
    if not isinstance(ms, dict) or any(
        ms.get(key) != source.get(key) for key in ("head", "tree", "paths_digest", "dirty")
    ):
        reasons.append("semver source is not the release source")
    if manifest.get("source_after") != {
        field: source.get(field) for field in ("head", "tree", "paths_digest", "dirty")
    }:
        reasons.append("semver audit changed source or lacks stable-source proof")
    results = manifest.get("results")
    if not isinstance(results, list) or len(results) != 4:
        return [*reasons, "semver does not contain exactly four crate results"]
    if [r.get("crate") for r in results if isinstance(r, dict)] != policy.get("public_crates"):
        reasons.append("semver crate set/order mismatch")
    for result in results:
        if not isinstance(result, dict):
            reasons.append("malformed semver crate result")
            continue
        crate = result.get("crate")
        if crate not in policy.get("public_crates", []):
            continue
        if result.get("command") != command(crate, policy["baseline_sha"]):
            reasons.append(f"semver {crate} command mismatch")
        if result.get("timed_out") is not False or (
            result.get("exit_code"),
            result.get("status"),
        ) not in ((0, "CLEAN"), (100, "FINDINGS")):
            reasons.append(f"semver {crate} did not finish cleanly or with deny-level findings")
        for stream in ("stdout", "stderr"):
            ref = result.get(stream)
            if not isinstance(ref, dict) or identity(root, ref.get("path")) != ref:
                reasons.append(f"semver {crate} {stream} raw artifact missing or mismatched")
    return reasons


def release_attestation(source: dict, local_qualified: bool = False) -> dict:
    try:
        action = runtime_action(
            root=REPO,
            source_head=source.get("head", ""),
            local_workflow=".github/workflows/release.yml",
            local_job="release",
        )
    except (OSError, RuntimeError, ValueError) as exc:
        action = {"error": str(exc)}
    hosted_checks = {
        "github_actions": os.environ.get("GITHUB_ACTIONS") == "true",
        "workflow_dispatch": os.environ.get("GITHUB_EVENT_NAME") == "workflow_dispatch",
        "main_ref": os.environ.get("GITHUB_REF") == "refs/heads/main",
        "head_matches": os.environ.get("GITHUB_SHA") == source.get("head"),
        "workspace_matches": bool(os.environ.get("GITHUB_WORKSPACE"))
        and Path(os.environ["GITHUB_WORKSPACE"]).resolve() == REPO.resolve(),
        "source_clean": source.get("dirty") is False,
        "workflow_matches": action.get("workflow") == ".github/workflows/release.yml",
        "job_matches": action.get("job") == "release",
        "action_source_matches": action.get("source_sha") == source.get("head"),
        "actor_present": bool(os.environ.get("GITHUB_ACTOR")),
    }
    if local_qualified:
        checks = {
            "not_github_actions": os.environ.get("GITHUB_ACTIONS") != "true",
            "workspace_matches": Path.cwd().resolve() == REPO.resolve(),
            "source_clean": source.get("dirty") is False,
            "action_is_local": action.get("context") == "local" and action.get("event") == "local",
            "action_source_matches": action.get("source_sha") == source.get("head"),
        }
        return {
            "kind": "local-release",
            "actor": None,
            "action": action,
            "checks": checks,
            "eligible": all(checks.values()),
        }
    return {
        "kind": "github-actions-release" if all(hosted_checks.values()) else "local-or-invalid",
        "actor": os.environ.get("GITHUB_ACTOR"),
        "action": action,
        "checks": hosted_checks,
        "eligible": all(hosted_checks.values()),
    }


def evaluate(value: object, *, root: Path = REPO, current: dict | None = None) -> dict:
    if not isinstance(value, dict):
        return {"status": "NOT_QUALIFIED", "reasons": ["release receipt is not an object"]}
    reasons: list[str] = []
    source = value.get("source")
    if not isinstance(source, dict):
        source = {}
        reasons.append("release source identity missing")
    if current is not None:
        for key in ("head", "tree", "paths_digest", "dirty"):
            if source.get(key) != current.get(key):
                reasons.append(f"release source {key} differs from current tree")
    if source.get("dirty") is not False:
        reasons.append("release source is dirty")
    if value.get("schema_version") != 1:
        reasons.append("release receipt schema_version != 1")
    policy = check_identity(root, value.get("policy"), "release policy", reasons)
    required = check_identity(root, value.get("required"), "release required", reasons)
    if not isinstance(policy, dict):
        policy = {}
    reasons.extend(policy_problems(policy))
    if not isinstance(required, dict) or required.get("schema_version") != 1:
        reasons.append("release required profile missing or malformed")
        required = {}
    ordinary_required = read_json(root / "tools/gates/required.json")
    if not isinstance(ordinary_required, dict) or required.get(
        "ordinary_gate_ids"
    ) != ordinary_required.get("required"):
        reasons.append("release ordinary set differs from ordinary required.json")
    if required.get("release_only") != [
        "semver-release",
        "human-api-wire-behavior-adjudication",
        "finding-closure",
        "finding-proof",
        "coverage-raw",
        "iai-raw",
        "release-receipt",
    ]:
        reasons.append("release-only required set is incomplete")
    if (
        policy.get("generated_policy")
        != "full-workspace viable mutants all caught; unviable reported and excluded; "
        "missed/timeout/equivalent fail"
    ):
        reasons.append("generated mutation quality policy was relaxed")
    try:
        current_version = workspace_version((root / "Cargo.toml").read_text(encoding="utf-8"))
    except OSError:
        current_version = None
    version = policy.get("candidate_version")
    if (
        not isinstance(version, str)
        or re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version) is None
        or version != current_version
        or version == policy.get("baseline_version")
    ):
        reasons.append("candidate version is undecided, unchanged, or differs from Cargo.toml")
    else:
        current_parts = tuple(int(part) for part in version.split("."))
        baseline_version = policy.get("baseline_version")
        baseline_parts = (
            tuple(int(part) for part in baseline_version.split("."))
            if isinstance(baseline_version, str)
            and re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", baseline_version)
            else ()
        )
        if (
            not baseline_parts
            or current_parts <= baseline_parts
            or current_parts[0] != 0
            or current_parts[1] <= baseline_parts[1]
            or current_parts[2] != 0
        ):
            reasons.append("candidate version is not a monotonic 0.x minor release")
    if policy.get("version_decision") != "APPROVED" or not policy.get("version_reviewer"):
        reasons.append("version decision/reviewer missing")
    if value.get("baseline_sha") != policy.get("baseline_sha"):
        reasons.append("release baseline SHA mismatch")
    if value.get("candidate_sha") != source.get("head"):
        reasons.append("release candidate SHA mismatch")
    if value.get("release_type") != policy.get("release_type"):
        reasons.append("release type mismatch")
    if value.get("source_after") != {
        field: source.get(field) for field in ("head", "tree", "paths_digest", "dirty")
    }:
        reasons.append("source changed during release collection")
    attestation = value.get("attestation")
    hosted_release = (
        isinstance(attestation, dict)
        and attestation.get("kind") == "github-actions-release"
        and isinstance(attestation.get("actor"), str)
        and bool(attestation.get("actor"))
    )
    local_release = isinstance(attestation, dict) and attestation.get("kind") == "local-release"
    expected_attestation_checks = (
        set(release_attestation(source)["checks"])
        if hosted_release
        else LOCAL_RELEASE_CHECKS
        if local_release
        else set()
    )
    if (
        not isinstance(attestation, dict)
        or (not hosted_release and not local_release)
        or attestation.get("eligible") is not True
        or not isinstance(attestation.get("checks"), dict)
        or set(attestation["checks"]) != expected_attestation_checks
        or not all(v is True for v in attestation["checks"].values())
    ):
        reasons.append("eligible hosted or explicit local release attestation missing or invalid")

    ordinary_receipt = check_identity(
        root, value.get("ordinary_receipt"), "ordinary receipt", reasons
    )
    if isinstance(ordinary_receipt, dict):
        ordinary_source = ordinary_receipt.get("source")
        if not isinstance(ordinary_source, dict) or any(
            ordinary_source.get(key) != source.get(key)
            for key in ("head", "tree", "paths_digest", "dirty")
        ):
            reasons.append("ordinary receipt is not the exact release source")
        path = root / value["ordinary_receipt"]["path"]
        code, _ = ordinary.validate(path, True, current=current, artifact_root=root)
        if code != 0:
            reasons.append("ordinary exact-source receipt is NOT_QUALIFIED")
        gates = ordinary_receipt.get("gates")
        results = gates.get("results", []) if isinstance(gates, dict) else []
        if not isinstance(results, list):
            results = []
        statuses = {
            r.get("id"): r.get("status")
            for r in results
            if isinstance(r, dict) and isinstance(r.get("id"), str)
        }
        release_gate_ids = required.get("ordinary_gate_ids")
        if not isinstance(release_gate_ids, list):
            release_gate_ids = []
            reasons.append("release ordinary gate list is malformed")
        for gate_id in release_gate_ids:
            if statuses.get(gate_id) != "PASS":
                reasons.append(
                    f"ordinary required gate {gate_id} is {statuses.get(gate_id, 'NOT_RUN')}"
                )
        generated = ordinary_receipt.get("generated_mutation_sweep")
        if not isinstance(generated, dict) or generated.get("status") != "PASS":
            reasons.append(
                "generated mutation is not quality PASS; REPORTED cannot waive "
                "missed/timeout/equivalent or omit unviable identities"
            )
    else:
        reasons.append("ordinary exact-source qualification is NOT_RUN")

    semver = check_identity(root, value.get("semver_manifest"), "semver manifest", reasons)
    reasons.extend(semver_problems(root, semver, source, policy))
    template = check_identity(
        root, value.get("adjudication_template"), "adjudication template", reasons
    )
    adjudication_ref = value.get("adjudication")
    if not isinstance(adjudication_ref, dict) or not str(
        adjudication_ref.get("path", "")
    ).startswith("target/release/input/"):
        reasons.append("human adjudication must be an external release input artifact")
    adjudication = check_identity(root, adjudication_ref, "adjudication", reasons)
    reasons.extend(
        adjudication_problems(
            root,
            adjudication,
            source.get("head"),
            policy.get("baseline_sha"),
            semver,
            template,
            attestation.get("actor") if hosted_release else None,
        )
    )
    plan = check_identity(root, value.get("plan"), "ticket plan", reasons)
    expected_closure = closure_report(root, plan, reasons)
    if value.get("closure") != expected_closure:
        reasons.append("stored 23-finding closure graph differs from ticket plan/source")
    finding_spec = check_identity(root, value.get("finding_spec"), "finding proof spec", reasons)
    finding_manifest = check_identity(
        root, value.get("finding_manifest"), "finding proof manifest", reasons
    )
    reasons.extend(
        finding_proof_problems(root, finding_manifest, finding_spec, plan, ordinary_receipt, source)
    )

    raw = value.get("raw_artifacts")
    expected_raw = required.get("required_raw_artifacts")
    if (
        not isinstance(raw, dict)
        or not isinstance(expected_raw, dict)
        or set(raw) != set(expected_raw)
    ):
        reasons.append("release raw artifact set incomplete")
        raw = {}
        expected_raw = expected_raw if isinstance(expected_raw, dict) else {}
    raw_payloads: dict[str, object | None] = {}
    for name, path in expected_raw.items():
        if not isinstance(raw.get(name), dict) or raw[name].get("path") != path:
            reasons.append(f"release raw artifact {name} path mismatch")
            continue
        raw_payloads[name] = check_identity(root, raw[name], name, reasons)
    gate_status_line = None
    if isinstance(ordinary_receipt, dict):
        gates = ordinary_receipt.get("gates")
        results = gates.get("results") if isinstance(gates, dict) else None
        if isinstance(results, list):
            gate_status_line = next(
                (
                    row.get("status_line")
                    for row in results
                    if isinstance(row, dict) and row.get("id") == "bench-iai"
                ),
                None,
            )
    reasons.extend(
        iai_raw_problems(
            root,
            raw_payloads.get("iai_baseline"),
            raw_payloads.get("iai_comparison"),
            raw.get("iai_output"),
            gate_status_line,
        )
    )
    coverage = raw_payloads.get("coverage_summary")
    metric_reasons: list[str] = []
    expected_metrics = metric_report(coverage, metric_reasons)
    reasons.extend(metric_reasons)
    if value.get("coverage") != expected_metrics:
        reasons.append("coverage metric collection states differ from raw summary")
    if (
        not isinstance(coverage, dict)
        or not isinstance(coverage.get("cargo_llvm_cov"), dict)
        or not coverage["cargo_llvm_cov"].get("version")
    ):
        reasons.append("coverage tool version missing from raw summary")
    if value.get("coverage_scope") != {
        "command": COVERAGE_COMMAND,
        "feature_scope": "workspace default features",
        "test_scope": "workspace all-targets excluding bench and doc-examples",
        "source_sha": source.get("head"),
    }:
        reasons.append("coverage source/feature/test scope mismatch")
    if (
        value.get("policy_decision_required")
        != "generated REPORTED vs ordinary quality PASS must be adjudicated without waiver"
    ):
        reasons.append("generated policy conflict disclosure missing")
    states = value.get("downstream_states")
    if not isinstance(states, dict) or set(states) != {
        "review",
        "merge",
        "consumer_qualification",
        "deployment",
        "activation",
    }:
        reasons.append("downstream state separation missing")
    stored = value.get("verdict")
    computed = {"status": "NOT_QUALIFIED" if reasons else "QUALIFIED", "reasons": reasons}
    if stored is not None and stored != computed:
        computed = {
            "status": "NOT_QUALIFIED",
            "reasons": [*reasons, "stored release verdict differs from recomputation"],
        }
    return computed


def collect(
    out: Path,
    ordinary_path: Path,
    semver_path: Path,
    adjudication_path: Path,
    local_qualified: bool = False,
) -> int:
    source = ordinary.source_identity()
    paths = {
        "policy": "tools/release/release-policy.json",
        "required": "tools/release/release-required.json",
        "adjudication_template": "tools/release/adjudication.json",
        "finding_spec": "tools/release/finding-proof-spec.json",
        "plan": "docs/bugbash/sep-21/tickets/plan.json",
    }
    policy = read_json(POLICY)
    value: dict = {
        "schema_version": 1,
        "source": source,
        "baseline_sha": policy.get("baseline_sha") if isinstance(policy, dict) else None,
        "candidate_sha": source["head"],
        "release_type": "minor",
    }
    for field, path in paths.items():
        value[field] = identity(REPO, path)
    value["adjudication"] = identity(REPO, str(adjudication_path))
    value["attestation"] = release_attestation(source, local_qualified)
    value["ordinary_receipt"] = identity(REPO, str(ordinary_path))
    value["semver_manifest"] = identity(REPO, str(semver_path))
    value["finding_manifest"] = identity(
        REPO, "target/release/finding-proof/finding-proof-manifest.json"
    )
    required = read_json(REQUIRED)
    raw_paths = required.get("required_raw_artifacts") if isinstance(required, dict) else None
    if not isinstance(raw_paths, dict):
        raw_paths = {}
    value["raw_artifacts"] = {name: identity(REPO, path) for name, path in raw_paths.items()}
    coverage_path = safe_artifact(REPO, raw_paths.get("coverage_summary"))
    coverage = read_json(coverage_path) if coverage_path is not None else None
    metric_reasons: list[str] = []
    value["coverage"] = metric_report(coverage, metric_reasons)
    value["coverage_scope"] = {
        "command": COVERAGE_COMMAND,
        "feature_scope": "workspace default features",
        "test_scope": "workspace all-targets excluding bench and doc-examples",
        "source_sha": source["head"],
    }
    value["closure"] = closure_report(REPO, read_json(PLAN), [])
    value["policy_decision_required"] = (
        "generated REPORTED vs ordinary quality PASS must be adjudicated without waiver"
    )
    value["downstream_states"] = {
        name: "NOT_RUN"
        for name in ("review", "merge", "consumer_qualification", "deployment", "activation")
    }
    after = ordinary.source_identity()
    value["source_after"] = {
        field: after[field] for field in ("head", "tree", "paths_digest", "dirty")
    }
    verdict = evaluate(value, root=REPO, current=after)
    value["verdict"] = verdict
    if not out.is_absolute():
        out = REPO / out
    if out.is_symlink() or not out.resolve().is_relative_to((REPO / "target").resolve()):
        raise ValueError("release receipt must be below target/ and not a symlink")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    print(f"release receipt: {out} {verdict['status']}")
    for reason in verdict["reasons"]:
        print(f"  - {reason}")
    return 0 if verdict["status"] == "QUALIFIED" else 1


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="action", required=True)
    c = sub.add_parser("collect")
    c.add_argument("--out", type=Path, default=Path("target/release/release-receipt.json"))
    c.add_argument("--ordinary", type=Path, default=Path("receipt.json"))
    c.add_argument(
        "--semver", type=Path, default=Path("target/release/semver/semver-manifest.json")
    )
    c.add_argument(
        "--adjudication", type=Path, default=Path("target/release/input/adjudication.json")
    )
    c.add_argument(
        "--local-qualified",
        action="store_true",
        help="request an exact-source local release verdict instead of hosted attestation",
    )
    v = sub.add_parser("validate")
    v.add_argument("receipt", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.action == "collect":
            return collect(
                args.out,
                args.ordinary,
                args.semver,
                args.adjudication,
                args.local_qualified,
            )
        value = read_json(args.receipt)
        verdict = evaluate(value, current=ordinary.source_identity())
        print(f"release receipt: {args.receipt} {verdict['status']}")
        for reason in verdict["reasons"]:
            print(f"  - {reason}")
        return 0 if verdict["status"] == "QUALIFIED" else 1
    except (OSError, ValueError, TypeError) as exc:
        print(f"release receipt NOT_QUALIFIED: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
