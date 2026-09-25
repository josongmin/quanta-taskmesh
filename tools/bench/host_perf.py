#!/usr/bin/env python3
# ruff: noqa: UP045 -- pyproject supports Python 3.9, which lacks PEP 604 unions.
"""Structural receipt for the in-process Taskmesh host load probe.

This checker admits individual raw runs for later quiet-host analysis. A valid
receipt is not a performance regression verdict or an industry comparison.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path
from typing import Any, Optional

REPO = Path(__file__).resolve().parents[2]
IDENTITY_KEYS = {
    "source_head",
    "source_tree",
    "source_dirty",
    "lock_sha256",
    "rustc",
    "features",
    "topology_fingerprint",
    "host_fingerprint",
}
PROVENANCE_KEYS = {
    "schema_version",
    "status",
    "reason",
    "scenario_sha256",
    "raw_sha256",
    "binary_sha256",
    "build_command",
    "build_artifact_features",
    "start_identity",
    "end_identity",
    "runner_exit_code",
}
CALIBRATION_KEYS = {
    "generator_headroom_ok",
    "observer_distorted",
    "producer_lag_limit_ns",
}
OUTCOMES = (
    "success",
    "rejected",
    "deadline",
    "cancelled",
    "task_error",
    "governor_error",
    "caller_dropped",
    "not_submitted",
    "unanswered_at_settlement",
)
COUNT_KEYS = (
    "intended",
    "submitted",
    "not_submitted",
    "responded",
    "caller_dropped",
    "waiting",
    "unanswered_at_settlement",
)
RECORD_KEYS = {
    "id",
    "class",
    "path",
    "intended_ns",
    "scheduled_lag_ns",
    "submitted_ns",
    "body_started_ns",
    "caller_response_ns",
    "caller_drop_ns",
    "body_finished_ns",
    "disposition",
}
SAMPLE_KEYS = {
    "intended_ns",
    "observed_ns",
    "classes",
    "capabilities",
    "conservation_ok",
}
SAMPLED_CLASS_KEYS = {
    "inflight",
    "queued",
    "accepted",
    "running",
    "cpu_units_held",
    "memory_units_held",
}


class ReceiptError(ValueError):
    """A raw or summary artifact cannot support the requested claim."""


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical(data: Any) -> bytes:
    return json.dumps(data, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()


def parse_object(data: bytes, name: str) -> dict[str, Any]:
    def unique(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        value: dict[str, Any] = {}
        for key, item in pairs:
            if key in value:
                raise ReceiptError(f"{name}: duplicate JSON key {key!r}")
            value[key] = item
        return value

    def reject_constant(value: str) -> None:
        raise ReceiptError(f"{name}: nonfinite JSON value {value}")

    try:
        value = json.loads(data, object_pairs_hook=unique, parse_constant=reject_constant)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ReceiptError(f"{name}: invalid JSON: {error}") from error
    if not isinstance(value, dict):
        raise ReceiptError(f"{name}: root must be an object")
    return value


def validate_with_rust(scenario_bytes: bytes, raw_bytes: bytes, features: list[str]) -> None:
    """Use the runner's typed scenario, raw ledger and Builder rules."""
    try:
        with tempfile.TemporaryDirectory(prefix="taskmesh-host-receipt-") as temp_dir:
            raw_path = Path(temp_dir) / "raw.json"
            raw_path.write_bytes(raw_bytes)
            command = ["cargo", "run", "--locked", "--quiet", "-p", "taskmesh-bench"]
            if features:
                command.extend(["--features", ",".join(features)])
            command.extend(["--example", "host_scenario_validate", "--", "--raw", str(raw_path)])
            result = subprocess.run(
                command,
                cwd=REPO,
                input=scenario_bytes,
                capture_output=True,
                check=False,
            )
    except OSError as error:
        raise ReceiptError(f"Rust scenario preflight unavailable: {error}") from error
    if result.returncode != 0 or not result.stdout.startswith(b"SCENARIO_VALID id="):
        detail = result.stderr.decode("utf-8", errors="replace").strip()
        raise ReceiptError(f"Rust scenario preflight failed: {detail[:1000]}")


def exact_keys(value: dict[str, Any], keys: set[str], name: str) -> None:
    if set(value) != keys:
        raise ReceiptError(f"{name}: missing/unknown fields: {sorted(keys ^ set(value))}")


def nat(value: Any, name: str) -> int:
    if type(value) is not int or value < 0:
        raise ReceiptError(f"{name}: expected nonnegative integer")
    return value


def decimal_nat(value: Any, name: str) -> int:
    if (
        not isinstance(value, str)
        or not value.isascii()
        or not value.isdecimal()
        or (len(value) > 1 and value.startswith("0"))
    ):
        raise ReceiptError(f"{name}: expected canonical decimal string")
    return int(value)


def _git(*args: str) -> str:
    result = subprocess.run(["git", *args], cwd=REPO, capture_output=True, text=True, check=True)
    return result.stdout.strip()


def local_identity(scenario: dict[str, Any], features: list[str]) -> dict[str, Any]:
    topology = scenario.get("topology")
    if not isinstance(topology, dict):
        raise ReceiptError("scenario: topology missing")
    rustc = subprocess.run(
        ["rustc", "--version"], capture_output=True, text=True, check=True
    ).stdout.strip()
    return {
        "source_head": _git("rev-parse", "HEAD"),
        "source_tree": _git("rev-parse", "HEAD^{tree}"),
        "source_dirty": bool(_git("status", "--porcelain=v1")),
        "lock_sha256": sha256((REPO / "Cargo.lock").read_bytes()),
        "rustc": rustc,
        "features": sorted(features),
        "topology_fingerprint": sha256(canonical(topology)),
        "host_fingerprint": sha256(
            canonical(
                {
                    "system": platform.system(),
                    "release": platform.release(),
                    "machine": platform.machine(),
                    "processor": platform.processor(),
                    "cpu_count": os.cpu_count(),
                    "node_sha256": sha256(platform.node().encode()),
                }
            )
        ),
    }


def validate_identity(identity: Any) -> None:
    if not isinstance(identity, dict):
        raise ReceiptError("identity must be an object")
    exact_keys(identity, IDENTITY_KEYS, "identity")
    if type(identity["source_dirty"]) is not bool:
        raise ReceiptError("identity.source_dirty must be boolean")
    if (
        not isinstance(identity["features"], list)
        or not all(isinstance(feature, str) and feature for feature in identity["features"])
        or identity["features"] != sorted(set(identity["features"]))
    ):
        raise ReceiptError("identity.features must be a sorted unique string list")
    for key in IDENTITY_KEYS - {"source_dirty", "features"}:
        if not isinstance(identity[key], str) or not identity[key].strip():
            raise ReceiptError(f"identity.{key} must be nonempty")


def validate_execution_provenance(
    data: bytes, raw_bytes: bytes, scenario_bytes: bytes, identity: dict[str, Any]
) -> None:
    provenance = parse_object(data, "execution provenance")
    exact_keys(provenance, PROVENANCE_KEYS, "execution provenance")
    if provenance["schema_version"] != 1 or provenance["status"] != "complete":
        raise ReceiptError("execution provenance is invalid or incomplete")
    if provenance["reason"] is not None or provenance["runner_exit_code"] != 0:
        raise ReceiptError("execution provenance contains a failed runner")
    if provenance["scenario_sha256"] != sha256(scenario_bytes) or provenance[
        "raw_sha256"
    ] != sha256(raw_bytes):
        raise ReceiptError("execution provenance artifact digest mismatch")
    binary_digest = provenance["binary_sha256"]
    if (
        not isinstance(binary_digest, str)
        or len(binary_digest) != 64
        or any(char not in "0123456789abcdef" for char in binary_digest)
    ):
        raise ReceiptError("execution provenance binary digest is invalid")
    command = provenance["build_command"]
    if (
        not isinstance(command, list)
        or not all(isinstance(part, str) and part for part in command)
        or command[:4] != ["cargo", "build", "--locked", "-p"]
        or "taskmesh-bench" not in command
        or "--example" not in command
        or "host_load_probe" not in command
    ):
        raise ReceiptError("execution provenance build command is invalid")
    requested_features = identity["features"]
    if requested_features:
        if command[-2:] != ["--features", ",".join(requested_features)]:
            raise ReceiptError("execution provenance feature build does not match identity")
    elif "--features" in command:
        raise ReceiptError("execution provenance declares unexpected build features")
    artifact_features = provenance["build_artifact_features"]
    if (
        not isinstance(artifact_features, list)
        or not all(isinstance(feature, str) and feature for feature in artifact_features)
        or artifact_features != sorted(set(artifact_features))
    ):
        raise ReceiptError("execution provenance artifact features are invalid")
    for key in ("start_identity", "end_identity"):
        validate_identity(provenance[key])
        if provenance[key] != identity:
            raise ReceiptError("execution provenance source or host changed during run")


def analyze_raw(raw: dict[str, Any], scenario: dict[str, Any]) -> dict[str, Any]:
    if raw.get("schema_version") != 2 or scenario.get("schema_version") != 1:
        raise ReceiptError("unsupported raw/scenario version")
    if raw.get("status") != {"kind": "complete"}:
        raise ReceiptError("raw host run is invalid or missing complete status")
    offers = scenario.get("offers")
    records = raw.get("records")
    classes = scenario.get("classes")
    load = scenario.get("load")
    if not all(isinstance(value, list) for value in (offers, records, classes)) or not isinstance(
        load, dict
    ):
        raise ReceiptError("scenario/raw population missing")
    if raw.get("scenario_id") != scenario.get("id") or len(records) != len(offers) or not offers:
        raise ReceiptError("scenario identity or intended row population differs")
    cut_ns = nat(raw.get("injection_window_ns"), "injection_window_ns")
    if cut_ns == 0 or cut_ns != nat(load.get("injection_ms"), "injection_ms") * 1_000_000:
        raise ReceiptError("injection window mismatch")
    interval_ns = nat(raw.get("interval_ns"), "interval_ns")
    if (
        interval_ns == 0
        or interval_ns != nat(load.get("interval_ms"), "interval_ms") * 1_000_000
        or cut_ns % interval_ns
        or cut_ns // interval_ns > 10_000
    ):
        raise ReceiptError("invalid report interval")
    intervals = [
        {
            "start_ns": start,
            "end_ns": start + interval_ns,
            "intended": 0,
            "submitted": 0,
            "not_submitted": 0,
            "started": 0,
            "responded": 0,
            "success": 0,
            "rejected": 0,
            "caller_dropped": 0,
        }
        for start in range(0, cut_ns, interval_ns)
    ]

    def bucket(time_ns: int) -> Optional[dict[str, int]]:
        return intervals[time_ns // interval_ns] if time_ns < cut_ns else None

    names = {item.get("name") for item in classes if isinstance(item, dict)}
    if len(names) != len(classes) or any(not isinstance(name, str) for name in names):
        raise ReceiptError("invalid class catalog")
    settlement_ms = nat(load.get("settlement_ms"), "settlement_ms")
    class_slo: dict[str, int] = {}
    for item in classes:
        slo = nat(item.get("slo_ms"), f"class {item['name']}.slo_ms")
        if slo == 0 or slo > settlement_ms:
            raise ReceiptError(f"class {item['name']}: SLO exceeds settlement")
        class_slo[item["name"]] = slo * 1_000_000
    snapshot_cadence_ns = nat(raw.get("snapshot_cadence_ns"), "snapshot_cadence_ns")
    if snapshot_cadence_ns != nat(load.get("snapshot_ms"), "snapshot_ms") * 1_000_000:
        raise ReceiptError("snapshot cadence differs from scenario")
    samples = raw.get("snapshots")
    if (
        not isinstance(samples, list)
        or (snapshot_cadence_ns == 0 and samples)
        or (
            snapshot_cadence_ns > 0
            and (cut_ns % snapshot_cadence_ns or len(samples) != cut_ns // snapshot_cadence_ns)
        )
    ):
        raise ReceiptError("snapshot sample population differs from cadence")
    sampled_peaks: dict[str, dict[str, int]] = {
        name: {"inflight": 0, "queued": 0, "accepted": 0, "running": 0} for name in names
    }
    capability_peaks: dict[str, int] = {}
    late_samples = 0
    max_snapshot_lag_ns = 0
    for index, sample in enumerate(samples):
        if not isinstance(sample, dict):
            raise ReceiptError(f"snapshot {index}: object required")
        exact_keys(sample, SAMPLE_KEYS, f"snapshot {index}")
        intended = nat(sample["intended_ns"], f"snapshot {index}.intended_ns")
        observed = nat(sample["observed_ns"], f"snapshot {index}.observed_ns")
        if intended != index * snapshot_cadence_ns or observed < intended:
            raise ReceiptError(f"snapshot {index}: invalid sample time")
        if sample["conservation_ok"] is not True:
            raise ReceiptError(f"snapshot {index}: conservation violation")
        lag = observed - intended
        max_snapshot_lag_ns = max(max_snapshot_lag_ns, lag)
        if observed >= cut_ns:
            late_samples += 1
        class_gauges = sample["classes"]
        if not isinstance(class_gauges, dict) or set(class_gauges) != names:
            raise ReceiptError(f"snapshot {index}: class catalog mismatch")
        for name, gauges in class_gauges.items():
            if not isinstance(gauges, dict):
                raise ReceiptError(f"snapshot {index}: class gauges missing")
            exact_keys(gauges, SAMPLED_CLASS_KEYS, f"snapshot {index}.{name}")
            for field in SAMPLED_CLASS_KEYS:
                if field in {"cpu_units_held", "memory_units_held"}:
                    value = decimal_nat(gauges[field], f"snapshot {index}.{name}.{field}")
                else:
                    value = nat(gauges[field], f"snapshot {index}.{name}.{field}")
                if observed < cut_ns and field in sampled_peaks[name]:
                    sampled_peaks[name][field] = max(sampled_peaks[name][field], value)
            if gauges["accepted"] + gauges["running"] > gauges["inflight"]:
                raise ReceiptError(f"snapshot {index}.{name}: impossible phase gauges")
        capabilities = sample["capabilities"]
        if not isinstance(capabilities, dict) or any(
            not isinstance(pool, str) for pool in capabilities
        ):
            raise ReceiptError(f"snapshot {index}: invalid capability catalog")
        for pool, in_use in capabilities.items():
            nat(in_use, f"snapshot {index}.{pool}.in_use")
            if observed < cut_ns:
                capability_peaks[pool] = max(capability_peaks.get(pool, 0), in_use)
    totals = Counter({key: 0 for key in COUNT_KEYS})
    cut = Counter({key: 0 for key in COUNT_KEYS})
    totals["intended"] = len(offers)
    cut["intended"] = len(offers)
    outcomes: dict[str, Counter[str]] = {}
    intended_by_path: Counter[str] = Counter()
    success_by_path: Counter[str] = Counter()
    slo_by_path: Counter[str] = Counter()
    latency_by_path: dict[str, list[int]] = {}
    success_latency: list[int] = []
    starts: Counter[str] = Counter()
    last_intended = -1
    for index, (offer, row) in enumerate(zip(offers, records)):
        if not isinstance(offer, dict) or not isinstance(row, dict):
            raise ReceiptError(f"row {index}: object required")
        exact_keys(row, RECORD_KEYS, f"row {index}")
        intended = nat(offer.get("send_time_ns"), f"offer {index}.send_time_ns")
        if intended < last_intended or intended >= cut_ns:
            raise ReceiptError(f"offer {index}: invalid intended schedule")
        last_intended = intended
        bucket(intended)["intended"] += 1
        if row.get("id") != index or any(
            row.get(key) != offer.get(source)
            for key, source in (
                ("class", "class"),
                ("path", "path"),
                ("intended_ns", "send_time_ns"),
            )
        ):
            raise ReceiptError(f"row {index}: missing, duplicate, reordered, or misattributed")
        class_name, path = row["class"], row["path"]
        if class_name not in names or path not in {"io", "blocking", "cpu"}:
            raise ReceiptError(f"row {index}: unknown class/path")
        population = f"{class_name}/{path}"
        intended_by_path[population] += 1
        lag = nat(row.get("scheduled_lag_ns"), f"row {index}.scheduled_lag_ns")
        submit = row.get("submitted_ns")
        start = row.get("body_started_ns")
        finish = row.get("body_finished_ns")
        response = row.get("caller_response_ns")
        drop = row.get("caller_drop_ns")
        disposition = row.get("disposition")
        if not isinstance(disposition, dict) or not isinstance(disposition.get("kind"), str):
            raise ReceiptError(f"row {index}: missing disposition")
        kind = disposition["kind"]
        exact_keys(
            disposition,
            {"kind", "outcome"} if kind == "responded" else {"kind"},
            f"row {index}.disposition",
        )
        if kind not in {"not_submitted", "responded", "caller_dropped", "unanswered_at_settlement"}:
            raise ReceiptError(f"row {index}: invalid settlement disposition {kind}")
        if kind == "not_submitted":
            if any(value is not None for value in (submit, start, finish, response, drop)):
                raise ReceiptError(f"row {index}: not-submitted offer has activity")
            totals["not_submitted"] += 1
            cut["not_submitted"] += 1
            bucket(intended)["not_submitted"] += 1
            outcome = kind
        else:
            submit = nat(submit, f"row {index}.submitted_ns")
            if submit < intended or submit >= cut_ns or lag > submit - intended:
                raise ReceiptError(f"row {index}: invalid submit/producer time")
            totals["submitted"] += 1
            cut["submitted"] += 1
            if (interval := bucket(submit)) is not None:
                interval["submitted"] += 1
            if start is not None:
                start = nat(start, f"row {index}.body_started_ns")
                if start < submit:
                    raise ReceiptError(f"row {index}: body starts before submission")
                starts[class_name] += 1
                if (interval := bucket(start)) is not None:
                    interval["started"] += 1
            if finish is not None:
                finish = nat(finish, f"row {index}.body_finished_ns")
                if start is None or finish < start:
                    raise ReceiptError(f"row {index}: invalid body finish")
            if kind == "responded":
                response = nat(response, f"row {index}.caller_response_ns")
                if response < submit or drop is not None:
                    raise ReceiptError(f"row {index}: invalid response timestamps")
                terminal = disposition.get("outcome")
                if not isinstance(terminal, dict) or terminal.get("kind") not in {
                    "success",
                    "rejected",
                    "deadline",
                    "cancelled",
                    "task_error",
                    "governor_error",
                }:
                    raise ReceiptError(f"row {index}: untyped response outcome")
                outcome = terminal["kind"]
                exact_keys(
                    terminal,
                    {"kind", "verdict"}
                    if outcome == "rejected"
                    else {"kind", "error"}
                    if outcome == "governor_error"
                    else {"kind"},
                    f"row {index}.outcome",
                )
                if outcome == "rejected" and not isinstance(terminal.get("verdict"), (dict, str)):
                    raise ReceiptError(f"row {index}: rejection lacks typed verdict")
                if outcome == "rejected" and start is not None:
                    raise ReceiptError(f"row {index}: rejected work started")
                if outcome == "success":
                    if finish is None or finish > response:
                        raise ReceiptError(f"row {index}: success lacks prior body finish")
                    success_latency.append(response - intended)
                    latency_by_path.setdefault(population, []).append(response - intended)
                    success_by_path[population] += 1
                    if response - intended <= class_slo[class_name]:
                        slo_by_path[population] += 1
                totals["responded"] += 1
                cut["responded" if response < cut_ns else "waiting"] += 1
                if (interval := bucket(response)) is not None:
                    interval["responded"] += 1
                    if outcome in {"success", "rejected"}:
                        interval[outcome] += 1
            elif kind == "caller_dropped":
                drop = nat(drop, f"row {index}.caller_drop_ns")
                if drop < submit or response is not None:
                    raise ReceiptError(f"row {index}: invalid caller drop")
                outcome = kind
                totals["caller_dropped"] += 1
                cut["caller_dropped" if drop < cut_ns else "waiting"] += 1
                if (interval := bucket(drop)) is not None:
                    interval["caller_dropped"] += 1
            else:
                if response is not None or drop is not None:
                    raise ReceiptError(f"row {index}: unanswered has a terminal")
                outcome = kind
                totals["unanswered_at_settlement"] += 1
                cut["waiting"] += 1
        outcomes.setdefault(population, Counter({name: 0 for name in OUTCOMES}))[outcome] += 1
    if totals["intended"] != totals["submitted"] + totals["not_submitted"] or totals[
        "submitted"
    ] != (totals["responded"] + totals["caller_dropped"] + totals["unanswered_at_settlement"]):
        raise ReceiptError("settlement conservation failed")
    for label, expected in (("cut", cut), ("settlement", totals)):
        found = raw.get(label)
        keys = set(COUNT_KEYS)
        if (
            not isinstance(found, dict)
            or set(found) != keys
            or any(nat(found[key], f"{label}.{key}") != expected[key] for key in keys)
        ):
            raise ReceiptError(f"{label} counts differ from raw rows")
    counters = raw.get("class_counters")
    if not isinstance(counters, dict) or set(counters) != names:
        raise ReceiptError("class counter catalog mismatch")
    for name, item in counters.items():
        if not isinstance(item, dict):
            raise ReceiptError(f"class {name}: counters missing")
        admitted = nat(item.get("admitted"), f"{name}.admitted")
        started = nat(item.get("started"), f"{name}.started")
        terminated = nat(item.get("terminated"), f"{name}.terminated")
        if started != starts[name] or not (started <= admitted == terminated):
            raise ReceiptError(f"class {name}: counter/row mismatch")
        if nat(item.get("inflight"), f"{name}.inflight") or nat(
            item.get("queued"), f"{name}.queued"
        ):
            raise ReceiptError(f"class {name}: residual ownership")
    final_capabilities = raw.get("final_capabilities")
    if not isinstance(final_capabilities, dict) or not final_capabilities:
        raise ReceiptError("final capability inventory missing")
    for pool, in_use in final_capabilities.items():
        if not isinstance(pool, str) or not pool or nat(in_use, f"{pool}.final_in_use"):
            raise ReceiptError(f"capability {pool}: residual ownership")
    if raw.get("drain_ok") is not True or raw.get("conservation_ok") is not True:
        raise ReceiptError("drain or final conservation failed")

    def nearest(values: list[int], quantile: float) -> Optional[int]:
        return sorted(values)[math.ceil(quantile * len(values)) - 1] if values else None

    return {
        "counts": dict(sorted(totals.items())),
        "by_class_path": {
            key: dict(sorted(value.items())) for key, value in sorted(outcomes.items())
        },
        "cohort_goodput": {
            key: {
                "intended": intended_by_path[key],
                "success": success_by_path[key],
                "slo_success": slo_by_path[key],
                "slo_fraction": slo_by_path[key] / intended_by_path[key],
                "success_per_sec": success_by_path[key] * 1_000_000_000 / cut_ns,
                "slo_per_sec": slo_by_path[key] * 1_000_000_000 / cut_ns,
                "slo_ns": class_slo[key.split("/", 1)[0]],
            }
            for key in sorted(intended_by_path)
        },
        "success_latency_count": len(success_latency),
        "success_latency_p99_ns": nearest(success_latency, 0.99),
        "success_latency_by_class_path": {
            key: {
                "count": len(latency_by_path.get(key, [])),
                "p50_ns": nearest(latency_by_path.get(key, []), 0.50),
                "p95_ns": nearest(latency_by_path.get(key, []), 0.95),
                "p99_ns": nearest(latency_by_path.get(key, []), 0.99),
            }
            for key in sorted(intended_by_path)
        },
        "max_producer_lag_ns": max(row["scheduled_lag_ns"] for row in records),
        "intervals": intervals,
        "sampled_peaks_lower_bound": sampled_peaks,
        "sampled_capability_peaks_lower_bound": capability_peaks,
        "snapshot_sample_count": len(samples),
        "late_snapshot_samples": late_samples,
        "max_snapshot_lag_ns": max_snapshot_lag_ns,
    }


def make_summary(
    raw_bytes: bytes,
    scenario_bytes: bytes,
    identity: dict[str, Any],
    calibration: dict[str, Any],
    p99_min_samples: int,
    provenance_bytes: Optional[bytes] = None,
) -> dict[str, Any]:
    validate_identity(identity)
    validate_with_rust(scenario_bytes, raw_bytes, identity["features"])
    if provenance_bytes is not None:
        validate_execution_provenance(provenance_bytes, raw_bytes, scenario_bytes, identity)
    if not isinstance(calibration, dict):
        raise ReceiptError("calibration must be an object")
    exact_keys(calibration, CALIBRATION_KEYS, "calibration")
    if (
        type(calibration["generator_headroom_ok"]) is not bool
        or type(calibration["observer_distorted"]) is not bool
    ):
        raise ReceiptError("calibration booleans are required")
    nat(calibration["producer_lag_limit_ns"], "producer_lag_limit_ns")
    if nat(p99_min_samples, "p99_min_samples") == 0:
        raise ReceiptError("p99_min_samples must be positive")
    raw = parse_object(raw_bytes, "raw")
    scenario = parse_object(scenario_bytes, "scenario")
    metrics = analyze_raw(raw, scenario)
    metrics["success_latency_quantile_method"] = "nearest_rank"
    if metrics["success_latency_count"] < p99_min_samples:
        metrics["success_latency_p99_ns"] = None
        metrics["success_latency_p99_status"] = "insufficient_population"
    else:
        metrics["success_latency_p99_status"] = "available"
    for population in metrics["success_latency_by_class_path"].values():
        if population["count"] < p99_min_samples:
            population["p99_ns"] = None
            population["p99_status"] = "insufficient_population"
        else:
            population["p99_status"] = "available"
    return {
        "schema_version": 2,
        "raw_sha256": sha256(raw_bytes),
        "scenario_sha256": sha256(scenario_bytes),
        "execution_provenance_sha256": sha256(provenance_bytes) if provenance_bytes else None,
        "identity": identity,
        "calibration": calibration,
        "p99_min_samples": p99_min_samples,
        "metrics": metrics,
    }


def verify_receipt(
    raw_bytes: bytes,
    scenario_bytes: bytes,
    summary_bytes: bytes,
    expected_identity: Optional[dict[str, Any]] = None,
    require_performance: bool = False,
    provenance_bytes: Optional[bytes] = None,
) -> dict[str, Any]:
    summary = parse_object(summary_bytes, "summary")
    exact_keys(
        summary,
        {
            "schema_version",
            "raw_sha256",
            "scenario_sha256",
            "execution_provenance_sha256",
            "identity",
            "calibration",
            "p99_min_samples",
            "metrics",
        },
        "summary",
    )
    expected = make_summary(
        raw_bytes,
        scenario_bytes,
        summary["identity"],
        summary["calibration"],
        summary["p99_min_samples"],
        provenance_bytes,
    )
    if summary != expected:
        raise ReceiptError("summary differs from raw/scenario reconstruction or digest")
    if expected_identity is not None and summary["identity"] != expected_identity:
        raise ReceiptError("source/features/topology/host fingerprint mismatch")
    if require_performance:
        metrics = summary["metrics"]
        calibration = summary["calibration"]
        if summary["identity"]["source_dirty"]:
            raise ReceiptError("dirty source is diagnostic only")
        if metrics["counts"].get("not_submitted", 0) or metrics["counts"].get(
            "unanswered_at_settlement", 0
        ):
            raise ReceiptError("generator-limited or unanswered run")
        if not calibration["generator_headroom_ok"] or calibration["observer_distorted"]:
            raise ReceiptError("generator/observer calibration failed")
        if metrics["max_producer_lag_ns"] > calibration["producer_lag_limit_ns"]:
            raise ReceiptError("producer lag exceeds calibrated limit")
        if metrics["late_snapshot_samples"]:
            raise ReceiptError("snapshot observer missed injection window")
        if any(
            population["count"] < summary["p99_min_samples"]
            for population in metrics["success_latency_by_class_path"].values()
        ):
            raise ReceiptError("unsupported per-class/path p99 population")
        if provenance_bytes is None:
            raise ReceiptError("execution provenance is required for performance")
        # The current booleans are caller assertions, not measured null-work,
        # A/A and observer-on/off receipts. Fail closed until those artifacts
        # have a versioned contract and independent verification.
        raise ReceiptError("measured calibration artifact is required for performance")
    return summary


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("create", "verify"))
    parser.add_argument("raw", type=Path)
    parser.add_argument("scenario", type=Path)
    parser.add_argument("summary", type=Path)
    parser.add_argument("--feature", action="append", default=[])
    parser.add_argument("--calibration", type=Path)
    parser.add_argument("--provenance", type=Path)
    parser.add_argument("--p99-min-samples", type=int, default=10_000)
    parser.add_argument("--require-performance", action="store_true")
    args = parser.parse_args()
    try:
        raw_bytes = args.raw.read_bytes()
        scenario_bytes = args.scenario.read_bytes()
        scenario = parse_object(scenario_bytes, "scenario")
        identity = local_identity(scenario, args.feature)
        provenance_bytes = args.provenance.read_bytes() if args.provenance else None
        if args.mode == "create":
            if args.summary.exists():
                raise ReceiptError("refusing to overwrite summary")
            if args.calibration is None:
                raise ReceiptError("create requires --calibration")
            calibration = parse_object(args.calibration.read_bytes(), "calibration")
            summary = make_summary(
                raw_bytes,
                scenario_bytes,
                identity,
                calibration,
                args.p99_min_samples,
                provenance_bytes,
            )
            args.summary.write_bytes(json.dumps(summary, indent=2, sort_keys=True).encode() + b"\n")
            print(f"STRUCTURALLY_VALID summary={args.summary} performance=UNQUALIFIED")
        else:
            summary = verify_receipt(
                raw_bytes,
                scenario_bytes,
                args.summary.read_bytes(),
                identity,
                require_performance=args.require_performance,
                provenance_bytes=provenance_bytes,
            )
            print(f"STRUCTURALLY_VALID performance=UNQUALIFIED raw_sha256={summary['raw_sha256']}")
    except (OSError, subprocess.CalledProcessError, ReceiptError) as error:
        print(f"host receipt rejected: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
