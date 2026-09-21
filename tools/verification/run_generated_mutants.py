#!/usr/bin/env python3
"""Run cargo-mutants in an isolated snapshot and preserve its exact denominator."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import sys
from pathlib import Path

import tomllib

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
    quality_denominator = denominator - counts["unviable"]
    # cargo-mutants could not compile unviable mutations, so they remain visible in
    # the full denominator but are not evidence for or against the semantic oracle.
    # An equivalence review preserves disposition but does not turn the campaign green.
    status = (
        "PASS"
        if counts["caught"] > 0
        and counts["caught"] == quality_denominator
        and counts["missed"] == counts["timeout"] == counts["equivalent"] == 0
        else "FAIL"
    )
    problems = [name for name in ("missed", "timeout", "equivalent") if counts[name]]
    if quality_denominator == 0:
        problems.append("no_scored_mutants")
    return counts, status, problems


def quality_accounting(counts: dict[str, int]) -> dict[str, object]:
    """Separate the complete campaign denominator from score-eligible outcomes."""
    denominator = sum(counts.values())
    return {
        "numerator": counts["caught"],
        "denominator": denominator - counts["unviable"],
        "excluded": {"unviable": counts["unviable"]},
    }


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


def unfiltered_list_command(args: argparse.Namespace) -> list[str]:
    """Enumerate the pre-exclusion denominator for auditable scope accounting."""
    argv = [
        "cargo",
        "mutants",
        "--workspace",
        "--list",
        "--json",
        "--no-config",
    ]
    for package in args.package:
        argv.extend(["--package", package])
    return argv


def parse_unfiltered_listing(stdout: str) -> list[str]:
    rows = json.loads(stdout)
    if not isinstance(rows, list) or not rows:
        raise ValueError("cargo-mutants unfiltered listing has no mutants")
    names = [mutant_name(row, context="unfiltered") for row in rows]
    if len(names) != len(set(names)):
        raise ValueError("duplicate mutant names in unfiltered listing")
    return sorted(names)


def excluded_mutants(
    unfiltered: list[str], planned: list[str], config_path: Path, *, require_every_pattern: bool
) -> list[str]:
    """Prove every omitted identity is selected by a checked-in narrow regex."""
    unfiltered_set = set(unfiltered)
    planned_set = set(planned)
    if not planned_set <= unfiltered_set:
        raise ValueError(
            "planned cargo-mutants denominator is not a subset of the unfiltered listing: "
            f"planned_only={sorted(planned_set - unfiltered_set)}"
        )
    config = tomllib.loads(config_path.read_text(encoding="utf-8"))
    patterns = config.get("exclude_re")
    if (
        not isinstance(patterns, list)
        or not patterns
        or not all(isinstance(pattern, str) and pattern for pattern in patterns)
    ):
        raise ValueError("mutants config requires non-empty exclude_re patterns")
    compiled = [re.compile(pattern) for pattern in patterns]
    excluded = sorted(unfiltered_set - planned_set)
    unmatched = [name for name in excluded if not any(pattern.search(name) for pattern in compiled)]
    if unmatched:
        raise ValueError(f"unapproved excluded mutant identities: {unmatched}")
    if require_every_pattern:
        unused = [
            pattern.pattern
            for pattern in compiled
            if not any(pattern.search(name) for name in excluded)
        ]
        if unused:
            raise ValueError(f"mutation exclusion patterns matched no identities: {unused}")
    return excluded


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


def execution_environment(parent: dict[str, str]) -> dict[str, str]:
    """Preserve the runner environment without collapsing per-job build dirs."""
    environment = parent.copy()
    environment.pop("CARGO_TARGET_DIR", None)
    return environment


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
    if not 1 <= args.jobs <= 3:
        parser.error("generated evidence requires a conservative --jobs value from 1 through 3")
    if args.campaign_timeout < 1 or (args.timeout is not None and args.timeout < 1):
        parser.error("timeout values must be positive")
    output_dir = prepare_output_dir(REPO, args.output_dir)
    campaign = create_isolated_campaign(REPO)
    try:
        isolated_raw_parent = campaign.root / "cargo-mutants-raw"
        argv_command = command(args, isolated_raw_parent)
        # cargo-mutants creates one source/build directory per parallel job.
        # An inherited absolute CARGO_TARGET_DIR would collapse those isolated
        # builds back onto one shared Cargo lock and target tree, so remove it.
        execution_env = execution_environment(dict(os.environ))
        recorded_env: dict[str, str] = {}
        discovery_argv = unfiltered_list_command(args)
        discovery = execute(
            discovery_argv,
            cwd=campaign.source,
            env=execution_env,
            timeout_seconds=min(args.campaign_timeout, 600),
        )
        result = execute(
            argv_command,
            cwd=campaign.source,
            env=execution_env,
            timeout_seconds=args.campaign_timeout,
        )
        raw_source = isolated_raw_parent / "mutants.out"
        raw_destination = output_dir / "raw" / "mutants.out"
        if raw_source.is_dir():
            shutil.copytree(raw_source, raw_destination)
        (output_dir / "raw").mkdir(parents=True, exist_ok=True)
        (output_dir / "raw" / "unfiltered-mutants.json").write_text(
            discovery.stdout, encoding="utf-8"
        )
        (output_dir / "raw" / "unfiltered-mutants.stderr.log").write_text(
            discovery.stderr, encoding="utf-8"
        )
        (output_dir / "raw" / "runner.stdout.log").write_text(result.stdout, encoding="utf-8")
        (output_dir / "raw" / "runner.stderr.log").write_text(result.stderr, encoding="utf-8")

        parse_error = None
        outcomes: dict[str, list[str]] = {category: [] for category in CATEGORIES}
        planned_mutants: list[str] = []
        baseline_sha256: str | None = None
        equivalents: list[dict[str, str]] = []
        excluded: list[str] = []
        counts = {**{category: 0 for category in CATEGORIES}, "equivalent": 0}
        semantic_status = "FAIL"
        problems: list[str] = []
        try:
            if result.timed_out:
                raise ValueError("cargo-mutants campaign timed out")
            if result.signal is not None:
                raise ValueError(f"cargo-mutants terminated by signal {result.signal}")
            if discovery.timed_out or discovery.signal is not None or discovery.exit_code != 0:
                raise ValueError("cargo-mutants unfiltered discovery did not complete successfully")
            outcomes = parse_outcomes(raw_destination)
            planned_mutants, baseline_sha256 = planned_baseline_identity(raw_destination)
            excluded = excluded_mutants(
                parse_unfiltered_listing(discovery.stdout),
                planned_mutants,
                campaign.source / ".cargo/mutants.toml",
                require_every_pattern=not args.package,
            )
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
            "schema_version": 2,
            "kind": "generated-cargo-mutants",
            "generated_at": utc_now(),
            "campaign": campaign.identity(),
            "source_after_sha256": source_after,
            "source_unchanged": source_after == campaign.source_before,
            "command": {
                "argv": argv_command,
                "cwd": str(campaign.source),
                "environment": recorded_env,
                "build_isolation": "cargo-mutants-one-build-directory-per-job",
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
            "discovery_process": {
                "argv": discovery_argv,
                "exit_code": discovery.exit_code,
                "signal": discovery.signal,
                "timed_out": discovery.timed_out,
                "duration_s": discovery.duration_s,
            },
            "status": semantic_status,
            "parse_error": parse_error,
            "counts": counts,
            "denominator": sum(counts.values()),
            "quality": quality_accounting(counts),
            "limitations": ["unviable"] if counts["unviable"] else [],
            "problems": sorted(set(problems)),
            "outcomes": outcomes,
            "planned_mutants": planned_mutants,
            "excluded_mutants": excluded,
            "excluded_count": len(excluded),
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
            config_paths=[
                REPO / "tools/verification/mutation-gate.json",
                REPO / ".cargo/mutants.toml",
            ],
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
