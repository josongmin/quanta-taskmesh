"""Admission orchestration tests use synthetic measurements, never performance proof."""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Callable

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_admission as admission  # noqa: E402
import host_perf  # noqa: E402

from tools.process_supervisor import SupervisedProcess  # noqa: E402


def test_quantile_rank_interval_does_not_invent_a_thin_tail() -> None:
    thin = admission.quantile_interval([10] * 10)
    assert thin["order_statistic_95_interval_ns"][1] is None
    assert thin["relative_interval_width"] is None
    thick = admission.quantile_interval([10] * 1000)
    assert thick["order_statistic_95_interval_ns"] == [10, 10]
    assert thick["relative_interval_width"] == 0
    assert "iid" in thick["assumption"] and "unverified" in thick["assumption"]
    with pytest.raises(host_perf.ReceiptError, match="empty quantile"):
        admission.quantile_interval([])


def fixture(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> tuple:
    build, study, controls = (tmp_path / name for name in ("build", "study", "controls"))
    for root in (build, study, controls):
        root.mkdir()
    monkeypatch.setattr(host_perf, "_git", lambda *args: "a" * 40 if args[0] == "rev-parse" else "")
    monkeypatch.setattr(
        host_perf, "source_identity", lambda: {"source_head": "a" * 40, "source_dirty": False}
    )
    witness_bytes = b'{"test": "synthetic witness"}'
    (build / "build-witness.json").write_bytes(witness_bytes)
    identity = {"source_head": "a" * 40, "source_dirty": False, "features": []}
    witness = {
        "source_head": "a" * 40,
        "example": "host_load_probe",
        "binary_sha256": "b" * 64,
        "features": [],
    }
    witness["build_environment"] = {}
    witness["effective_build_environment"] = {"CARGO_INCREMENTAL": "0"}
    witness["cargo_config_sha256"] = {}
    monkeypatch.setattr(admission.host_build, "require_matching_library_profiles", lambda *a: None)
    monkeypatch.setattr(admission.host_build, "require_oracle_profiles", lambda *a: None)
    monkeypatch.setattr(admission.host_build, "cargo_config_sha256", lambda _root: {})
    monkeypatch.setattr(
        admission.host_build,
        "verified_profile",
        lambda *_args: {
            "opt_level": "3",
            "debug_assertions": False,
            "test": False,
            "overflow_checks": False,
        },
    )
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
                digests = {
                    role: host_perf.sha256(f"synthetic {role}".encode())
                    for role in admission.host_study.ARTIFACTS
                }
                digests["raw"] = host_perf.sha256((directory / "raw").read_bytes())
                attempts.append(
                    {
                        "id": name,
                        "rate_per_second": rate,
                        "pair_index": pair,
                        "arm": arm,
                        "artifact_sha256": {
                            filename: digests[role]
                            for role, filename in admission.host_study.ARTIFACTS.items()
                        },
                    }
                )
                start = 100 + len(attempts) * 10
                fake_runs[name] = {
                    "identity": identity,
                    "binary_sha256": "b" * 64,
                    "boot_time_ns": 1,
                    "resource_cadence_ms": 250,
                    "window": (start, start + 5),
                    "artifact_sha256": digests,
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
        admission,
        "run_process",
        lambda *a, **k: SupervisedProcess(
            0,
            "test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
            "test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
            "test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n"
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n",
            "",
            False,
            None,
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


def test_oracle_uses_frozen_effective_environment_over_ambient_incremental(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    args = fixture(tmp_path, monkeypatch)
    monkeypatch.setenv("CARGO_INCREMENTAL", "1")
    original = admission.run_process
    observed = {}

    def oracle(*a, **kwargs):
        observed.update(kwargs["env"])
        return original(*a, **kwargs)

    monkeypatch.setattr(admission, "run_process", oracle)
    report = admission.admit(*args[:6])
    recorded = report["oracle"]["effective_build_environment"]
    assert observed["CARGO_INCREMENTAL"] == recorded["CARGO_INCREMENTAL"] == "0"
    assert (
        observed["CARGO_TARGET_DIR"]
        == recorded["CARGO_TARGET_DIR"]
        == str(args[2].resolve() / "target")
    )
    assert all(observed[key] == value for key, value in recorded.items())
    assert recorded["CARGO_PROFILE_TEST_OPT_LEVEL"] == report["build_profile"]["opt_level"]


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
            admission,
            "run_process",
            lambda *a, **k: SupervisedProcess(code, "", "failure", False, None),
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


@pytest.mark.parametrize(
    "profile",
    [
        {"opt_level": "0", "debug_assertions": False, "test": False, "overflow_checks": False},
        {"opt_level": "3", "debug_assertions": True, "test": False},
        {"opt_level": "3", "debug_assertions": False, "test": True},
    ],
)
def test_debug_and_test_profiles_cannot_admit_measurements(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, profile: dict
) -> None:
    args = fixture(tmp_path, monkeypatch)
    monkeypatch.setattr(admission.host_build, "verified_profile", lambda *_args: profile)
    with pytest.raises(host_perf.ReceiptError, match="requires optimized non-test build"):
        admission.admit(*args[:6])


@pytest.mark.parametrize("timed_out,signum", [(True, None), (False, 15)])
def test_timeout_or_interrupted_oracle_never_publishes_admission(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, timed_out: bool, signum: object
) -> None:
    args = fixture(tmp_path, monkeypatch)
    monkeypatch.setattr(
        admission,
        "run_process",
        lambda *a, **k: SupervisedProcess(0, "partial", "failed", timed_out, signum),
    )
    with pytest.raises(host_perf.ReceiptError, match="correctness oracle failed"):
        admission.admit(*args[:6])
    assert (args[5] / "oracle.stdout").read_text() == "partial"
    execution = json.loads((args[5] / "oracle.execution.json").read_bytes())
    assert execution["timed_out"] is timed_out
    assert execution["interrupted_by_signal"] == signum
    assert not (args[5] / "admission.json").exists()


def test_effective_codegen_override_rejects_before_rebuild_or_oracle(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    args = fixture(tmp_path, monkeypatch)
    witness = admission.host_build.verify(args[2])
    witness["build_environment"]["RUSTFLAGS"] = "-C opt-level=0"
    with pytest.raises(host_perf.ReceiptError, match="custom compiler flags"):
        admission.admit(*args[:6])
    assert True not in args[6]
    assert not args[5].exists()


def test_validated_population_cannot_differ_from_the_sealed_ledger(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    args = fixture(tmp_path, monkeypatch)
    next(iter(args[9].values()))["artifact_sha256"]["summary"] = "f" * 64
    with pytest.raises(host_perf.ReceiptError, match="differ from sealed ledger"):
        admission.admit(*args[:6])
    assert True not in args[6]
    assert not args[5].exists()


def test_changed_raw_read_cannot_supply_unvalidated_tail_values(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    args = fixture(tmp_path, monkeypatch)
    path = args[1] / next(iter(args[9])) / "raw"
    original = Path.read_bytes
    changed = json.loads(original(path))
    changed["records"][0]["caller_response_ns"] = 999

    def replaced_read(current: Path) -> bytes:
        if current == path:
            return host_perf.canonical(changed)
        return original(current)

    monkeypatch.setattr(admission, "read_regular_bytes", replaced_read)
    with pytest.raises(host_perf.ReceiptError, match="between typed validation and tail analysis"):
        admission.admit(*args[:6])
    assert True not in args[6]
    assert not args[5].exists()


def replace_preregistered_scenarios(args: tuple, transform: Callable[[dict, int], None]) -> tuple:
    """Keep synthetic custody valid so semantic rejection is the tested boundary."""
    contract = json.loads(args[0])
    for rate in contract["rate_grid"]:
        matching = [a for a in args[7]["attempts"] if a["rate_per_second"] == rate]
        scenario = json.loads((args[1] / matching[0]["id"] / "scenario").read_bytes())
        transform(scenario, rate)
        data = host_perf.canonical(scenario)
        contract["scenario_sha256_by_rate"][str(rate)] = host_perf.sha256(data)
        args[8][str(rate)]["scenario_sha256"] = host_perf.sha256(data)
        for attempt in matching:
            (args[1] / attempt["id"] / "scenario").write_bytes(data)
    data = host_perf.canonical(contract)
    args[7]["contract_sha256"] = host_perf.sha256(data)
    return (data, *args[1:])


@pytest.mark.parametrize("change", ["body", "mix", "class_policy", "topology", "load"])
def test_preregistered_digests_do_not_allow_different_workloads_across_rates(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, change: str
) -> None:
    args = fixture(tmp_path, monkeypatch)

    def transform(scenario: dict, rate: int) -> None:
        if change == "mix":
            split = rate // (2 if rate == 1000 else 4)
            scenario["offers"] = [{"path": "io", "body": {"kind": "noop"}}] * split + [
                {"path": "io", "body": {"kind": "async_sleep", "millis": 1}}
            ] * (rate - split)
        elif rate == 2000:
            if change == "body":
                scenario["offers"] = [{"path": "io", "body": {"kind": "noop"}}] * rate
            elif change == "class_policy":
                scenario["classes"][0]["max_queue_depth"] = 100
            elif change == "topology":
                scenario["topology"] = {"cpu_workers": 2}
            else:
                scenario["load"]["settlement_ms"] = 100

    args = replace_preregistered_scenarios(args, transform)
    with pytest.raises(host_perf.ReceiptError, match="workload .*changed across rates"):
        admission.admit(*args[:6])
    assert True not in args[6]
    assert not args[5].exists()


def test_admission_rejects_actual_dirty_source_before_proof_execution(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    args = fixture(tmp_path, monkeypatch)
    monkeypatch.setattr(
        host_perf, "source_identity", lambda: {"source_head": "a" * 40, "source_dirty": True}
    )
    with pytest.raises(host_perf.ReceiptError, match="exact clean frozen source"):
        admission.admit(*args[:6])
    assert args[6] == []
    assert not args[5].exists()


def test_admission_rejects_content_drift_after_oracle(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    args = fixture(tmp_path, monkeypatch)
    identity = {"source_head": "a" * 40, "source_dirty": False}
    sources = iter(
        [
            {**identity, "source_content_sha256": "a" * 64},
            {**identity, "source_content_sha256": "b" * 64},
        ]
    )
    monkeypatch.setattr(host_perf, "source_identity", lambda: next(sources))
    with pytest.raises(host_perf.ReceiptError, match="source checkout changed"):
        admission.admit(*args[:6])
    assert (args[5] / "oracle.stdout").exists()
    assert not (args[5] / "admission.json").exists()


@pytest.mark.parametrize("path", ["requested_stack_async", "requested_stack_blocking", "local"])
def test_other_public_paths_require_separate_admission_contract(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, path: str
) -> None:
    args = replace_preregistered_scenarios(
        fixture(tmp_path, monkeypatch),
        lambda scenario, rate: scenario.update(offers=[{"path": path}] * rate),
    )
    with pytest.raises(host_perf.ReceiptError, match="supports only io/blocking/cpu"):
        admission.admit(*args[:6])
    assert True not in args[6]
    assert not args[5].exists()


def test_bundle_parsing_and_digest_check_use_the_same_bytes(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    args = fixture(tmp_path, monkeypatch)
    path = args[4] / "1000" / "bundle_path"
    original_read = Path.read_bytes
    original = original_read(path)
    changed = json.loads(original)
    changed["identity"]["host"] = "unvalidated"
    reads = []

    def swapped_read(current: Path) -> bytes:
        if current == path:
            reads.append(current)
            return host_perf.canonical(changed) if len(reads) == 1 else original
        return original_read(current)

    monkeypatch.setattr(admission, "read_regular_bytes", swapped_read)
    with pytest.raises(host_perf.ReceiptError, match="calibration executable/digest differs"):
        admission.admit(*args[:6])
    assert len(reads) == 1
    assert True not in args[6]
