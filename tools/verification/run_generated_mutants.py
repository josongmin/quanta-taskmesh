#!/usr/bin/env python3
"""Run cargo-mutants in an isolated snapshot and preserve its exact denominator."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from campaign import (  # noqa: E402
    canonical_digest,
    create_isolated_campaign,
    evidence_envelope,
    exact_tool_version,
    execute,
    prepare_output_dir,
    sha256_file,
    utc_now,
    write_json,
)

REPO = Path(__file__).resolve().parents[2]
CATEGORIES = ("caught", "missed", "unviable", "timeout")
SUMMARY_CATEGORY = {
    "CaughtMutant": "caught",
    "MissedMutant": "missed",
    "Unviable": "unviable",
    "Timeout": "timeout",
}


def read_category_file(path: Path) -> list[str]:
    if not path.is_file():
        raise ValueError(f"missing cargo-mutants outcome file: {path.name}")
    values = [
        line.strip() for line in path.read_text(encoding="utf-8").splitlines() if line.strip()
    ]
    if len(values) != len(set(values)):
        raise ValueError(f"duplicate mutant IDs in {path.name}")
    return values


def mutant_name(value: object, *, context: str) -> str:
    if not isinstance(value, dict):
        raise ValueError(f"{context} mutant must be an object")
    name = value.get("name")
    if not isinstance(name, str) or not name.strip():
        raise ValueError(f"{context} mutant requires a non-empty name")
    return name


def parse_outcomes(mutants_out: Path) -> dict[str, list[str]]:
    """Cross-check cargo-mutants v27's three authoritative identity surfaces."""
    outcomes = {
        category: read_category_file(mutants_out / f"{category}.txt") for category in CATEGORIES
    }
    owners: dict[str, str] = {}
    for category, mutants in outcomes.items():
        for mutant in mutants:
            if mutant in owners:
                raise ValueError(
                    f"mutant appears in both {owners[mutant]} and {category}: {mutant}"
                )
            owners[mutant] = category
    if not owners:
        raise ValueError("cargo-mutants produced a zero-mutant denominator")
    mutants = json.loads((mutants_out / "mutants.json").read_text(encoding="utf-8"))
    raw_outcomes = json.loads((mutants_out / "outcomes.json").read_text(encoding="utf-8"))
    if not isinstance(mutants, list) or not mutants:
        raise ValueError("cargo-mutants mutants.json has no planned denominator")
    planned_names = [mutant_name(mutant, context="planned") for mutant in mutants]
    if len(planned_names) != len(set(planned_names)):
        raise ValueError("duplicate mutant names in mutants.json")
    outcome_rows = raw_outcomes.get("outcomes") if isinstance(raw_outcomes, dict) else None
    if not isinstance(outcome_rows, list):
        raise ValueError("cargo-mutants outcomes.json is malformed")
    if not all(isinstance(row, dict) for row in outcome_rows):
        raise ValueError("cargo-mutants outcomes.json contains a non-object row")
    baseline_rows = [row for row in outcome_rows if row.get("scenario") == "Baseline"]
    if len(baseline_rows) != 1 or baseline_rows[0].get("summary") != "Success":
        raise ValueError("cargo-mutants unmutated baseline did not complete successfully")
    executed: dict[str, str] = {}
    executed_by_category = {category: set() for category in CATEGORIES}
    for row in outcome_rows:
        scenario = row.get("scenario")
        if scenario == "Baseline":
            continue
        if not isinstance(scenario, dict) or "Mutant" not in scenario:
            raise ValueError("cargo-mutants outcomes.json contains an unknown scenario")
        name = mutant_name(scenario["Mutant"], context="executed")
        if name in executed:
            raise ValueError(f"duplicate mutant name in outcomes.json: {name}")
        summary = row.get("summary")
        category = SUMMARY_CATEGORY.get(summary)
        if category is None:
            raise ValueError(f"unsupported mutant outcome summary {summary!r}: {name}")
        executed[name] = category
        executed_by_category[category].add(name)

    planned = set(planned_names)
    categorized = set(owners)
    executed_names = set(executed)
    if planned != categorized or planned != executed_names:
        raise ValueError(
            "cargo-mutants identity sets differ: "
            f"planned_not_categorized={sorted(planned - categorized)} "
            f"planned_not_executed={sorted(planned - executed_names)} "
            f"categorized_only={sorted(categorized - planned)} "
            f"executed_only={sorted(executed_names - planned)}"
        )
    for category in CATEGORIES:
        categorized_names = set(outcomes[category])
        if categorized_names != executed_by_category[category]:
            raise ValueError(
                f"cargo-mutants {category} identities disagree between category file and "
                "outcomes.json"
            )
    return outcomes


def planned_baseline_identity(mutants_out: Path) -> tuple[list[str], str]:
    """Record the validated plan and successful baseline from structured raw files."""
    planned_rows = json.loads((mutants_out / "mutants.json").read_text(encoding="utf-8"))
    planned = sorted(mutant_name(item, context="planned") for item in planned_rows)
    outcome_document = json.loads((mutants_out / "outcomes.json").read_text(encoding="utf-8"))
    baseline = next(
        row for row in outcome_document["outcomes"] if row.get("scenario") == "Baseline"
    )
    return planned, canonical_digest(baseline)


def load_equivalents(path: Path | None, missed: list[str]) -> list[dict[str, str]]:
    if path is None:
        return []
    data = json.loads(path.read_text(encoding="utf-8"))
    reviews = data.get("equivalents", [])
    if not isinstance(reviews, list):
        raise ValueError("equivalents must be a list")
    seen: set[str] = set()
    for review in reviews:
        if not isinstance(review, dict):
            raise ValueError("each equivalent review must be an object")
        for key in ("mutant_id", "reachability_evidence", "reviewer"):
            if not isinstance(review.get(key), str) or not review[key].strip():
                raise ValueError(f"equivalent review requires non-empty {key}")
        mutant_id = review["mutant_id"]
        if mutant_id not in missed:
            raise ValueError(f"equivalent review does not name a missed mutant: {mutant_id}")
        if mutant_id in seen:
            raise ValueError(f"duplicate equivalent review: {mutant_id}")
        seen.add(mutant_id)
    return sorted(reviews, key=lambda review: review["mutant_id"])


def classify_generated(
    outcomes: dict[str, list[str]], equivalents: list[dict[str, str]]
) -> tuple[dict[str, int], str, list[str]]:
    equivalent_ids = {review["mutant_id"] for review in equivalents}
    remaining_missed = [mutant for mutant in outcomes["missed"] if mutant not in equivalent_ids]
    counts = {
        "caught": len(outcomes["caught"]),
        "missed": len(remaining_missed),
        "unviable": len(outcomes["unviable"]),
        "timeout": len(outcomes["timeout"]),
        "equivalent": len(equivalent_ids),
    }
    denominator = sum(counts.values())
    if denominator == 0:
        raise ValueError("generated denominator is zero")
    # An equivalence review preserves disposition but does not turn the campaign green.
    status = (
        "PASS"
        if counts
        == {"caught": denominator, "missed": 0, "unviable": 0, "timeout": 0, "equivalent": 0}
        else "FAIL"
    )
    problems = [name for name in ("missed", "unviable", "timeout", "equivalent") if counts[name]]
    return counts, status, problems


def apply_process_truth(status: str, problems: list[str], result: object) -> tuple[str, list[str]]:
    """A semantic PASS is valid only for a normal successful producer process."""
    if status == "PASS" and (
        getattr(result, "exit_code", None) != 0
        or getattr(result, "signal", None) is not None
        or bool(getattr(result, "timed_out", False))
    ):
        return "FAIL", sorted({*problems, "invalid_success_process"})
    return status, problems


def raw_manifest(root: Path) -> list[dict[str, object]]:
    artifacts = []
    for path in sorted(item for item in root.rglob("*") if item.is_file()):
        artifacts.append(
            {
                "path": str(path.relative_to(root)),
                "size": path.stat().st_size,
                "sha256": sha256_file(path),
            }
        )
    if not artifacts:
        raise ValueError("cargo-mutants raw artifact directory is empty")
    return artifacts


def command(args: argparse.Namespace, raw_parent: Path) -> list[str]:
    argv = [
        "cargo",
        "mutants",
        "--workspace",
        "--baseline",
        "run",
        "--no-shuffle",
        "--colors",
        "never",
        "--output",
        str(raw_parent),
        "--jobs",
        str(args.jobs),
    ]
    if args.timeout is not None:
        argv.extend(["--timeout", str(args.timeout)])
    for package in args.package:
        argv.extend(["--package", package])
    return argv


def tool_identity(source: Path) -> list[dict[str, str]]:
    versions = {
        "cargo-mutants": exact_tool_version(["cargo", "mutants", "--version"], cwd=source),
        "cargo": exact_tool_version(["cargo", "--version"], cwd=source),
        "rustc": exact_tool_version(["rustc", "-Vv"], cwd=source),
    }
    return [
        {
            "name": name,
            "version": version,
            "identity_sha256": canonical_digest({"name": name, "version": version}),
        }
        for name, version in versions.items()
    ]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, default=REPO / "target/sep21/v02/generated")
    parser.add_argument("--package", action="append", default=[])
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--timeout", type=int)
    parser.add_argument("--campaign-timeout", type=int, default=86400)
    parser.add_argument("--equivalents", type=Path)
    parser.add_argument("--keep-isolation", action="store_true")
    args = parser.parse_args(argv)
    if args.jobs != 1:
        parser.error(
            "generated evidence requires --jobs 1: one absolute campaign CARGO_TARGET_DIR "
            "must not be shared by parallel mutant jobs"
        )
    if args.campaign_timeout < 1 or (args.timeout is not None and args.timeout < 1):
        parser.error("timeout values must be positive")
    output_dir = prepare_output_dir(REPO, args.output_dir)
    campaign = create_isolated_campaign(REPO)
    try:
        isolated_raw_parent = campaign.root / "cargo-mutants-raw"
        argv_command = command(args, isolated_raw_parent)
        recorded_env = {"CARGO_TARGET_DIR": str(campaign.target)}
        result = execute(
            argv_command,
            cwd=campaign.source,
            env={**os.environ, **recorded_env},
            timeout_seconds=args.campaign_timeout,
        )
        raw_source = isolated_raw_parent / "mutants.out"
        raw_destination = output_dir / "raw" / "mutants.out"
        if raw_source.is_dir():
            shutil.copytree(raw_source, raw_destination)
        (output_dir / "raw").mkdir(parents=True, exist_ok=True)
        (output_dir / "raw" / "runner.stdout.log").write_text(result.stdout, encoding="utf-8")
        (output_dir / "raw" / "runner.stderr.log").write_text(result.stderr, encoding="utf-8")

        parse_error = None
        outcomes: dict[str, list[str]] = {category: [] for category in CATEGORIES}
        planned_mutants: list[str] = []
        baseline_sha256: str | None = None
        equivalents: list[dict[str, str]] = []
        counts = {**{category: 0 for category in CATEGORIES}, "equivalent": 0}
        semantic_status = "FAIL"
        problems: list[str] = []
        try:
            if result.timed_out:
                raise ValueError("cargo-mutants campaign timed out")
            if result.signal is not None:
                raise ValueError(f"cargo-mutants terminated by signal {result.signal}")
            outcomes = parse_outcomes(raw_destination)
            planned_mutants, baseline_sha256 = planned_baseline_identity(raw_destination)
            equivalents = load_equivalents(args.equivalents, outcomes["missed"])
            counts, semantic_status, problems = classify_generated(outcomes, equivalents)
            semantic_status, problems = apply_process_truth(semantic_status, problems, result)
        except (ValueError, OSError, json.JSONDecodeError) as error:
            parse_error = str(error)
            problems = ["incomplete_or_invalid_raw_artifact"]

        source_after = campaign.source_after(REPO)
        if source_after != campaign.source_before:
            semantic_status = "FAIL"
            problems.append("original_source_changed")
        raw_files = raw_manifest(output_dir / "raw")
        receipt = {
            "schema_version": 1,
            "kind": "generated-cargo-mutants",
            "generated_at": utc_now(),
            "campaign": campaign.identity(),
            "source_after_sha256": source_after,
            "source_unchanged": source_after == campaign.source_before,
            "command": {
                "argv": argv_command,
                "cwd": str(campaign.source),
                "environment": recorded_env,
                "sha256": canonical_digest({"argv": argv_command, "environment": recorded_env}),
            },
            "tools": tool_identity(campaign.source),
            "process": {
                "exit_code": result.exit_code,
                "signal": result.signal,
                "timed_out": result.timed_out,
                "started_at": result.started_at,
                "finished_at": result.finished_at,
                "duration_s": result.duration_s,
            },
            "status": semantic_status,
            "parse_error": parse_error,
            "counts": counts,
            "denominator": sum(counts.values()),
            "problems": sorted(set(problems)),
            "outcomes": outcomes,
            "planned_mutants": planned_mutants,
            "baseline_sha256": baseline_sha256,
            "equivalents": equivalents,
            "raw_artifacts": raw_files,
        }
        receipt_path = output_dir / "receipt.generated-mutations.json"
        write_json(receipt_path, receipt)
        raw_paths = sorted(path for path in (output_dir / "raw").rglob("*") if path.is_file())
        envelope_status = "TIMEOUT" if result.timed_out else semantic_status
        envelope = evidence_envelope(
            repo=REPO,
            campaign=campaign,
            producer_id="mutation-campaign",
            command=argv_command,
            environment=recorded_env,
            tools=receipt["tools"],
            config_paths=[REPO / "tools/verification/mutation-gate.json"],
            artifact_paths=[
                *[(path, "raw") for path in raw_paths],
                (receipt_path, "summary"),
            ],
            job="local-generated-mutations",
            status=envelope_status,
            exit_code=result.exit_code,
            started_at=result.started_at,
            finished_at=result.finished_at,
            selected_count=sum(counts.values()),
            executed_count=sum(counts.values()),
        )
        write_json(output_dir / "evidence-envelope.json", envelope)
        print(
            json.dumps(
                {"status": semantic_status, "counts": counts, "problems": problems}, sort_keys=True
            )
        )
        return 0 if semantic_status == "PASS" else 1
    finally:
        if not args.keep_isolation:
            campaign.cleanup()


if __name__ == "__main__":
    raise SystemExit(main())
