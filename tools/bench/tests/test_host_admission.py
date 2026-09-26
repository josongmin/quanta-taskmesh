"""Admission orchestration tests use synthetic measurements, never performance proof."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_admission as admission  # noqa: E402
import host_perf  # noqa: E402


def test_quantile_rank_interval_does_not_invent_a_thin_tail() -> None:
    thin = admission.quantile_interval([10] * 10)
    assert thin["order_statistic_95_interval_ns"][1] is None
    assert thin["relative_interval_width"] is None
    thick = admission.quantile_interval([10] * 1000)
    assert thick["order_statistic_95_interval_ns"] == [10, 10]
    assert thick["relative_interval_width"] == 0
    with pytest.raises(host_perf.ReceiptError, match="empty quantile"):
        admission.quantile_interval([])


def fixture(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> tuple:
    build, study, controls = (tmp_path / name for name in ("build", "study", "controls"))
    for root in (build, study, controls):
        root.mkdir()
    monkeypatch.setattr(host_perf, "_git", lambda *args: "a" * 40 if args[0] == "rev-parse" else "")
    witness_bytes = b'{"test": "synthetic witness"}'
    (build / "build-witness.json").write_bytes(witness_bytes)
    identity = {"source_head": "a" * 40, "source_dirty": False, "features": []}
    witness = {
        "source_head": "a" * 40,
        "example": "host_load_probe",
        "binary_sha256": "b" * 64,
        "features": [],
    }
    calls = []

    def verify(_root: Path, *, rebuild: bool = False) -> dict:
        calls.append(rebuild)
        return witness

    monkeypatch.setattr(admission.host_build, "verify", verify)
    policies, scenarios, control_manifest, assessments, attempts = {}, {}, {}, {}, []
    fake_runs = {}
    for rate_index, rate in enumerate((1000, 2000)):
        key = str(rate)
        scenario = {
            "load": {"warmup_ms": 0, "injection_ms": 1000},
            "classes": [{"name": "c", "slo_ms": 10}],
            "offers": [{"path": "io"}] * rate,
        }
        scenario_bytes = host_perf.canonical(scenario)
        scenarios[key] = host_perf.sha256(scenario_bytes)
        point = controls / key
        point.mkdir()
        policy = b"policy"
        (point / "policy").write_bytes(policy)
        policies[key] = host_perf.sha256(policy)
        bundle = {
            "host_binary_sha256": "b" * 64,
            "identity": identity,
            "boot_time_ns": 1,
            "resource_cadence_ms": 250,
        }
        bundle_bytes = host_perf.canonical(bundle)
        (point / "bundle_path").write_bytes(bundle_bytes)
        control_manifest[key] = {role: f"{key}/{role}" for role in admission.CONTROL_ROLES}
        assessments[key] = {
            "violations": [],
            "source_head": "a" * 40,
            "scenario_sha256": scenarios[key],
            "bundle_sha256": host_perf.sha256(bundle_bytes),
            "process_windows_epoch_ns": [(10 + rate_index * 10, 15 + rate_index * 10)],
        }
        for pair in range(4):
            order = ("baseline", "candidate") if pair % 2 == 0 else ("candidate", "baseline")
            for arm in order:
                name = f"r{rate}-p{pair}-{arm}"
                directory = study / name
                directory.mkdir()
                (directory / "scenario").write_bytes(scenario_bytes)
                (directory / "raw").write_bytes(
                    host_perf.canonical(
                        {
                            "records": [
                                {
                                    "class": "c",
                                    "path": "io",
                                    "intended_ns": 0,
                                    "caller_response_ns": 10,
                                    "disposition": {"outcome": {"kind": "success"}},
                                }
                            ]
                            * 1000
                        }
                    )
                )
                attempts.append(
                    {"id": name, "rate_per_second": rate, "pair_index": pair, "arm": arm}
                )
                start = 100 + len(attempts) * 10
                fake_runs[name] = {
                    "identity": identity,
                    "binary_sha256": "b" * 64,
                    "boot_time_ns": 1,
                    "resource_cadence_ms": 250,
                    "window": (start, start + 5),
                    "artifact_sha256": {"raw": name},
                    "resource_observation": {},
                    "metrics": {
                        "cohort_goodput": {
                            "c/io": {
                                "slo_fraction": 1.0 if rate == 1000 else 0.5,
                                "slo_per_sec": 1000,
                            }
                        }
                    },
                }
    contract = {
        "schema_version": 1,
        "status": "frozen_taskmesh_full_host_series",
        "source_head": "a" * 40,
        "build_witness_sha256": host_perf.sha256(witness_bytes),
        "rate_grid": [1000, 2000],
        "scenario_sha256_by_rate": scenarios,
        "control_policy_sha256_by_rate": policies,
        "warmup_ms": 0,
        "injection_ms": 1000,
        "repetitions_per_rate": 4,
        "minimum_slo_fraction": 0.99,
        "slo_ms_by_class": {"c": 10},
        "min_success_samples_per_cohort": 1000,
        "max_p99_relative_interval_width": 0.1,
        "max_span_ns": 1000,
    }
    contract_bytes = host_perf.canonical(contract)
    ledger = {
        "contract_sha256": host_perf.sha256(contract_bytes),
        "failed": [],
        "excluded": [],
        "attempts": attempts,
    }
    monkeypatch.setattr(admission.host_study, "verify", lambda _root: ledger)
    monkeypatch.setattr(
        admission.host_control_assess,
        "assess",
        lambda _policy, **paths: assessments[paths["bundle_path"].parent.name],
    )
    monkeypatch.setattr(
        admission.host_compare, "read_run", lambda _root, _files, _scenario, label: fake_runs[label]
    )
    monkeypatch.setattr(
        admission.subprocess,
        "run",
        lambda *a, **k: subprocess.CompletedProcess(
            [],
            0,
            b"test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
            b"test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
            b"test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
            b"test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n",
            b"",
        ),
    )
    return (
        contract_bytes,
        study,
        build,
        host_perf.canonical(control_manifest),
        controls,
        tmp_path / "output",
        calls,
        ledger,
        assessments,
        fake_runs,
    )


def test_admission_reexecutes_rebuild_oracle_and_reports_only_highest_tested_rate(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    args = fixture(tmp_path, monkeypatch)
    report = admission.admit(*args[:6])
    assert report["highest_tested_passing_rate"] == 1000
    assert report["knee_bracketed"] is True
    assert report["performance"] == "SCOPED_HOST_SERIES"
    assert args[6].count(True) == 1
    assert (args[5] / "oracle.stdout").is_file()
    assert (args[5] / "admission.json").is_file()


@pytest.mark.parametrize(
    "change,error",
    [
        ("failed", "failed/excluded"),
        ("control", "budget failed"),
        ("binary", "executable/features"),
        ("overlap", "windows overlap"),
        ("unbalanced", "unbalanced"),
        ("oracle", "correctness oracle failed"),
        ("empty_oracle", "oracle population"),
    ],
)
def test_admission_rejects_incomplete_or_incompatible_evidence(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, change: str, error: str
) -> None:
    args = fixture(tmp_path, monkeypatch)
    if change == "failed":
        args[7]["failed"] = ["hidden"]
    elif change == "control":
        args[8]["1000"]["violations"] = ["distortion"]
    elif change == "binary":
        next(iter(args[9].values()))["binary_sha256"] = "other"
    elif change == "overlap":
        next(iter(args[9].values()))["window"] = (10, 15)
    elif change == "unbalanced":
        args[7]["attempts"].pop()
    else:
        code = 1 if change == "oracle" else 0
        monkeypatch.setattr(
            admission.subprocess,
            "run",
            lambda *a, **k: subprocess.CompletedProcess([], code, b"", b"failure"),
        )
    with pytest.raises(host_perf.ReceiptError, match=error):
        admission.admit(*args[:6])
    assert not (args[5] / "admission.json").exists()


@pytest.mark.parametrize(
    "field,value",
    [
        ("minimum_slo_fraction", None),
        ("rate_grid", []),
        ("repetitions_per_rate", 3),
        ("max_span_ns", True),
    ],
)
def test_unfrozen_contract_values_reject(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, field: str, value: object
) -> None:
    args = fixture(tmp_path, monkeypatch)
    contract = json.loads(args[0])
    contract[field] = value
    with pytest.raises(host_perf.ReceiptError):
        admission.contract(host_perf.canonical(contract))
