#!/usr/bin/env python3
# ruff: noqa: UP045 -- pyproject supports Python 3.9, which lacks PEP 604 unions.
"""Recheck paired host receipts and report run-level descriptive effects.

This is deliberately not a performance admission gate. B00's frozen SLO and
B04's measured calibration must exist before interpreting an effect as a win.
"""

from __future__ import annotations

import argparse
import random
import sys
from collections import Counter
from pathlib import Path
from typing import Any, Optional

import host_perf
from host_run import write_new

SCHEMA_VERSION = 1
RUN_KEYS = {"raw", "summary", "provenance", "binary", "topology", "resources"}
COMMON_IDENTITY_KEYS = {
    "rustc",
    "features",
    "topology_fingerprint",
    "host_fingerprint",
    "host_environment",
    "build_environment",
}


def artifact(root: Path, value: Any, label: str) -> bytes:
    if not isinstance(value, str) or not value:
        raise host_perf.ReceiptError(f"{label}: relative artifact path required")
    path = (root / value).resolve()
    if not path.is_relative_to(root.resolve()) or not path.is_file():
        raise host_perf.ReceiptError(f"{label}: artifact missing or outside manifest directory")
    return path.read_bytes()


def read_run(root: Path, files: Any, scenario: bytes, label: str) -> dict[str, Any]:
    if not isinstance(files, dict):
        raise host_perf.ReceiptError(f"{label}: run artifacts must be an object")
    host_perf.exact_keys(files, RUN_KEYS, label)
    data = {name: artifact(root, path, f"{label}.{name}") for name, path in files.items()}
    summary = host_perf.verify_receipt(
        data["raw"],
        scenario,
        data["summary"],
        provenance_bytes=data["provenance"],
        binary_bytes=data["binary"],
        topology_bytes=data["topology"],
        resource_bytes=data["resources"],
    )
    identity = summary["identity"]
    if identity["source_dirty"]:
        raise host_perf.ReceiptError(f"{label}: dirty source is diagnostic only")
    metrics = summary["metrics"]
    if (
        metrics["counts"]["not_submitted"]
        or metrics["counts"]["unanswered_at_settlement"]
        or metrics["late_snapshot_samples"]
    ):
        raise host_perf.ReceiptError(f"{label}: incomplete producer or observer population")
    provenance = host_perf.parse_object(data["provenance"], f"{label}.provenance")
    resources = host_perf.validate_resource_artifact(data["resources"], provenance["runner_pid"])
    if resources["status"] != "complete":
        raise host_perf.ReceiptError(f"{label}: resource sampling is unavailable")
    start = resources["sampling_started_epoch_ns"]
    end = resources["sampling_ended_epoch_ns"]
    if start == 0 or end <= start:
        raise host_perf.ReceiptError(f"{label}: invalid resource window")
    return {
        "identity": identity,
        "metrics": metrics,
        "binary_sha256": provenance["binary_sha256"],
        "provenance_sha256": host_perf.sha256(data["provenance"]),
        "artifact_sha256": {name: host_perf.sha256(body) for name, body in sorted(data.items())},
        "window": (start, end),
        "boot_time_ns": resources["boot_time_ns"],
        "resource_cadence_ms": resources["cadence_ms"],
    }


def effect_interval(differences: list[float]) -> dict[str, Any]:
    if len(differences) < 4:
        raise host_perf.ReceiptError("at least four independent pairs are required")
    mean = sum(differences) / len(differences)
    rng = random.Random(0)
    estimates = sorted(
        sum(rng.choice(differences) for _ in differences) / len(differences) for _ in range(10_000)
    )
    return {
        "paired_differences": differences,
        "mean_difference_per_second": mean,
        "descriptive_bootstrap_95_interval": [estimates[249], estimates[9749]],
        "resampling_unit": "independent_paired_run",
    }


def compare(manifest_bytes: bytes, root: Path) -> dict[str, Any]:
    manifest = host_perf.parse_object(manifest_bytes, "comparison manifest")
    host_perf.exact_keys(
        manifest, {"schema_version", "max_span_ns", "points"}, "comparison manifest"
    )
    if manifest["schema_version"] != SCHEMA_VERSION:
        raise host_perf.ReceiptError("unsupported comparison manifest version")
    max_span_ns = manifest["max_span_ns"]
    if type(max_span_ns) is not int or max_span_ns <= 0:
        raise host_perf.ReceiptError("comparison max_span_ns must be positive")
    points = manifest["points"]
    if not isinstance(points, list) or not points:
        raise host_perf.ReceiptError("comparison requires rate points")
    seen_rates: set[int] = set()
    all_runs: list[tuple[int, int, str]] = []
    reference: Optional[dict[str, Any]] = None
    arm_identities: dict[str, dict[str, Any]] = {}
    arm_binaries: dict[str, str] = {}
    boot_times: set[int] = set()
    resource_cadences: set[int] = set()
    workload_shape: Optional[dict[str, Any]] = None
    reference_mix: Optional[Counter[bytes]] = None
    run_artifacts = []
    result_points = []
    for point_index, point in enumerate(points):
        label = f"point {point_index}"
        if not isinstance(point, dict):
            raise host_perf.ReceiptError(f"{label}: object required")
        host_perf.exact_keys(point, {"rate_per_second", "scenario", "pairs"}, label)
        rate = point["rate_per_second"]
        if type(rate) is not int or rate <= 0 or rate in seen_rates:
            raise host_perf.ReceiptError(f"{label}: rate must be a unique positive integer")
        if seen_rates and rate <= max(seen_rates):
            raise host_perf.ReceiptError("rate points must be in increasing order")
        seen_rates.add(rate)
        scenario_bytes = artifact(root, point["scenario"], f"{label}.scenario")
        scenario = host_perf.parse_object(scenario_bytes, f"{label}.scenario")
        load = scenario.get("load")
        offers = scenario.get("offers")
        if (
            not isinstance(load, dict)
            or not isinstance(offers, list)
            or (
                type(load.get("injection_ms")) is not int
                or len(offers) * 1000 != rate * load["injection_ms"]
            )
        ):
            raise host_perf.ReceiptError(f"{label}: declared rate differs from intended arrivals")
        offer_mix = Counter(
            host_perf.canonical(
                {key: value for key, value in offer.items() if key != "send_time_ns"}
            )
            for offer in offers
            if isinstance(offer, dict)
        )
        if sum(offer_mix.values()) != len(offers):
            raise host_perf.ReceiptError(f"{label}: malformed offer")
        shape = {
            "topology": scenario.get("topology"),
            "classes": scenario.get("classes"),
            "load": {key: value for key, value in load.items() if key != "max_records"},
            "offer_kinds": sorted(offer_mix),
        }
        if workload_shape is None:
            workload_shape = shape
            reference_mix = offer_mix
        elif shape != workload_shape:
            raise host_perf.ReceiptError(f"{label}: workload body or topology changed across rates")
        elif reference_mix is not None and any(
            reference_mix[kind] * len(offers) != offer_mix[kind] * sum(reference_mix.values())
            for kind in offer_mix
        ):
            raise host_perf.ReceiptError(f"{label}: workload mix changed across rates")
        pairs = point["pairs"]
        if not isinstance(pairs, list) or len(pairs) < 4:
            raise host_perf.ReceiptError(f"{label}: at least four pairs required")
        orders = {"baseline_candidate": 0, "candidate_baseline": 0}
        per_cohort: dict[str, list[float]] = {}
        for pair_index, pair in enumerate(pairs):
            pair_label = f"{label} pair {pair_index}"
            if not isinstance(pair, dict):
                raise host_perf.ReceiptError(f"{pair_label}: object required")
            host_perf.exact_keys(pair, {"order", "baseline", "candidate"}, pair_label)
            order = pair["order"]
            if not isinstance(order, str) or order not in orders:
                raise host_perf.ReceiptError(f"{pair_label}: invalid arm order")
            orders[order] += 1
            runs = {
                arm: read_run(root, pair[arm], scenario_bytes, f"{pair_label} {arm}")
                for arm in ("baseline", "candidate")
            }
            first, second = order.split("_")
            if runs[first]["window"][1] > runs[second]["window"][0]:
                raise host_perf.ReceiptError(f"{pair_label}: arm windows overlap or reverse")
            for arm, run in runs.items():
                identity = run["identity"]
                common = {key: identity[key] for key in COMMON_IDENTITY_KEYS}
                if reference is None:
                    reference = common
                elif common != reference:
                    raise host_perf.ReceiptError(f"{pair_label}: host, feature or topology differs")
                if arm in arm_identities and identity != arm_identities[arm]:
                    raise host_perf.ReceiptError(f"{pair_label}: {arm} source identity changed")
                arm_identities.setdefault(arm, identity)
                if arm in arm_binaries and run["binary_sha256"] != arm_binaries[arm]:
                    raise host_perf.ReceiptError(f"{pair_label}: {arm} binary changed")
                arm_binaries.setdefault(arm, run["binary_sha256"])
                all_runs.append((*run["window"], run["provenance_sha256"]))
                run_artifacts.append(
                    {
                        "rate_per_second": rate,
                        "pair_index": pair_index,
                        "arm": arm,
                        "sha256": run["artifact_sha256"],
                    }
                )
                boot_times.add(run["boot_time_ns"])
                resource_cadences.add(run["resource_cadence_ms"])
            baseline_cohorts = runs["baseline"]["metrics"]["cohort_goodput"]
            candidate_cohorts = runs["candidate"]["metrics"]["cohort_goodput"]
            if baseline_cohorts.keys() != candidate_cohorts.keys():
                raise host_perf.ReceiptError(f"{pair_label}: cohort catalog differs")
            for cohort, base in baseline_cohorts.items():
                candidate = candidate_cohorts[cohort]
                if (
                    base["intended"] != candidate["intended"]
                    or base["slo_ns"] != candidate["slo_ns"]
                ):
                    raise host_perf.ReceiptError(f"{pair_label}: cohort population or SLO differs")
                per_cohort.setdefault(cohort, []).append(
                    candidate["slo_per_sec"] - base["slo_per_sec"]
                )
        if abs(orders["baseline_candidate"] - orders["candidate_baseline"]) > 1:
            raise host_perf.ReceiptError(f"{label}: arm order is not balanced")
        result_points.append(
            {
                "rate_per_second": rate,
                "scenario_sha256": host_perf.sha256(scenario_bytes),
                "pairs": len(pairs),
                "cohorts": {
                    name: effect_interval(values) for name, values in sorted(per_cohort.items())
                },
            }
        )
    if len({digest for _, _, digest in all_runs}) != len(all_runs):
        raise host_perf.ReceiptError("a run was reused across comparison roles or rates")
    if len(boot_times) != 1 or len(resource_cadences) != 1:
        raise host_perf.ReceiptError("comparison host boot or resource cadence changed")
    windows = sorted(all_runs)
    if any(previous[1] > current[0] for previous, current in zip(windows, windows[1:])):
        raise host_perf.ReceiptError("comparison runs overlap")
    if windows[-1][1] - windows[0][0] > max_span_ns:
        raise host_perf.ReceiptError("comparison acquisition exceeds declared span")
    return {
        "schema_version": SCHEMA_VERSION,
        "status": "DESCRIPTIVE_ONLY",
        "performance": "UNQUALIFIED",
        "reason": "B00 contract and B04 measured calibration are not verified by this tool",
        "manifest_sha256": host_perf.sha256(manifest_bytes),
        "max_span_ns": max_span_ns,
        "baseline_source_head": arm_identities["baseline"]["source_head"],
        "candidate_source_head": arm_identities["candidate"]["source_head"],
        "run_artifacts": run_artifacts,
        "points": result_points,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    manifest_bytes: Optional[bytes] = None
    try:
        manifest_bytes = args.manifest.read_bytes()
        report = compare(manifest_bytes, args.manifest.resolve().parent)
        write_new(args.output, host_perf.canonical(report) + b"\n")
        print(f"DESCRIPTIVE_ONLY performance=UNQUALIFIED output={args.output}")
        return 0
    except (OSError, host_perf.ReceiptError) as error:
        rejected = {
            "schema_version": SCHEMA_VERSION,
            "status": "REJECTED",
            "performance": "UNQUALIFIED",
            "reason": str(error),
            "manifest_sha256": host_perf.sha256(manifest_bytes) if manifest_bytes else None,
        }
        if not args.output.exists():
            try:
                write_new(args.output, host_perf.canonical(rejected) + b"\n")
            except OSError:
                pass
        print(f"host comparison rejected: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
