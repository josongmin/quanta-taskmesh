#!/usr/bin/env python3
"""Admit a preregistered full-host series from reexecuted build/oracle evidence.

The result is a scoped highest-tested-rate observation. It is neither an
industry ranking nor a guarantee about untested rates, hosts or public paths.
"""

from __future__ import annotations

import argparse
import math
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

import host_build
import host_compare
import host_control_assess
import host_perf
import host_study
from host_run import write_new

VERSION = 1
CONTRACT_KEYS = {
    "schema_version",
    "status",
    "source_head",
    "build_witness_sha256",
    "rate_grid",
    "scenario_sha256_by_rate",
    "control_policy_sha256_by_rate",
    "warmup_ms",
    "injection_ms",
    "repetitions_per_rate",
    "minimum_slo_fraction",
    "slo_ms_by_class",
    "min_success_samples_per_cohort",
    "max_p99_relative_interval_width",
    "max_span_ns",
}
CONTROL_ROLES = (
    "policy",
    "scenario_path",
    "host_raw",
    "host_summary",
    "generator_raw",
    "aa_directory",
    "snapshot_directory",
    "recorder_directory",
    "sampler_directory",
    "bundle_path",
)
ORACLE = [
    "cargo",
    "test",
    "--locked",
    "-p",
    "taskmesh",
    "--test",
    "hardening_mixed_overload",
    "--test",
    "hardening_deadline_custody",
    "--test",
    "hardening_drain",
    "--test",
    "hardening_executor_protocol",
]


def fraction(value: Any, name: str) -> float:
    if type(value) not in (int, float) or not math.isfinite(value) or not 0 < value <= 1:
        raise host_perf.ReceiptError(f"{name} must be a finite fraction in (0,1]")
    return float(value)


def contract(data: bytes) -> dict:
    value = host_perf.parse_object(data, "frozen host contract")
    host_perf.exact_keys(value, CONTRACT_KEYS, "frozen host contract")
    if (
        type(value["schema_version"]) is not int
        or value["schema_version"] != VERSION
        or value["status"] != "frozen_taskmesh_full_host_series"
    ):
        raise host_perf.ReceiptError("host contract is not frozen")
    for name, length in (("source_head", 40), ("build_witness_sha256", 64)):
        item = value[name]
        if (
            not isinstance(item, str)
            or len(item) != length
            or any(c not in "0123456789abcdef" for c in item)
        ):
            raise host_perf.ReceiptError(f"invalid frozen {name}")
    rates = value["rate_grid"]
    if (
        not isinstance(rates, list)
        or len(rates) < 2
        or any(type(r) is not int or r <= 0 for r in rates)
        or rates != sorted(set(rates))
    ):
        raise host_perf.ReceiptError(
            "frozen rate grid requires at least two increasing absolute rates"
        )
    for key in ("scenario_sha256_by_rate", "control_policy_sha256_by_rate"):
        entries = value[key]
        if (
            not isinstance(entries, dict)
            or entries.keys() != {str(r) for r in rates}
            or any(
                not isinstance(d, str)
                or len(d) != 64
                or any(c not in "0123456789abcdef" for c in d)
                for d in entries.values()
            )
        ):
            raise host_perf.ReceiptError(f"frozen {key} differs from rate grid")
    for key in ("injection_ms", "max_span_ns", "min_success_samples_per_cohort"):
        if host_perf.nat(value[key], key) == 0:
            raise host_perf.ReceiptError(f"frozen {key} must be positive")
    host_perf.nat(value["warmup_ms"], "warmup_ms")
    repetitions = host_perf.nat(value["repetitions_per_rate"], "repetitions_per_rate")
    if repetitions < 4 or repetitions % 2:
        raise host_perf.ReceiptError("frozen series requires at least four balanced pairs per rate")
    for key in ("minimum_slo_fraction", "max_p99_relative_interval_width"):
        fraction(value[key], key)
    slos = value["slo_ms_by_class"]
    if (
        not isinstance(slos, dict)
        or not slos
        or any(
            not isinstance(k, str) or not k or host_perf.nat(v, "class SLO") == 0
            for k, v in slos.items()
        )
    ):
        raise host_perf.ReceiptError("frozen per-class SLO catalog missing")
    return value


def quantile_interval(values: list[int], p: float = 0.99) -> dict:
    """Exact binomial order-statistic ranks; dependence assumption is explicit.

    Within-run samples need exchangeability for nominal coverage. The admission
    additionally requires every independent run to pass; no request-level pooling.
    """
    n = len(values)
    if n == 0:
        raise host_perf.ReceiptError("empty quantile population")
    mode = min(n, int((n + 1) * p))
    mass = [0.0] * (n + 1)
    mass[mode] = 1.0
    for k in range(mode, 0, -1):
        mass[k - 1] = mass[k] * k / (n - k + 1) * (1 - p) / p
    for k in range(mode, n):
        mass[k + 1] = mass[k] * (n - k) / (k + 1) * p / (1 - p)
    total, cumulative, lower, upper = sum(mass), 0.0, None, None
    for k, probability in enumerate(mass):
        cumulative += probability / total
        if lower is None and cumulative >= 0.025:
            lower = k
        if cumulative >= 0.975:
            upper = k + 1
            break
    ordered = sorted(values)
    estimate = ordered[math.ceil(p * n) - 1]
    lo = ordered[lower - 1] if lower is not None and lower > 0 else None
    hi = ordered[upper - 1] if upper is not None and upper <= n else None
    width = (hi - lo) / estimate if lo is not None and hi is not None and estimate > 0 else None
    return {
        "count": n,
        "p99_ns": estimate,
        "rank_interval": [lower, upper],
        "order_statistic_95_interval_ns": [lo, hi],
        "relative_interval_width": width,
        "assumption": "exchangeable within-run success latencies; no pooled run samples",
    }


def admit(
    contract_bytes: bytes,
    study_root: Path,
    build_root: Path,
    controls_bytes: bytes,
    controls_root: Path,
    output: Path,
) -> dict:
    frozen = contract(contract_bytes)
    if host_perf._git("rev-parse", "HEAD") != frozen["source_head"] or host_perf._git(
        "status", "--porcelain=v1"
    ):
        raise host_perf.ReceiptError(
            "typed validation requires the exact clean frozen source checkout"
        )
    ledger = host_study.verify(study_root)
    if (
        ledger["contract_sha256"] != host_perf.sha256(contract_bytes)
        or ledger["failed"]
        or ledger["excluded"]
    ):
        raise host_perf.ReceiptError("study contract mismatch or failed/excluded attempts")
    witness_bytes = (build_root / "build-witness.json").read_bytes()
    if host_perf.sha256(witness_bytes) != frozen["build_witness_sha256"]:
        raise host_perf.ReceiptError("frozen build witness digest differs")
    controls = host_perf.parse_object(controls_bytes, "series controls")
    if controls.keys() != {str(rate) for rate in frozen["rate_grid"]}:
        raise host_perf.ReceiptError("controls do not cover frozen grid")
    # Validate retained proof cheaply first; the actual independent rebuild and
    # engine oracle happen after all raw populations have been reconstructed.
    witness = host_build.verify(build_root)
    if (
        witness["source_head"] != frozen["source_head"]
        or witness["example"] != "host_load_probe"
        or witness["features"] != []
    ):
        raise host_perf.ReceiptError(
            "build witness source/target differs from frozen full-host claim"
        )
    profile = host_build.verified_profile(build_root, witness)
    if profile["opt_level"] not in ("2", "3") or profile["debug_assertions"] or profile["test"]:
        raise host_perf.ReceiptError(
            "measured admission requires optimized non-test build without debug assertions"
        )
    runs, windows, identities, populations, assessments, resource_bounds = [], [], [], {}, {}, []
    control_inputs, control_identities, boots, cadences = {}, [], set(), set()
    for rate in frozen["rate_grid"]:
        role_paths = controls[str(rate)]
        if not isinstance(role_paths, dict) or role_paths.keys() != set(CONTROL_ROLES):
            raise host_perf.ReceiptError("control roles missing or extra")
        paths = {}
        for role, relative in role_paths.items():
            if not isinstance(relative, str) or not relative:
                raise host_perf.ReceiptError("control artifact path required")
            path = (controls_root / relative).resolve()
            if not path.is_relative_to(controls_root.resolve()):
                raise host_perf.ReceiptError("control path escapes manifest root")
            paths[role] = path
        policy_path = paths.pop("policy")
        policy = policy_path.read_bytes()
        if host_perf.sha256(policy) != frozen["control_policy_sha256_by_rate"][str(rate)]:
            raise host_perf.ReceiptError("control budget changed after preregistration")
        assessment = host_control_assess.assess(policy, **paths)
        if assessment["violations"]:
            raise host_perf.ReceiptError(f"measured control budget failed at rate {rate}")
        if (
            assessment["source_head"] != frozen["source_head"]
            or assessment["scenario_sha256"] != frozen["scenario_sha256_by_rate"][str(rate)]
        ):
            raise host_perf.ReceiptError("measured calibration source/scenario differs")
        bundle = host_perf.parse_object(paths["bundle_path"].read_bytes(), "bound controls")
        if (
            host_perf.sha256(paths["bundle_path"].read_bytes()) != assessment["bundle_sha256"]
            or bundle["host_binary_sha256"] != witness["binary_sha256"]
        ):
            raise host_perf.ReceiptError(
                "measured calibration executable/digest differs from frozen build"
            )
        assessments[str(rate)] = assessment
        control_inputs[str(rate)] = (policy, paths, policy_path)
        control_identities.append(bundle["identity"])
        boots.add(bundle["boot_time_ns"])
        cadences.add(bundle["resource_cadence_ms"])
        windows.extend(tuple(window) for window in assessment["process_windows_epoch_ns"])
    orders = {rate: [] for rate in frozen["rate_grid"]}
    last_series_end = 0
    for attempt in ledger["attempts"]:
        rate, arm = attempt["rate_per_second"], attempt["arm"]
        if rate not in orders:
            raise host_perf.ReceiptError("unplanned rate outside frozen grid")
        orders[rate].append(arm)
        directory = study_root / attempt["id"]
        scenario_bytes = (directory / "scenario").read_bytes()
        if host_perf.sha256(scenario_bytes) != frozen["scenario_sha256_by_rate"][str(rate)]:
            raise host_perf.ReceiptError("attempt scenario differs from frozen rate")
        scenario = host_perf.parse_object(scenario_bytes, "series scenario")
        if (
            scenario["load"]["warmup_ms"] != frozen["warmup_ms"]
            or scenario["load"]["injection_ms"] != frozen["injection_ms"]
            or len(scenario["offers"]) * 1000 != rate * frozen["injection_ms"]
        ):
            raise host_perf.ReceiptError("scenario window or arrival rate differs")
        if {c["name"]: c["slo_ms"] for c in scenario["classes"]} != frozen["slo_ms_by_class"]:
            raise host_perf.ReceiptError("scenario class SLO catalog differs")
        if any(
            o.get(k) is not None
            for o in scenario["offers"]
            for k in ("cancel_after_ms", "drop_after_ms", "deadline_ms")
        ):
            raise host_perf.ReceiptError(
                "capacity claim excludes intentional caller abort fixtures"
            )
        files = dict(host_study.ARTIFACTS)
        run = host_compare.read_run(directory, files, scenario_bytes, attempt["id"])
        identity = run["identity"]
        if (
            identity["source_head"] != frozen["source_head"]
            or run["binary_sha256"] != witness["binary_sha256"]
            or identity["features"] != witness["features"]
        ):
            raise host_perf.ReceiptError(
                "attempt source/executable/features differs from frozen build"
            )
        if any(identity != reference for reference in control_identities):
            raise host_perf.ReceiptError("series identity differs from measured calibration")
        boots.add(run["boot_time_ns"])
        cadences.add(run["resource_cadence_ms"])
        if identities and identity != identities[0]:
            raise host_perf.ReceiptError("series host/source identity changed")
        identities.append(identity)
        if run["window"][0] < last_series_end:
            raise host_perf.ReceiptError("series indexed process order is reversed or overlaps")
        last_series_end = run["window"][1]
        windows.append(run["window"])
        raw = host_perf.parse_object((directory / "raw").read_bytes(), "series raw")
        by_cohort = {}
        for row in raw["records"]:
            if row["disposition"].get("outcome", {}).get("kind") == "success":
                by_cohort.setdefault(f"{row['class']}/{row['path']}", []).append(
                    row["caller_response_ns"] - row["intended_ns"]
                )
        cohort_rows = {}
        for name, cohort in run["metrics"]["cohort_goodput"].items():
            values = by_cohort.get(name, [])
            interval = (
                quantile_interval(values)
                if values
                else {
                    "count": 0,
                    "p99_ns": None,
                    "rank_interval": [None, None],
                    "order_statistic_95_interval_ns": [None, None],
                    "relative_interval_width": None,
                    "assumption": "no observed success population",
                }
            )

            supported = (
                interval["count"] >= frozen["min_success_samples_per_cohort"]
                and interval["relative_interval_width"] is not None
                and interval["relative_interval_width"] <= frozen["max_p99_relative_interval_width"]
            )
            cohort_rows[name] = {
                "slo_fraction": cohort["slo_fraction"],
                "slo_per_sec": cohort["slo_per_sec"],
                "tail": interval,
                "passes": supported and cohort["slo_fraction"] >= frozen["minimum_slo_fraction"],
            }
        populations.setdefault(rate, []).append(cohort_rows)
        success_count = sum(len(values) for values in by_cohort.values())
        cpu_lower_bound = run["resource_observation"].get("sampled_cpu_delta_ns_lower_bound")
        resource_bounds.append(
            {
                "attempt": attempt["id"],
                **run["resource_observation"],
                "injection_cohort_successes": success_count,
                "whole_trial_cpu_ns_per_cohort_success_lower_bound": (
                    cpu_lower_bound / success_count
                    if cpu_lower_bound is not None and success_count
                    else None
                ),
                "population": (
                    "whole probe includes warmup/startup/settlement; "
                    "denominator is all successful injection-cohort completions"
                ),
            }
        )
        runs.append(
            {"attempt": attempt["id"], "sha256": run["artifact_sha256"], "cohorts": cohort_rows}
        )
    for rate, order in orders.items():
        pairs = [tuple(order[i : i + 2]) for i in range(0, len(order), 2)]
        if (
            len(pairs) != frozen["repetitions_per_rate"]
            or any(p not in (("baseline", "candidate"), ("candidate", "baseline")) for p in pairs)
            or pairs.count(("baseline", "candidate")) != pairs.count(("candidate", "baseline"))
        ):
            raise host_perf.ReceiptError(f"rate {rate}: missing or unbalanced planned pairs")
    if len(boots) != 1 or len(cadences) != 1:
        raise host_perf.ReceiptError("series/control boot or sampler cadence changed")
    windows.sort()
    if (
        not windows
        or any(a[1] > b[0] for a, b in zip(windows, windows[1:]))
        or windows[-1][1] - windows[0][0] > frozen["max_span_ns"]
    ):
        raise host_perf.ReceiptError("series/control windows overlap or exceed frozen span")
    points = [
        {
            "rate_per_second": rate,
            "passes_every_run_and_cohort": all(
                c["passes"] for row in populations[rate] for c in row.values()
            ),
        }
        for rate in frozen["rate_grid"]
    ]
    seen_failure = False
    for point in points:
        if seen_failure and point["passes_every_run_and_cohort"]:
            raise host_perf.ReceiptError("nonmonotonic grid requires investigation")
        seen_failure |= not point["passes_every_run_and_cohort"]
    # No result directory is reused; failed rebuild/oracle logs remain inspectable.
    output.mkdir(parents=True, exist_ok=False)
    host_build.verify(build_root, rebuild=True)
    env = dict(os.environ)
    env["CARGO_TARGET_DIR"] = str(build_root.resolve() / "target")
    oracle_profile = {
        "CARGO_PROFILE_TEST_OPT_LEVEL": profile["opt_level"],
        "CARGO_PROFILE_TEST_DEBUG_ASSERTIONS": str(profile["debug_assertions"]).lower(),
        "CARGO_PROFILE_TEST_OVERFLOW_CHECKS": str(profile["overflow_checks"]).lower(),
    }
    env.update(oracle_profile)
    process = subprocess.run(
        ORACLE, cwd=build_root / "source", env=env, capture_output=True, check=False
    )
    write_new(output / "oracle.stdout", process.stdout)
    write_new(output / "oracle.stderr", process.stderr)
    if process.returncode:
        raise host_perf.ReceiptError("exact frozen-source engine correctness oracle failed")
    oracle_results = re.findall(
        rb"test result: ok\. (\d+) passed; 0 failed; 0 ignored; 0 measured; 0 filtered out",
        process.stdout,
    )
    if len(oracle_results) != 4 or sum(int(count) for count in oracle_results) < 45:
        raise host_perf.ReceiptError("exact correctness oracle population incomplete")
    # Recheck mutable input custody after expensive rebuild/oracle execution.
    if (
        host_study.verify(study_root) != ledger
        or (build_root / "build-witness.json").read_bytes() != witness_bytes
    ):
        raise host_perf.ReceiptError("study/build proof changed during admission")
    host_build.verify(build_root)
    for rate, (policy, paths, policy_path) in control_inputs.items():
        if policy_path.read_bytes() != policy:
            raise host_perf.ReceiptError("control budget artifact changed during admission")
        if host_control_assess.assess(policy, **paths) != assessments[rate]:
            raise host_perf.ReceiptError("measured controls changed during admission")
    if host_perf._git("rev-parse", "HEAD") != frozen["source_head"] or host_perf._git(
        "status", "--porcelain=v1"
    ):
        raise host_perf.ReceiptError("source checkout changed during measured admission")
    passing = [p["rate_per_second"] for p in points if p["passes_every_run_and_cohort"]]
    report = {
        "schema_version": VERSION,
        "status": "MEASURED_SERIES_ADMITTED",
        "performance": "SCOPED_HOST_SERIES",
        "scope": (
            "same-source full-host repeated observations; "
            "no peer superiority or untested-rate claim"
        ),
        "contract_sha256": host_perf.sha256(contract_bytes),
        "ledger": ledger,
        "build_witness_sha256": host_perf.sha256(witness_bytes),
        "build_profile": profile,
        "build_environment": witness["build_environment"],
        "controls": assessments,
        "oracle": {
            "command": ORACLE,
            "profile_overrides": oracle_profile,
            "stdout_sha256": host_perf.sha256(process.stdout),
            "stderr_sha256": host_perf.sha256(process.stderr),
        },
        "highest_tested_passing_rate": max(passing) if passing else None,
        "knee_bracketed": bool(passing) and max(passing) < max(frozen["rate_grid"]),
        "points": points,
        "runs": runs,
        "resource_bounds": resource_bounds,
        "resource_efficiency": (
            "lower bound only: whole-trial sampled CPU per injection-cohort success; "
            "exact CPU and steady-state efficiency remain unobserved"
        ),
    }
    write_new(output / "admission.json", host_perf.canonical(report) + b"\n")
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("contract", type=Path)
    parser.add_argument("study", type=Path)
    parser.add_argument("build", type=Path)
    parser.add_argument("controls", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        report = admit(
            args.contract.read_bytes(),
            args.study,
            args.build,
            args.controls.read_bytes(),
            args.controls.parent,
            args.output,
        )
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"measured series rejected: {error}", file=sys.stderr)
        return 1
    print(
        f"{report['status']} highest_tested_rate={report['highest_tested_passing_rate']} "
        f"knee_bracketed={report['knee_bracketed']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
