#!/usr/bin/env python3
"""Evaluate measured host-control budgets without granting performance status.

The policy is an explicit, source- and scenario-bound input. This tool evaluates
retained control measurements; it does not acquire them, freeze B00, or qualify
a rate series.
"""

from __future__ import annotations

import argparse
import math
import sys
from pathlib import Path
from typing import Any

import host_aa
import host_controls
import host_perf
import host_sampler
from host_run import write_new

SCHEMA_VERSION = 2
POLICY_KEYS = {
    "schema_version",
    "scenario_sha256",
    "source_head",
    "min_pairs",
    "min_success_samples_per_cohort",
    "max_span_ns",
    "max_generator_lag_ns",
    "max_host_lag_ns",
    "max_aa_relative_delta",
    "max_aa_p99_relative_delta",
    "max_snapshot_relative_delta",
    "max_snapshot_p99_relative_delta",
    "max_recorder_response_fraction_delta",
    "max_recorder_process_duration_relative_delta",
    "max_sampler_relative_delta",
    "max_sampler_p99_relative_delta",
}


def finite_fraction(value: Any, label: str) -> float:
    if type(value) not in (int, float) or not 0 <= value <= 1 or not math.isfinite(value):
        raise host_perf.ReceiptError(f"{label} must be a finite fraction in [0, 1]")
    return float(value)


def parse_policy(data: bytes) -> dict[str, Any]:
    policy = host_perf.parse_object(data, "control budget policy")
    host_perf.exact_keys(policy, POLICY_KEYS, "control budget policy")
    if type(policy["schema_version"]) is not int or policy["schema_version"] != SCHEMA_VERSION:
        raise host_perf.ReceiptError("unsupported control budget policy version")
    for key in ("scenario_sha256", "source_head"):
        value = policy[key]
        if (
            not isinstance(value, str)
            or len(value) != (64 if key == "scenario_sha256" else 40)
            or any(char not in "0123456789abcdef" for char in value)
        ):
            raise host_perf.ReceiptError(f"control budget {key} is invalid")
    pairs = host_perf.nat(policy["min_pairs"], "control budget min_pairs")
    if pairs < 2 or pairs > 50:
        raise host_perf.ReceiptError("control budget min_pairs must be 2..=50")
    if (
        host_perf.nat(
            policy["min_success_samples_per_cohort"],
            "control budget min_success_samples_per_cohort",
        )
        == 0
    ):
        raise host_perf.ReceiptError("control budget success sample floor must be positive")
    for key in ("max_span_ns", "max_generator_lag_ns", "max_host_lag_ns"):
        if host_perf.nat(policy[key], f"control budget {key}") == 0:
            raise host_perf.ReceiptError(f"control budget {key} must be positive")
    for key in (
        "max_aa_relative_delta",
        "max_aa_p99_relative_delta",
        "max_snapshot_relative_delta",
        "max_snapshot_p99_relative_delta",
        "max_recorder_response_fraction_delta",
        "max_recorder_process_duration_relative_delta",
        "max_sampler_relative_delta",
        "max_sampler_p99_relative_delta",
    ):
        finite_fraction(policy[key], f"control budget {key}")
    return policy


def pair_runs(runs: Any, minimum: int, label: str) -> list[tuple[dict, dict]]:
    if not isinstance(runs, list) or len(runs) % 2 or len(runs) < minimum * 2:
        raise host_perf.ReceiptError(f"{label} lacks the declared independent pairs")
    return [(runs[index], runs[index + 1]) for index in range(0, len(runs), 2)]


def balanced_pairs(runs: Any, minimum: int, label: str, modes: tuple[str, str]) -> list:
    pairs = pair_runs(runs, minimum, label)
    orders = [(first["mode"], second["mode"]) for first, second in pairs]
    forward, reverse = modes, tuple(reversed(modes))
    if any(order not in (forward, reverse) for order in orders):
        raise host_perf.ReceiptError(f"{label} pair does not contain both control arms")
    if orders.count(forward) != orders.count(reverse):
        raise host_perf.ReceiptError(f"{label} pair order is unbalanced")
    return pairs


def cohort_rates(run: dict, label: str) -> dict[str, float]:
    cohorts = run["metrics"]["cohort_goodput"]
    if not isinstance(cohorts, dict) or not cohorts:
        raise host_perf.ReceiptError(f"{label} has no SLO-goodput cohorts")
    return {name: cohort["slo_per_sec"] for name, cohort in cohorts.items()}


def relative_delta(first: float, second: float, label: str) -> float:
    if (
        type(first) not in (int, float)
        or type(second) not in (int, float)
        or not math.isfinite(first)
        or not math.isfinite(second)
        or first < 0
        or second < 0
        or max(first, second) == 0
    ):
        raise host_perf.ReceiptError(f"{label} has no positive comparable value")
    return abs(first - second) / max(first, second)


def rate_effects(first: dict, second: dict, label: str) -> dict[str, float]:
    first_rates = cohort_rates(first, label)
    second_rates = cohort_rates(second, label)
    if first_rates.keys() != second_rates.keys():
        raise host_perf.ReceiptError(f"{label} cohort catalog differs")
    return {
        name: relative_delta(first_rates[name], second_rates[name], f"{label}/{name}")
        for name in first_rates
    }


def p99_effects(first: dict, second: dict, minimum: int, label: str) -> dict[str, float]:
    first_populations = first["metrics"]["success_latency_by_class_path"]
    second_populations = second["metrics"]["success_latency_by_class_path"]
    if (
        not isinstance(first_populations, dict)
        or not first_populations
        or not isinstance(second_populations, dict)
        or first_populations.keys() != second_populations.keys()
    ):
        raise host_perf.ReceiptError(f"{label} p99 cohort catalog differs")
    effects = {}
    for name, population in first_populations.items():
        other = second_populations[name]
        if (
            host_perf.nat(population["count"], f"{label}/{name}.count") < minimum
            or host_perf.nat(other["count"], f"{label}/{name}.other_count") < minimum
        ):
            raise host_perf.ReceiptError(f"{label}/{name} has a thin p99 population")
        effects[name] = relative_delta(population["p99_ns"], other["p99_ns"], f"{label}/{name} p99")
    return effects


def recorder_response_effect(first: dict, second: dict) -> float:
    first_counts = first["metrics"]["counts"]
    second_counts = second["metrics"]["counts"]
    intended = first_counts["intended"]
    if intended <= 0 or intended != second_counts["intended"]:
        raise host_perf.ReceiptError("recorder pair intended population differs")
    return abs(first_counts["responded"] - second_counts["responded"]) / intended


def read_bound_artifact(path: Path, expected_sha256: str, label: str) -> bytes:
    data = path.read_bytes()
    if host_perf.sha256(data) != expected_sha256:
        raise host_perf.ReceiptError(f"{label} changed after control verification")
    return data


def run_health(metrics: dict, label: str, index: int) -> dict:
    counts = metrics["counts"]
    return {
        "study": label,
        "index": index,
        "not_submitted": host_perf.nat(counts["not_submitted"], f"{label}.not_submitted"),
        "unanswered_at_settlement": host_perf.nat(
            counts["unanswered_at_settlement"], f"{label}.unanswered_at_settlement"
        ),
        "max_producer_lag_ns": host_perf.nat(
            metrics["max_producer_lag_ns"], f"{label}.max_producer_lag_ns"
        ),
        "late_snapshot_samples": host_perf.nat(
            metrics["late_snapshot_samples"], f"{label}.late_snapshot_samples"
        ),
    }


def study_health(directory: Path, runs: list, label: str) -> list[dict]:
    observations = []
    for index, run in enumerate(runs):
        # Minimal mode deliberately has no summary or Snapshot observation.
        # Its typed raw verifier rejects Snapshot sampling and unresolved callers.
        if run["summary_sha256"] is None:
            metrics = {**run["metrics"], "late_snapshot_samples": 0}
        else:
            summary = host_perf.parse_object(
                read_bound_artifact(
                    host_aa.paths(directory, index)["summary"],
                    run["summary_sha256"],
                    f"{label} summary {index}",
                ),
                f"{label} summary {index}",
            )
            metrics = summary["metrics"]
        observations.append(run_health(metrics, label, index))
    return observations


def assess(
    policy_bytes: bytes,
    scenario_path: Path,
    host_raw: Path,
    host_summary: Path,
    generator_raw: Path,
    aa_directory: Path,
    snapshot_directory: Path,
    recorder_directory: Path,
    sampler_directory: Path,
    bundle_path: Path,
) -> dict[str, Any]:
    policy = parse_policy(policy_bytes)
    scenario_bytes = scenario_path.read_bytes()
    if host_perf.sha256(scenario_bytes) != policy["scenario_sha256"]:
        raise host_perf.ReceiptError("control budget scenario differs")
    bundle_bytes = bundle_path.read_bytes()
    bundle = host_controls.verify_bundle(
        bundle_bytes,
        scenario_path,
        host_raw,
        host_summary,
        generator_raw,
        aa_directory,
        snapshot_directory,
        recorder_directory,
    )
    identity = bundle["identity"]
    if bundle["scenario_sha256"] != policy["scenario_sha256"]:
        raise host_perf.ReceiptError("control bundle scenario differs from budget policy")
    if identity["source_dirty"] or identity["source_head"] != policy["source_head"]:
        raise host_perf.ReceiptError("control budget requires its declared clean source")
    if bundle["observed_span_ns"] > policy["max_span_ns"]:
        raise host_perf.ReceiptError("control budget acquisition span exceeded")
    digests = bundle["artifact_sha256"]
    target = host_perf.parse_object(
        read_bound_artifact(host_summary, digests["target"]["summary"], "target summary"),
        "target summary",
    )
    aa = host_perf.parse_object(
        read_bound_artifact(aa_directory / "aa-bundle.json", digests["aa_bundle"], "A/A bundle"),
        "A/A bundle",
    )
    snapshot = host_perf.parse_object(
        read_bound_artifact(
            snapshot_directory / "snapshot-bundle.json",
            digests["snapshot_bundle"],
            "Snapshot bundle",
        ),
        "Snapshot bundle",
    )
    recorder = host_perf.parse_object(
        read_bound_artifact(
            recorder_directory / "recorder-bundle.json",
            digests["recorder_bundle"],
            "recorder bundle",
        ),
        "recorder bundle",
    )
    sampler_bytes = (sampler_directory / "sampler-bundle.json").read_bytes()
    sampler = host_sampler.verify_bundle(sampler_bytes, sampler_directory, scenario_bytes)
    if (
        sampler["identity"] != identity
        or sampler["binary_sha256"] != bundle["host_binary_sha256"]
        or sampler["scenario_sha256"] != policy["scenario_sha256"]
        or sampler["runs"][0]["boot_time_ns"] != bundle["boot_time_ns"]
        or sampler["runs"][0]["resource_cadence_ms"] != bundle["resource_cadence_ms"]
    ):
        raise host_perf.ReceiptError("sampler source, binary, boot or cadence differs")
    resource_artifacts = [
        (host_controls.sidecars(host_raw)["resources"], digests["target"]["resources"]),
        (host_controls.sidecars(generator_raw)["resources"], digests["generator"]["resources"]),
    ]
    for directory, runs in (
        (aa_directory, aa["runs"]),
        (snapshot_directory, snapshot["runs"]),
        (recorder_directory, recorder["runs"]),
    ):
        resource_artifacts.extend(
            (host_aa.paths(directory, index)["resources"], run["resources_sha256"])
            for index, run in enumerate(runs)
        )
    windows = []
    for path, digest in resource_artifacts:
        resource = host_perf.parse_object(
            read_bound_artifact(path, digest, "control resources"), "control resources"
        )
        windows.append((resource["sampling_started_epoch_ns"], resource["sampling_ended_epoch_ns"]))
    windows.extend(tuple(run["window_epoch_ns"]) for run in sampler["runs"])
    windows.sort()
    if any(previous[1] > current[0] for previous, current in zip(windows, windows[1:])):
        raise host_perf.ReceiptError("sampler and control process windows overlap")
    combined_span_ns = windows[-1][1] - windows[0][0]
    health = [run_health(target["metrics"], "target", 0)]
    for directory, study, label in (
        (aa_directory, aa, "A/A"),
        (snapshot_directory, snapshot, "Snapshot"),
        (recorder_directory, recorder, "recorder"),
        (sampler_directory, sampler, "sampler"),
    ):
        health.extend(study_health(directory, study["runs"], label))
    aa_effects = [
        rate_effects(first, second, f"A/A pair {index}")
        for index, (first, second) in enumerate(pair_runs(aa["runs"], policy["min_pairs"], "A/A"))
    ]
    aa_p99_effects = [
        p99_effects(first, second, policy["min_success_samples_per_cohort"], f"A/A pair {index}")
        for index, (first, second) in enumerate(pair_runs(aa["runs"], policy["min_pairs"], "A/A"))
    ]
    snapshot_effects = []
    snapshot_p99_effects = []
    for index, (first, second) in enumerate(
        balanced_pairs(snapshot["runs"], policy["min_pairs"], "Snapshot", ("on", "off"))
    ):
        off, on = (first, second) if first["mode"] == "off" else (second, first)
        snapshot_effects.append(rate_effects(off, on, f"Snapshot pair {index}"))
        snapshot_p99_effects.append(
            p99_effects(off, on, policy["min_success_samples_per_cohort"], f"Snapshot pair {index}")
        )
    recorder_effects = []
    recorder_duration_effects = []
    for first, second in balanced_pairs(
        recorder["runs"], policy["min_pairs"], "recorder", ("full", "minimal")
    ):
        full, minimal = (first, second) if first["mode"] == "full" else (second, first)
        recorder_effects.append(recorder_response_effect(full, minimal))
        recorder_duration_effects.append(
            relative_delta(
                full["probe_process_duration_ns"],
                minimal["probe_process_duration_ns"],
                "recorder probe process duration",
            )
        )
    max_aa = max(value for pair in aa_effects for value in pair.values())
    max_snapshot = max(value for pair in snapshot_effects for value in pair.values())
    max_recorder = max(recorder_effects)
    sampler_effects = []
    sampler_p99_effects = []
    for index, (first, second) in enumerate(
        balanced_pairs(sampler["runs"], policy["min_pairs"], "sampler", ("on", "off"))
    ):
        off, on = (first, second) if first["mode"] == "off" else (second, first)
        sampler_effects.append(rate_effects(off, on, f"sampler pair {index}"))
        sampler_p99_effects.append(
            p99_effects(off, on, policy["min_success_samples_per_cohort"], f"sampler pair {index}")
        )
    max_sampler = max(value for pair in sampler_effects for value in pair.values())
    max_aa_p99 = max(value for pair in aa_p99_effects for value in pair.values())
    max_snapshot_p99 = max(value for pair in snapshot_p99_effects for value in pair.values())
    max_sampler_p99 = max(value for pair in sampler_p99_effects for value in pair.values())
    observations = {
        "generator_not_submitted": bundle["generator_not_submitted"],
        "host_not_submitted": bundle["host_not_submitted"],
        "generator_max_lag_ns": bundle["generator_max_lag_ns"],
        "host_max_lag_ns": bundle["host_max_lag_ns"],
        "max_aa_relative_delta": max_aa,
        "max_aa_p99_relative_delta": max_aa_p99,
        "max_snapshot_relative_delta": max_snapshot,
        "max_snapshot_p99_relative_delta": max_snapshot_p99,
        "max_recorder_response_fraction_delta": max_recorder,
        "max_recorder_process_duration_relative_delta": max(recorder_duration_effects),
        "max_sampler_relative_delta": max_sampler,
        "max_sampler_p99_relative_delta": max_sampler_p99,
        "aa_pairs": len(aa_effects),
        "snapshot_pairs": len(snapshot_effects),
        "recorder_pairs": len(recorder_effects),
        "sampler_pairs": len(sampler_effects),
        "combined_span_ns": combined_span_ns,
        "run_health": health,
    }
    violations = []
    for run in health:
        for key in ("not_submitted", "unanswered_at_settlement", "late_snapshot_samples"):
            if run[key] != 0:
                violations.append(f"{run['study']}/{run['index']}/{key}")
        if run["max_producer_lag_ns"] > policy["max_host_lag_ns"]:
            violations.append(f"{run['study']}/{run['index']}/max_producer_lag_ns")
    for key in ("generator_not_submitted", "host_not_submitted"):
        if observations[key] != 0:
            violations.append(key)
    if combined_span_ns > policy["max_span_ns"]:
        violations.append("combined_span_ns")
    for observed, limit in (
        ("generator_max_lag_ns", "max_generator_lag_ns"),
        ("host_max_lag_ns", "max_host_lag_ns"),
        ("max_aa_relative_delta", "max_aa_relative_delta"),
        ("max_aa_p99_relative_delta", "max_aa_p99_relative_delta"),
        ("max_snapshot_relative_delta", "max_snapshot_relative_delta"),
        ("max_snapshot_p99_relative_delta", "max_snapshot_p99_relative_delta"),
        ("max_recorder_response_fraction_delta", "max_recorder_response_fraction_delta"),
        (
            "max_recorder_process_duration_relative_delta",
            "max_recorder_process_duration_relative_delta",
        ),
        ("max_sampler_relative_delta", "max_sampler_relative_delta"),
        ("max_sampler_p99_relative_delta", "max_sampler_p99_relative_delta"),
    ):
        if observations[observed] > policy[limit]:
            violations.append(observed)
    return {
        "schema_version": SCHEMA_VERSION,
        "status": "BUDGET_PASS_DIAGNOSTIC" if not violations else "BUDGET_FAIL",
        "performance": "UNQUALIFIED",
        "reason": "B00 contract and fixed-host rate series require separate proof",
        "policy_sha256": host_perf.sha256(policy_bytes),
        "bundle_sha256": host_perf.sha256(bundle_bytes),
        "sampler_bundle_sha256": host_perf.sha256(sampler_bytes),
        "scenario_sha256": policy["scenario_sha256"],
        "source_head": policy["source_head"],
        "process_windows_epoch_ns": windows,
        "observations": observations,
        "violations": violations,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("policy", type=Path)
    parser.add_argument("scenario", type=Path)
    parser.add_argument("host_raw", type=Path)
    parser.add_argument("host_summary", type=Path)
    parser.add_argument("generator_raw", type=Path)
    parser.add_argument("aa_directory", type=Path)
    parser.add_argument("snapshot_directory", type=Path)
    parser.add_argument("recorder_directory", type=Path)
    parser.add_argument("sampler_directory", type=Path)
    parser.add_argument("control_bundle", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        policy_bytes = args.policy.read_bytes()
        report = assess(
            policy_bytes,
            args.scenario,
            args.host_raw,
            args.host_summary,
            args.generator_raw,
            args.aa_directory,
            args.snapshot_directory,
            args.recorder_directory,
            args.sampler_directory,
            args.control_bundle,
        )
    except (OSError, ValueError, KeyError, TypeError, host_perf.ReceiptError) as error:
        report = {
            "schema_version": SCHEMA_VERSION,
            "status": "REJECTED",
            "performance": "UNQUALIFIED",
            "reason": str(error),
        }
    try:
        write_new(args.output, host_perf.canonical(report) + b"\n")
    except (OSError, host_perf.ReceiptError) as error:
        print(f"control budget report could not be retained: {error}", file=sys.stderr)
        return 1
    print(f"{report['status']} performance=UNQUALIFIED output={args.output}")
    return 0 if report["status"] == "BUDGET_PASS_DIAGNOSTIC" else 1


if __name__ == "__main__":
    raise SystemExit(main())
