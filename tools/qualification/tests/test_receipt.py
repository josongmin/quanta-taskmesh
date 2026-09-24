"""The receipt validator must fail closed (H16-022-A02)."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import subprocess
import sys
import time
from copy import deepcopy
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[3]
MODULE = REPO / "tools" / "qualification" / "receipt.py"
_spec = importlib.util.spec_from_file_location("receipt", MODULE)
assert _spec and _spec.loader
receipt = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(receipt)

from tools.qualification import producer_evidence as producer_validation  # noqa: E402
from tools.verification import campaign as v02_campaign  # noqa: E402


def hosted_action(source: dict, job: str = "qualification") -> dict:
    return {
        "context": "github-actions",
        "workflow": ".github/workflows/ci.yml",
        "workflow_ref": "taskmesh/taskmesh/.github/workflows/ci.yml@refs/heads/main",
        "workflow_sha256": "e" * 64,
        "source_sha": source["head"],
        "job": job,
        "event": "push",
        "actions": [],
    }


def _producer_records(source: dict, results: list[dict]) -> list[dict]:
    specs = receipt.registrations(receipt.INVENTORY)
    records = []
    for index, spec in enumerate(specs):
        manifest_path = REPO / spec["manifest"]
        manifest = json.loads(manifest_path.read_text())
        surface = spec["surface"]
        if surface == "curated":
            inventory_path = REPO / "tools/verification/mutations.json"
            inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
            mutation_ids = [item["id"] for item in inventory["mutations"]]
            summary = {
                "schema_version": 3,
                "kind": "curated-single-edit-inventory",
                "selection": "full",
                "inventory": "tools/verification/mutations.json",
                "inventory_sha256": hashlib.sha256(inventory_path.read_bytes()).hexdigest(),
                "inventory_total": len(mutation_ids),
                "total": len(mutation_ids),
                "problems": 0,
                "status": "PASS",
                "status_counts": {"KILLED": len(mutation_ids)},
                "scope": {
                    "runner_counts": {"cargo": 1},
                    "control_entries": 0,
                    "semantics": "curated-single-edit-only-not-generated-score",
                },
                "source_unchanged": True,
                "campaign": {
                    "source_head": source["head"],
                    "snapshot_sha256": source["paths_digest"],
                    "source_dirty": source["dirty"],
                },
                "results": [
                    {"mutation_id": mutation_id, "status": "KILLED"} for mutation_id in mutation_ids
                ],
            }
            selected = len(mutation_ids)
        elif surface == "generated":
            summary = {
                "schema_version": 2,
                "kind": "generated-cargo-mutants",
                "status": "PASS",
                "problems": [],
                "counts": {"caught": 1, "missed": 0, "unviable": 0, "timeout": 0, "equivalent": 0},
                "denominator": 1,
                "quality": {"numerator": 1, "denominator": 1, "excluded": {"unviable": 0}},
                "limitations": [],
                "outcomes": {
                    "caught": ["m1"],
                    "missed": [],
                    "unviable": [],
                    "timeout": [],
                },
                "planned_mutants": ["m1"],
                "baseline_sha256": "b" * 64,
                "command": {"argv": ["cargo", "mutants", "--workspace"]},
                "process": {
                    "exit_code": 0,
                    "signal": None,
                    "timed_out": False,
                    "interrupted_by_signal": None,
                },
                "discovery_process": {
                    "exit_code": 0,
                    "signal": None,
                    "timed_out": False,
                    "interrupted_by_signal": None,
                },
                "excluded_mutants": [],
                "excluded_count": 0,
                "source_unchanged": True,
                "campaign": {
                    "source_head": source["head"],
                    "snapshot_sha256": source["paths_digest"],
                    "source_dirty": source["dirty"],
                },
            }
            selected = 1
        elif surface == "fuzz":
            targets = []
            for name, checkpoints in manifest["required_targets"].items():
                targets.append(
                    {
                        "target": name,
                        "attempted": True,
                        "runs": 1,
                        "valid_inputs": 1,
                        "semantic_checkpoints": {checkpoint: 1 for checkpoint in checkpoints},
                    }
                )
            summary = {
                "schema_version": 1,
                "kind": "taskmesh-fuzz-evidence",
                "producer_id": spec["producer_id"],
                "required_targets": list(manifest["required_targets"]),
                "executed_targets": list(manifest["required_targets"]),
                "targets": targets,
                "source_stable": True,
                "status": "PASS",
                "problems": [],
            }
            selected = len(targets)
        else:
            models = []
            loom = manifest["checkers"]["loom"]
            for model_id in loom["required_models"]:
                models.append(
                    {"checker": "loom", "model_id": model_id, "completed": 1, **loom["bounds"]}
                )
            shuttle = manifest["checkers"]["shuttle"]
            for model_id, expected in shuttle["required_models"].items():
                models.append(
                    {
                        "checker": "shuttle",
                        "model_id": model_id,
                        "seed": expected["seed"],
                        "requested": expected["requested"],
                        "completed": expected["requested"],
                        "scheduler": shuttle["scheduler"],
                        "max_steps": shuttle["max_steps"],
                    }
                )
            summary = {
                "schema_version": 1,
                "kind": "taskmesh-modelcheck-evidence",
                "producer_id": spec["producer_id"],
                "models": models,
                "replay": {
                    "model_id": manifest["replay"]["model_id"],
                    "seed": manifest["replay"]["seed"],
                    "result": manifest["replay"]["result"],
                    "artifact": {
                        "path": "target/modelcheck/replay/schedule-fixture.txt",
                        "sha256": "d" * 64,
                        "size": 1,
                        "role": "replay",
                    },
                },
                "processes": {
                    name: {"exit_code": 0, "signal": None, "timed_out": False}
                    for name in ("loom", "shuttle", "replay_generate", "replay_verify")
                },
                "source_stable": True,
                "status": "PASS",
                "problems": [],
            }
            selected = len(models)
        manifest_identity = {
            "path": spec["manifest"],
            "sha256": hashlib.sha256(manifest_path.read_bytes()).hexdigest(),
            "size": manifest_path.stat().st_size,
        }
        summary_identity = {"path": spec["summary"], "sha256": f"{index + 1:x}" * 64, "size": 1}
        envelope = {
            "schema_version": 1,
            "kind": "taskmesh-evidence-envelope",
            "producer": {"id": spec["producer_id"], "version": spec["version"]},
            "source": {
                "head": source["head"],
                "tree_digest": source["paths_digest"],
                "dirty": source["dirty"],
            },
            "command": {
                "argv": summary["command"]["argv"] if surface == "generated" else ["producer"],
                "cwd": ".",
                "environment": {},
            },
            "tools": [{"name": "tool", "version": "tool 1.0.0"}],
            "configs": [
                {
                    "path": path,
                    "sha256": (
                        manifest_identity["sha256"] if path == spec["manifest"] else "9" * 64
                    ),
                }
                for path in spec["required_configs"]
            ],
            "action": hosted_action(source, spec["hosted_job"]),
            "artifacts": [
                {"path": f"target/raw-{index}.log", "sha256": "f" * 64, "size": 1, "role": "raw"},
                {**summary_identity, "role": "summary"},
                *([deepcopy(summary["replay"]["artifact"])] if surface == "modelcheck" else []),
            ],
            "result": {
                "status": "PASS",
                "exit_code": 0,
                "started_at": "2026-09-16T00:00:00Z",
                "finished_at": "2026-09-16T00:01:00Z",
                "selected_count": selected,
                "executed_count": selected,
            },
        }
        envelope["command"]["sha256"] = receipt.canonical_digest(
            {key: envelope["command"][key] for key in ("argv", "cwd", "environment")}
        )
        envelope["tools"][0]["identity_sha256"] = receipt.canonical_digest(
            {"name": "tool", "version": "tool 1.0.0"}
        )
        record = {
            "registration": spec["registration"],
            "gate_id": spec["gate_id"],
            "surface": surface,
            "manifest": {
                "identity": manifest_identity,
                "payload_sha256": receipt.canonical_digest(manifest),
                "payload": manifest,
            },
            "summary": {
                "identity": summary_identity,
                "payload_sha256": receipt.canonical_digest(summary),
                "payload": summary,
            },
            "envelope": {
                "identity": {"path": spec["envelope"], "sha256": "a" * 64, "size": 1},
                "payload_sha256": receipt.canonical_digest(envelope),
                "payload": envelope,
            },
        }
        gate = next(item for item in results if item["id"] == spec["gate_id"])
        gate.update(
            {
                "evidence_kind": "producer",
                "producer_registration": spec["registration"],
                "evidence_digest": record["envelope"]["payload_sha256"],
                "imported_producer": True,
                "derived_from": spec["envelope"],
                "producer_started_at": envelope["result"]["started_at"],
                "producer_finished_at": envelope["result"]["finished_at"],
            }
        )
        records.append(record)
    return records


def hosted_gate_runner(results: list[dict]) -> dict:
    producer_gate_ids = sorted(spec["gate_id"] for spec in receipt.registrations(receipt.INVENTORY))
    raw_results = [result for result in results if result["id"] not in producer_gate_ids]
    return {
        "exit_code": 1,
        "timed_out": False,
        "interrupted_by_signal": None,
        "started_at": "2026-09-16T00:00:00Z",
        "duration_s": 1.0,
        "skipped_gate_ids": producer_gate_ids,
        "raw_required": receipt.summarize_gate_results(raw_results),
    }


def passing_gate_results(required: list[str]) -> list[dict]:
    results = [{"id": gate, "status": "PASS", "exit_code": 0} for gate in required]
    inventory = json.loads((REPO / "tools" / "gates" / "inventory.json").read_text())
    specs = {
        gate["id"]: gate["status_line"] for gate in inventory["gates"] if "status_line" in gate
    }
    for result in results:
        if result["id"] in specs:
            spec = specs[result["id"]]
            status_line = f"{spec['marker']} {spec['require']}"
            if result["id"] == "test":
                status_line += (
                    " runner=cargo runner_version=1.95.0"
                    " fixture_runner=cargo cargo_version=1.95.0"
                )
            result["status_line"] = status_line
    return results


def qualified_receipt() -> dict:
    required = json.loads((REPO / "tools" / "gates" / "required.json").read_text())["required"]
    results = passing_gate_results(required)
    source = {
        "head": "0" * 40,
        "tree": "1" * 40,
        "dirty": False,
        "isolated_checkout": True,
        "paths_digest": "a" * 64,
    }
    producers = _producer_records(source, results)
    by_surface = {item["surface"]: item["summary"]["payload"] for item in producers}
    return {
        "schema_version": receipt.SCHEMA_VERSION,
        "generated_at": "2026-09-16T00:00:00+00:00",
        "finished_at": "2026-09-16T00:10:00+00:00",
        "environment": {"toolchain": "rustc 1.95.0"},
        "gate_runner": hosted_gate_runner(results),
        "source": source,
        "source_after": {
            "head": source["head"],
            "tree": source["tree"],
            "paths_digest": source["paths_digest"],
            "dirty": False,
        },
        "attestation": {
            "kind": "github-actions",
            "hosted_qualification_eligible": True,
            "head": source["head"],
            "checks": {name: True for name in receipt.ATTESTATION_CHECK_NAMES},
            "action": hosted_action(source),
            "producer_jobs": {
                spec["hosted_job"]: "success" for spec in receipt.registrations(receipt.INVENTORY)
            },
        },
        "gates": {
            "schema_version": receipt.GATE_SCHEMA_VERSION,
            **receipt.summarize_gate_results(results),
            "results": results,
        },
        "mutations": by_surface["curated"],
        "generated_mutation_sweep": by_surface["generated"],
        "producers": producers,
    }


CLEAN = {"head": "0" * 40, "dirty": False, "paths_digest": "a" * 64}


def producer_record(value: dict, surface: str) -> dict:
    return next(item for item in value["producers"] if item["surface"] == surface)


def resign_summary(value: dict, surface: str) -> None:
    record = producer_record(value, surface)
    record["summary"]["payload_sha256"] = receipt.canonical_digest(record["summary"]["payload"])


def test_a_complete_clean_receipt_qualifies() -> None:
    assert receipt.evaluate(qualified_receipt(), CLEAN)["status"] == "QUALIFIED"


def test_hosted_import_cannot_hide_an_unrelated_raw_gate_failure() -> None:
    value = qualified_receipt()
    value["gate_runner"]["raw_required"]["required_not_passed"] = ["fmt-check"]
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("gate runner" in reason for reason in verdict["reasons"])


@pytest.mark.parametrize("duration", [float("nan"), float("inf"), -1, True])
def test_gate_runner_duration_must_be_finite_nonnegative(duration: object) -> None:
    value = qualified_receipt()
    value["gate_runner"]["duration_s"] = duration
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("gate runner duration_s" in reason for reason in verdict["reasons"])


def test_local_evidence_never_claims_hosted_qualification() -> None:
    r = qualified_receipt()
    r["attestation"] = receipt.collection_attestation(r["source"], hosted_ci=False)
    r["source"]["isolated_checkout"] = False
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("explicit local qualification" in reason for reason in verdict["reasons"])


def local_qualified_receipt() -> dict:
    r = qualified_receipt()
    source = r["source"]
    action = receipt.runtime_action(
        root=REPO,
        source_head=source["head"],
        local_workflow=".github/workflows/ci.yml",
        local_job="local-qualification",
    )
    r["attestation"] = {
        "kind": "local",
        "hosted_qualification_eligible": False,
        "local_qualification_eligible": True,
        "head": source["head"],
        "checks": {name: True for name in receipt.LOCAL_ATTESTATION_CHECK_NAMES},
        "action": action,
        "producer_jobs": {},
    }
    r["source"]["isolated_checkout"] = False
    for record in r["producers"]:
        record["envelope"]["payload"]["action"] = action
        record["envelope"]["payload_sha256"] = receipt.canonical_digest(
            record["envelope"]["payload"]
        )
        gate_id = next(
            spec["gate_id"]
            for spec in receipt.registrations(receipt.INVENTORY)
            if spec["registration"] == record["registration"]
        )
        next(result for result in r["gates"]["results"] if result["id"] == gate_id)[
            "evidence_digest"
        ] = record["envelope"]["payload_sha256"]
    for result in r["gates"]["results"]:
        result.pop("imported_producer", None)
    r["gate_runner"] = {
        "exit_code": 0,
        "timed_out": False,
        "interrupted_by_signal": None,
        "started_at": "2026-09-16T00:00:00Z",
        "duration_s": 1.0,
        "skipped_gate_ids": [],
        "raw_required": receipt.summarize_gate_results(r["gates"]["results"]),
    }
    return r


def test_explicit_clean_local_full_receipt_qualifies() -> None:
    assert receipt.evaluate(local_qualified_receipt(), CLEAN)["status"] == "QUALIFIED"


@pytest.mark.parametrize("exit_code", [1, None, False, "0"])
def test_local_pass_sidecar_cannot_hide_runner_failure(exit_code: object) -> None:
    value = local_qualified_receipt()
    assert value["gates"]["qualified"] is True
    value["gate_runner"]["exit_code"] = exit_code
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("gate runner" in reason for reason in verdict["reasons"])


def test_gate_runner_timeout_cannot_qualify_after_zero_exit() -> None:
    value = local_qualified_receipt()
    value["gate_runner"]["timed_out"] = True
    value["gate_runner"]["exit_code"] = 0
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("gate runner" in reason and "timeout" in reason for reason in verdict["reasons"])


@pytest.mark.parametrize("field", ["timed_out", "interrupted_by_signal"])
def test_gate_runner_process_flags_are_required(field: str) -> None:
    value = local_qualified_receipt()
    del value["gate_runner"][field]
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("gate runner" in reason and field in reason for reason in verdict["reasons"])


def test_generated_mutation_sweep_is_required_and_separate_from_curated() -> None:
    r = qualified_receipt()
    assert r["generated_mutation_sweep"] is not r["mutations"]
    assert r["generated_mutation_sweep"]["kind"] == "generated-cargo-mutants"
    assert r["mutations"]["kind"] == "curated-single-edit-inventory"
    assert receipt.evaluate(r, CLEAN)["status"] == "QUALIFIED"


def test_generated_and_curated_summaries_cannot_be_conflated() -> None:
    r = qualified_receipt()
    r["generated_mutation_sweep"] = deepcopy(r["mutations"])
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("generated mutation sidecar differs" in reason for reason in verdict["reasons"])


def test_generated_subset_cannot_be_promoted_to_pass() -> None:
    r = qualified_receipt()
    summary = producer_record(r, "generated")["summary"]["payload"]
    summary["command"]["argv"].extend(["--package", "taskmesh-rayon"])
    resign_summary(r, "generated")
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED", "generated mutation subset was promoted to PASS"
    assert any("subset" in reason for reason in verdict["reasons"])


def test_curated_one_mutant_cannot_self_assert_full_inventory() -> None:
    r = qualified_receipt()
    summary = producer_record(r, "curated")["summary"]["payload"]
    summary["results"] = summary["results"][:1]
    summary["inventory_total"] = summary["total"] = 1
    summary["status_counts"] = {"KILLED": 1}
    envelope = producer_record(r, "curated")["envelope"]["payload"]
    envelope["result"]["selected_count"] = envelope["result"]["executed_count"] = 1
    resign_summary(r, "curated")
    record = producer_record(r, "curated")
    record["envelope"]["payload_sha256"] = receipt.canonical_digest(envelope)
    gate = next(item for item in r["gates"]["results"] if item["id"] == "mutants-critical")
    gate["evidence_digest"] = record["envelope"]["payload_sha256"]
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED", "one-mutant curated denominator was trusted"
    assert any("curated mutation inventory" in reason for reason in verdict["reasons"])


def test_curated_inventory_digest_and_order_are_authoritative() -> None:
    for drift in ("digest", "order"):
        r = qualified_receipt()
        summary = producer_record(r, "curated")["summary"]["payload"]
        if drift == "digest":
            summary["inventory_sha256"] = "0" * 64
        else:
            summary["results"][0], summary["results"][1] = (
                summary["results"][1],
                summary["results"][0],
            )
        resign_summary(r, "curated")
        verdict = receipt.evaluate(r, CLEAN)
        assert verdict["status"] == "NOT_QUALIFIED", drift
        assert any("curated mutation inventory" in reason for reason in verdict["reasons"])


def test_generated_summary_workspace_claim_must_match_executed_command() -> None:
    r = qualified_receipt()
    record = producer_record(r, "generated")
    envelope = record["envelope"]["payload"]
    envelope["command"]["argv"] = ["cargo", "mutants", "--package", "taskmesh-rayon"]
    envelope["command"]["sha256"] = receipt.canonical_digest(
        {key: envelope["command"][key] for key in ("argv", "cwd", "environment")}
    )
    record["envelope"]["payload_sha256"] = receipt.canonical_digest(envelope)
    gate = next(item for item in r["gates"]["results"] if item["id"] == "mutants-generated")
    gate["evidence_digest"] = record["envelope"]["payload_sha256"]
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED", "workspace summary masked package command"
    assert any("generated mutation command" in reason for reason in verdict["reasons"])


@pytest.mark.parametrize("drift", ["missing", "duplicate", "different", "baseline"])
def test_generated_planned_identity_and_baseline_are_required(drift: str) -> None:
    r = qualified_receipt()
    summary = producer_record(r, "generated")["summary"]["payload"]
    if drift == "missing":
        del summary["planned_mutants"]
    elif drift == "duplicate":
        summary["planned_mutants"] = ["m1", "m1"]
    elif drift == "different":
        summary["planned_mutants"] = ["m2"]
    else:
        del summary["baseline_sha256"]
    resign_summary(r, "generated")
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED", drift
    assert any(
        "generated mutation planned" in reason or "baseline" in reason
        for reason in verdict["reasons"]
    )


def test_generated_outcomes_cannot_be_omitted_behind_counts() -> None:
    r = qualified_receipt()
    summary = producer_record(r, "generated")["summary"]["payload"]
    del summary["outcomes"]
    resign_summary(r, "generated")
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("generated mutation outcomes" in reason for reason in verdict["reasons"])


def test_generated_unviable_is_visible_but_not_a_quality_failure() -> None:
    r = qualified_receipt()
    record = producer_record(r, "generated")
    summary = record["summary"]["payload"]
    envelope = record["envelope"]["payload"]
    result = envelope["result"]
    summary["counts"]["unviable"] = 1
    summary["denominator"] = 2
    summary["quality"] = {
        "numerator": 1,
        "denominator": 1,
        "excluded": {"unviable": 1},
    }
    summary["limitations"] = ["unviable"]
    summary["outcomes"]["unviable"] = ["m2"]
    summary["planned_mutants"] = ["m1", "m2"]
    result["selected_count"] = result["executed_count"] = 2
    assert producer_validation._generated_problems(summary, result, envelope) == []


@pytest.mark.parametrize("process_name", ["process", "discovery_process"])
def test_generated_interrupted_process_cannot_qualify(process_name: str) -> None:
    r = qualified_receipt()
    record = producer_record(r, "generated")
    summary = record["summary"]["payload"]
    summary[process_name]["interrupted_by_signal"] = 15
    envelope = record["envelope"]["payload"]

    problems = producer_validation._generated_problems(summary, envelope["result"], envelope)

    assert any(f"generated mutation {process_name} did not complete cleanly" in p for p in problems)


def test_generated_exclusion_count_must_match_list() -> None:
    r = qualified_receipt()
    record = producer_record(r, "generated")
    summary = record["summary"]["payload"]
    summary["excluded_mutants"] = ["omitted-mutant"]
    envelope = record["envelope"]["payload"]

    problems = producer_validation._generated_problems(summary, envelope["result"], envelope)

    assert any("excluded mutant count" in p for p in problems)


@pytest.mark.parametrize("field", ["quality", "limitations"])
def test_generated_unviable_accounting_cannot_be_omitted(field: str) -> None:
    r = qualified_receipt()
    record = producer_record(r, "generated")
    summary = record["summary"]["payload"]
    del summary[field]
    problems = producer_validation._generated_problems(
        summary, record["envelope"]["payload"]["result"], record["envelope"]["payload"]
    )
    assert any(field in problem for problem in problems)


def test_generated_structured_raw_binds_plan_baseline_and_category_ids(tmp_path: Path) -> None:
    r = qualified_receipt()
    record = producer_record(r, "generated")
    spec = next(
        item for item in receipt.registrations(receipt.INVENTORY) if item["surface"] == "generated"
    )
    summary = record["summary"]["payload"]
    envelope = record["envelope"]["payload"]
    output = (tmp_path / spec["summary"]).parent
    raw = output / "raw" / "mutants.out"
    raw.mkdir(parents=True)
    config = tmp_path / ".cargo/mutants.toml"
    config.parent.mkdir(parents=True)
    config.write_text('exclude_re = ["excluded"]\n', encoding="utf-8")
    summary["excluded_mutants"] = ["m-excluded"]
    summary["excluded_count"] = 1
    baseline = {"scenario": "Baseline", "summary": "Success"}
    summary["baseline_sha256"] = receipt.canonical_digest(baseline)
    documents = {
        "mutants.json": [{"name": "m1"}],
        "outcomes.json": {
            "outcomes": [
                baseline,
                {"scenario": {"Mutant": {"name": "m1"}}, "summary": "CaughtMutant"},
            ]
        },
    }
    for name, payload in documents.items():
        (raw / name).write_text(json.dumps(payload), encoding="utf-8")
    for name, value in (
        ("caught.txt", "m1\n"),
        ("missed.txt", ""),
        ("unviable.txt", ""),
        ("timeout.txt", ""),
    ):
        (raw / name).write_text(value, encoding="utf-8")
    unfiltered = output / "raw/unfiltered-mutants.json"
    unfiltered.write_text(json.dumps([{"name": "m1"}, {"name": "m-excluded"}]), encoding="utf-8")
    summary["raw_artifacts"] = []
    for path in sorted([*raw.iterdir(), unfiltered]):
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        summary["raw_artifacts"].append(
            {
                "path": path.relative_to(output / "raw").as_posix(),
                "sha256": digest,
                "size": path.stat().st_size,
            }
        )
        if path.stat().st_size:
            envelope["artifacts"].append(
                {
                    "path": path.relative_to(tmp_path).as_posix(),
                    "sha256": digest,
                    "size": path.stat().st_size,
                    "role": "raw",
                }
            )
    assert (
        producer_validation._generated_structured_raw_problems(tmp_path, spec, summary, envelope)
        == []
    )
    summary["excluded_mutants"] = ["m-other"]
    assert any(
        "excluded mutants differ from unfiltered" in problem
        for problem in producer_validation._generated_structured_raw_problems(
            tmp_path, spec, summary, envelope
        )
    )
    summary["excluded_mutants"] = ["m-excluded"]
    unfiltered.write_text(json.dumps([{"name": "m1"}, {"name": "unapproved"}]), encoding="utf-8")
    digest = hashlib.sha256(unfiltered.read_bytes()).hexdigest()
    raw_record = next(item for item in summary["raw_artifacts"] if item["path"] == unfiltered.name)
    raw_record["sha256"] = digest
    raw_record["size"] = unfiltered.stat().st_size
    envelope_record = next(
        item for item in envelope["artifacts"] if item["path"].endswith(unfiltered.name)
    )
    envelope_record["sha256"] = digest
    envelope_record["size"] = unfiltered.stat().st_size
    assert any(
        "excluded mutant is not covered by source policy" in problem
        for problem in producer_validation._generated_structured_raw_problems(
            tmp_path, spec, summary, envelope
        )
    )
    unfiltered.write_text(json.dumps([{"name": "m1"}, {"name": "m-excluded"}]), encoding="utf-8")
    digest = hashlib.sha256(unfiltered.read_bytes()).hexdigest()
    raw_record["sha256"] = envelope_record["sha256"] = digest
    raw_record["size"] = envelope_record["size"] = unfiltered.stat().st_size
    original_raw = deepcopy(summary["raw_artifacts"])
    duplicate = deepcopy(original_raw[0])
    duplicate["sha256"] = "0" * 64
    summary["raw_artifacts"].insert(0, duplicate)
    problems = producer_validation._generated_structured_raw_problems(
        tmp_path, spec, summary, envelope
    )
    assert any("duplicate or differ from disk" in problem for problem in problems)
    summary["raw_artifacts"] = [
        *original_raw,
        {"path": "unknown.log", "sha256": "0" * 64, "size": 1},
    ]
    problems = producer_validation._generated_structured_raw_problems(
        tmp_path, spec, summary, envelope
    )
    assert any("duplicate or differ from disk" in problem for problem in problems)
    summary["raw_artifacts"] = original_raw
    (raw / "mutants.json").write_text('[{"name":"m2"}]', encoding="utf-8")
    problems = producer_validation._generated_structured_raw_problems(
        tmp_path, spec, summary, envelope
    )
    assert any("planned IDs differ" in problem for problem in problems)
    (raw / "mutants.json").write_text("{}", encoding="utf-8")
    problems = producer_validation._generated_structured_raw_problems(
        tmp_path, spec, summary, envelope
    )
    assert any("not a list" in problem for problem in problems)
    (raw / "mutants.json").write_bytes(b"\xff")
    problems = producer_validation._generated_structured_raw_problems(
        tmp_path, spec, summary, envelope
    )
    assert any("malformed JSON" in problem for problem in problems)


def test_run_json_never_reuses_a_stale_sidecar(tmp_path: Path, monkeypatch) -> None:
    sidecar = tmp_path / "stale.json"
    sidecar.write_text('{"qualified": true}\n', encoding="utf-8")
    monkeypatch.setattr(
        receipt,
        "run_process",
        lambda *args, **kwargs: SimpleNamespace(
            returncode=1,
            stderr="crashed",
            timed_out=False,
            interrupted_by_signal=None,
        ),
    )
    run = receipt.run_json(["broken-runner"], sidecar, timeout_seconds=1)
    assert run.exit_code == 1
    assert run.payload["error"] == "runner did not write its receipt"
    assert "qualified" not in run.payload


def test_run_json_bounds_a_runner_that_exits_zero_after_timeout(
    tmp_path: Path, monkeypatch
) -> None:
    sidecar = tmp_path / "runner.json"
    child = (
        "import json, signal, sys; "
        "signal.signal(signal.SIGTERM, lambda *_: sys.exit(0)); "
        "open(sys.argv[-1], 'w').write(json.dumps({'results': []})); "
        "signal.pause()"
    )
    monkeypatch.setattr(receipt, "REPO", tmp_path)
    started = time.monotonic()
    run = receipt.run_json([sys.executable, "-c", child], sidecar, timeout_seconds=0.2)
    assert time.monotonic() - started < 2
    assert run.payload == {"results": []}
    assert run.exit_code == 0
    assert run.timed_out is True


def test_run_json_preserves_completed_runner_output(tmp_path: Path, monkeypatch) -> None:
    sidecar = tmp_path / "runner.json"
    child = (
        "import json, sys; "
        "open(sys.argv[-1], 'w').write(json.dumps({'results': [{'id': 'done'}]})); "
        "print('runner completed')"
    )
    monkeypatch.setattr(receipt, "REPO", tmp_path)
    run = receipt.run_json([sys.executable, "-c", child], sidecar, timeout_seconds=2)
    assert run.payload == {"results": [{"id": "done"}]}
    assert run.exit_code == 0
    assert run.timed_out is False
    assert run.interrupted_by_signal is None


def test_run_json_records_an_undecodable_sidecar_as_failed_evidence(
    tmp_path: Path, monkeypatch
) -> None:
    sidecar = tmp_path / "runner.json"
    child = "import sys; open(sys.argv[-1], 'wb').write(b'\\xff')"
    monkeypatch.setattr(receipt, "REPO", tmp_path)

    run = receipt.run_json([sys.executable, "-c", child], sidecar, timeout_seconds=2)

    assert run.exit_code == 0
    assert "runner wrote invalid" in run.payload["error"]
    assert run.payload.get("results") is None


@pytest.mark.parametrize(
    "declared",
    [
        "/tmp/out.json",
        "target/../source.json",
        "target",
        "tools/source.json",
    ],
)
def test_registered_output_cleanup_rejects_unsafe_paths(tmp_path: Path, declared: str) -> None:
    (tmp_path / "target").mkdir()
    with pytest.raises(ValueError):
        receipt.clear_registered_outputs(
            tmp_path, [{"summary": declared, "envelope": "target/safe/envelope.json"}]
        )


def test_registered_output_cleanup_rejects_symlink_escape(tmp_path: Path) -> None:
    (tmp_path / "target").mkdir()
    source = tmp_path / "source"
    source.mkdir()
    (source / "summary.json").write_text("do not delete", encoding="utf-8")
    (tmp_path / "target" / "escape").symlink_to(source, target_is_directory=True)
    with pytest.raises(ValueError, match="below repository target"):
        receipt.clear_registered_outputs(
            tmp_path,
            [
                {
                    "summary": "target/escape/summary.json",
                    "envelope": "target/safe/envelope.json",
                }
            ],
        )
    assert (source / "summary.json").read_text(encoding="utf-8") == "do not delete"


def test_registered_output_cleanup_rejects_symlinked_target_root(tmp_path: Path) -> None:
    outside = tmp_path / "outside"
    outside.mkdir()
    summary = outside / "summary.json"
    summary.write_text("do not delete", encoding="utf-8")
    (tmp_path / "target").symlink_to(outside, target_is_directory=True)
    with pytest.raises(ValueError, match="target/ must not be a symlink"):
        receipt.clear_registered_outputs(
            tmp_path,
            [
                {
                    "summary": "target/summary.json",
                    "envelope": "target/envelope.json",
                }
            ],
        )
    assert summary.read_text(encoding="utf-8") == "do not delete"


@pytest.mark.parametrize(
    "declared", ["../outside.json", "/tmp/outside.json", "target/../outside.json"]
)
def test_producer_collection_rejects_inventory_path_escape(tmp_path: Path, declared: str) -> None:
    records = producer_validation.collect_records(
        tmp_path,
        [
            {
                "registration": "test",
                "gate_id": "test-gate",
                "surface": "test",
                "manifest": declared,
                "summary": "target/summary.json",
                "envelope": "target/envelope.json",
            }
        ],
    )
    assert "error" in records[0]
    assert "repo-relative" in records[0]["error"]


def test_producer_validation_rejects_symlinked_sidecar(tmp_path: Path) -> None:
    outside = tmp_path.parent / f"{tmp_path.name}-producer-sidecar-outside.json"
    outside.write_text('{"private": "do not read"}', encoding="utf-8")
    (tmp_path / "target").mkdir()
    (tmp_path / "target" / "summary.json").symlink_to(outside)
    problems = producer_validation._embedded_sidecar_problems(
        root=tmp_path, path="target/summary.json", embedded={}, name="summary"
    )
    assert any("escapes repository root" in problem for problem in problems)
    assert all("do not read" not in problem for problem in problems)


def test_registered_output_cleanup_only_unlinks_confined_files(tmp_path: Path) -> None:
    output = tmp_path / "target" / "campaign"
    output.mkdir(parents=True)
    summary = output / "summary.json"
    envelope = output / "envelope.json"
    summary.write_text("summary", encoding="utf-8")
    envelope.write_text("envelope", encoding="utf-8")
    receipt.clear_registered_outputs(
        tmp_path,
        [
            {
                "summary": "target/campaign/summary.json",
                "envelope": "target/campaign/envelope.json",
            }
        ],
    )
    assert not summary.exists() and not envelope.exists()


def test_a_source_mismatch_is_not_qualified() -> None:
    verdict = receipt.evaluate(qualified_receipt(), {**CLEAN, "paths_digest": "different"})
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("source digest" in r for r in verdict["reasons"])


def test_final_receipt_rejects_a_conflicting_pass_status_line() -> None:
    value = qualified_receipt()
    result = next(item for item in value["gates"]["results"] if item["id"] == "bench-iai")
    result["status_line"] = "taskmesh-iai-gate status=BASELINE_CREATED status=QUALIFIED"
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("bench-iai" in reason and "status line" in reason for reason in verdict["reasons"])


def test_imported_producer_pass_uses_its_envelope_instead_of_a_local_status_line() -> None:
    value = qualified_receipt()
    result = next(item for item in value["gates"]["results"] if item["id"] == "modelcheck")
    assert result["imported_producer"] is True
    result.pop("status_line")
    assert receipt.evaluate(value, CLEAN)["status"] == "QUALIFIED"


def test_a_receipt_missing_the_collectors_shape_is_not_qualified() -> None:
    for key in receipt.REQUIRED_RECEIPT_KEYS:
        r = qualified_receipt()
        r.pop(key)
        verdict = receipt.evaluate(r, CLEAN)
        assert verdict["status"] == "NOT_QUALIFIED", key
        assert any(f"lacks {key!r}" in x for x in verdict["reasons"]), key


def test_gitignored_files_are_not_part_of_the_source_identity(repo: Path) -> None:
    (repo / ".gitignore").write_text("*.local\n")
    subprocess.run(["git", "add", ".gitignore"], cwd=repo, check=True)
    subprocess.run(
        [
            "git",
            "-c",
            "user.email=t@example.com",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "ignore",
        ],
        cwd=repo,
        check=True,
    )
    before = receipt.source_identity(repo)
    (repo / "scratch.local").write_text("not source\n")
    after = receipt.source_identity(repo)
    assert after["paths_digest"] == before["paths_digest"], "an ignored file is not source"
    assert not after["dirty"], "and does not dirty the tree"


def test_a_receipt_for_another_head_is_not_qualified() -> None:
    verdict = receipt.evaluate(qualified_receipt(), {**CLEAN, "head": "elsewhere"})
    assert any("is not the current HEAD" in r for r in verdict["reasons"])


def test_a_receipt_claiming_a_clean_tree_over_a_dirty_one_is_caught() -> None:
    # The forgery the audit demonstrated: `dirty: false` and twenty PASSes,
    # hand-written on a dirty tree with the right digest. The tree in front of
    # the validator is the authority on its own dirtiness.
    verdict = receipt.evaluate(qualified_receipt(), {**CLEAN, "dirty": True})
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any(
        "claims a clean tree but the current working tree is dirty" in r for r in verdict["reasons"]
    )


def test_a_mutation_subreceipt_that_disagrees_about_the_tree_is_caught() -> None:
    r = qualified_receipt()
    r["mutations"]["campaign"]["source_dirty"] = True
    resign_summary(r, "curated")
    verdict = receipt.evaluate(r, CLEAN)
    assert any("campaign source identity differs" in reason for reason in verdict["reasons"])


def test_a_dirty_tree_is_not_an_immutable_qualification() -> None:
    r = qualified_receipt()
    r["source"]["dirty"] = True
    verdict = receipt.evaluate(r, {**CLEAN, "dirty": True})
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("dirty" in x for x in verdict["reasons"])


def test_a_tree_that_changed_during_the_run_is_not_qualified() -> None:
    r = qualified_receipt()
    r["source_after"]["paths_digest"] = "moved"
    verdict = receipt.evaluate(r, CLEAN)
    assert any("changed while the gates ran" in x for x in verdict["reasons"])


@pytest.mark.parametrize("missing_digest", [None, ""])
def test_source_after_requires_the_exact_nonempty_source_digest(
    missing_digest: object,
) -> None:
    r = qualified_receipt()
    r["source_after"]["paths_digest"] = missing_digest
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("source_after paths_digest" in reason for reason in verdict["reasons"])


@pytest.mark.parametrize("missing_digest", [None, ""])
def test_source_requires_a_nonempty_paths_digest(missing_digest: object) -> None:
    r = qualified_receipt()
    r["source"]["paths_digest"] = missing_digest
    verdict = receipt.evaluate(r, current=None)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("source paths_digest" in reason for reason in verdict["reasons"])


def test_a_tree_whose_dirty_state_changed_during_the_run_is_not_qualified() -> None:
    r = qualified_receipt()
    r["source_after"]["dirty"] = True
    verdict = receipt.evaluate(r, CLEAN)
    assert any("dirtiness changed" in reason for reason in verdict["reasons"])


def test_a_stored_verdict_must_match_the_recomputed_verdict() -> None:
    r = qualified_receipt()
    r["verdict"] = {"status": "NOT_QUALIFIED", "reasons": ["forged"]}
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("stored verdict does not match" in reason for reason in verdict["reasons"])


def test_a_missing_required_gate_is_not_qualified() -> None:
    r = qualified_receipt()
    r["gates"]["results"] = [x for x in r["gates"]["results"] if x["id"] != "semgrep"]
    r["gates"].update(receipt.summarize_gate_results(r["gates"]["results"]))
    verdict = receipt.evaluate(r, CLEAN)
    assert any("semgrep was not run" in x for x in verdict["reasons"])


def test_a_skipped_or_failed_required_gate_is_not_qualified() -> None:
    for status in ("FAIL", "SKIPPED_PLATFORM", "NOT_RUN"):
        r = qualified_receipt()
        r["gates"]["results"][0]["status"] = status
        r["gates"].update(receipt.summarize_gate_results(r["gates"]["results"]))
        verdict = receipt.evaluate(r, CLEAN)
        assert verdict["status"] == "NOT_QUALIFIED", status
        assert any(status in x for x in verdict["reasons"]), status


@pytest.mark.parametrize("exit_code", [None, 1, 99, "0", False])
def test_a_passing_required_gate_requires_integer_exit_code_zero(exit_code: object) -> None:
    r = qualified_receipt()
    result = next(
        result for result in r["gates"]["results"] if result["id"] != receipt.MUTATION_GATE_ID
    )
    result["exit_code"] = exit_code
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("PASS without integer exit_code 0" in reason for reason in verdict["reasons"])


def test_a_passing_gate_cannot_report_interruption() -> None:
    r = qualified_receipt()
    result = next(
        result for result in r["gates"]["results"] if result["id"] != receipt.MUTATION_GATE_ID
    )
    result["interrupted_by_signal"] = 15
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("interrupted" in reason for reason in verdict["reasons"])


def test_an_optional_passing_gate_also_requires_integer_exit_code_zero() -> None:
    r = qualified_receipt()
    r["gates"]["results"].append({"id": "optional-evidence", "status": "PASS"})
    r["gates"].update(receipt.summarize_gate_results(r["gates"]["results"]))
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("optional-evidence" in reason for reason in verdict["reasons"])


def test_a_surviving_mutation_is_not_qualified() -> None:
    r = qualified_receipt()
    r["mutations"]["results"][0]["status"] = "SURVIVED"
    r["mutations"]["problems"] = 1
    r["mutations"]["status_counts"] = {"SURVIVED": 1}
    resign_summary(r, "curated")
    assert any("did not pass" in x for x in receipt.evaluate(r, CLEAN)["reasons"])


def test_a_not_run_msrv_gate_is_not_qualified() -> None:
    r = qualified_receipt()
    result = next(result for result in r["gates"]["results"] if result["id"] == "consumer-msrv")
    result["status"] = "NOT_RUN"
    r["gates"].update(receipt.summarize_gate_results(r["gates"]["results"]))
    assert "consumer-msrv" in r["gates"]["required_not_run"]
    assert any("NOT_RUN" in x for x in receipt.evaluate(r, CLEAN)["reasons"])


def test_stale_gate_summary_is_rejected_even_when_results_are_complete() -> None:
    r = qualified_receipt()
    r["gates"]["required_not_run"] = [receipt.MUTATION_GATE_ID]
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED", "a stale gate summary must fail closed"
    assert any(
        "required_not_run" in reason and "does not match" in reason for reason in verdict["reasons"]
    )


def test_duplicate_gate_ids_are_rejected() -> None:
    r = qualified_receipt()
    r["gates"]["results"].append(dict(r["gates"]["results"][0]))
    verdict = receipt.evaluate(r, CLEAN)
    assert any("duplicate result ids" in reason for reason in verdict["reasons"])


def test_mutation_summary_must_match_the_results() -> None:
    r = qualified_receipt()
    r["mutations"]["problems"] = 1
    resign_summary(r, "curated")
    verdict = receipt.evaluate(r, CLEAN)
    assert any("summary is not PASS" in reason for reason in verdict["reasons"])


def test_mutation_scope_must_match_the_results() -> None:
    r = qualified_receipt()
    r["mutations"]["status_counts"] = {"KILLED": 100}
    resign_summary(r, "curated")
    verdict = receipt.evaluate(r, CLEAN)
    assert any("status_counts drift" in reason for reason in verdict["reasons"])


def test_partial_mutation_receipt_cannot_qualify() -> None:
    r = qualified_receipt()
    r["mutations"]["selection"] = "subset"
    resign_summary(r, "curated")
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("not a full-inventory run" in reason for reason in verdict["reasons"])


def test_mutation_manifest_digest_is_bound_to_the_envelope() -> None:
    r = qualified_receipt()
    record = producer_record(r, "curated")
    record["envelope"]["payload"]["configs"][0]["sha256"] = "0" * 64
    record["envelope"]["payload_sha256"] = receipt.canonical_digest(record["envelope"]["payload"])
    gate = next(item for item in r["gates"]["results"] if item["id"] == receipt.MUTATION_GATE_ID)
    gate["evidence_digest"] = record["envelope"]["payload_sha256"]
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("registered manifest" in reason for reason in verdict["reasons"])


@pytest.mark.parametrize("field", ["source", "source_after", "gates"])
def test_malformed_receipt_objects_fail_closed_without_crashing(field: str) -> None:
    r = qualified_receipt()
    r[field] = []
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("not an object" in reason for reason in verdict["reasons"])


def test_attestation_requires_the_exact_collector_check_set() -> None:
    r = qualified_receipt()
    r["attestation"]["checks"] = {"forged": True}
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("attestation checks" in reason for reason in verdict["reasons"])


def test_derived_mutation_digest_must_match_the_embedded_evidence() -> None:
    r = qualified_receipt()
    mutation_gate = next(
        result for result in r["gates"]["results"] if result["id"] == receipt.MUTATION_GATE_ID
    )
    mutation_gate["evidence_digest"] = "forged"
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED", "mutated evidence digest was trusted"
    assert any("evidence linkage differs" in reason for reason in verdict["reasons"])


def test_missing_registered_producer_is_not_qualified() -> None:
    r = qualified_receipt()
    r["producers"] = r["producers"][:-1]
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("registration set differs" in reason for reason in verdict["reasons"])


@pytest.mark.parametrize("exit_code", [None, 1])
def test_producer_pass_requires_exact_exit_zero(exit_code: object) -> None:
    r = qualified_receipt()
    record = producer_record(r, "fuzz")
    record["envelope"]["payload"]["result"]["exit_code"] = exit_code
    record["envelope"]["payload_sha256"] = receipt.canonical_digest(record["envelope"]["payload"])
    gate = next(item for item in r["gates"]["results"] if item["id"] == "fuzz")
    gate["exit_code"] = exit_code
    gate["evidence_digest"] = record["envelope"]["payload_sha256"]
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("PASS requires exit_code 0" in reason for reason in verdict["reasons"])


@pytest.mark.parametrize(
    ("field", "value", "fragment"),
    [
        ("source", "wrong-sha", "expected source"),
        ("tool", "wrong-tool", "identity_sha256"),
        ("action", "wrong-job", "action job differs from registered producer job"),
        ("artifact", "wrong-artifact", "summary identity"),
    ],
)
def test_producer_identity_drift_is_rejected(field: str, value: str, fragment: str) -> None:
    r = qualified_receipt()
    record = producer_record(r, "fuzz")
    envelope = record["envelope"]["payload"]
    if field == "source":
        envelope["source"]["head"] = "9" * 40
    elif field == "tool":
        envelope["tools"][0]["identity_sha256"] = "9" * 64
    elif field == "action":
        envelope["action"]["job"] = value
    else:
        summary = next(item for item in envelope["artifacts"] if item["role"] == "summary")
        summary["sha256"] = "9" * 64
    record["envelope"]["payload_sha256"] = receipt.canonical_digest(envelope)
    gate = next(item for item in r["gates"]["results"] if item["id"] == "fuzz")
    gate["evidence_digest"] = record["envelope"]["payload_sha256"]
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any(fragment in reason for reason in verdict["reasons"]), verdict


@pytest.mark.parametrize("mode", ["zero", "partial"])
def test_fuzz_zero_or_partial_semantic_witnesses_are_rejected(mode: str) -> None:
    r = qualified_receipt()
    summary = producer_record(r, "fuzz")["summary"]["payload"]
    if mode == "zero":
        summary["targets"][0]["valid_inputs"] = 0
    else:
        summary["targets"][0]["semantic_checkpoints"].popitem()
    resign_summary(r, "fuzz")
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("fuzz target" in reason for reason in verdict["reasons"])


def test_duplicate_fuzz_target_cannot_overwrite_a_zero_witness_row() -> None:
    value = qualified_receipt()
    summary = producer_record(value, "fuzz")["summary"]["payload"]
    duplicate = deepcopy(summary["targets"][0])
    duplicate["valid_inputs"] = 0
    summary["targets"].insert(0, duplicate)
    resign_summary(value, "fuzz")
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("fuzz target rows" in reason for reason in verdict["reasons"])


@pytest.mark.parametrize("field", ["required_targets", "executed_targets"])
def test_duplicate_fuzz_declared_or_executed_target_is_not_qualified(field: str) -> None:
    value = qualified_receipt()
    summary = producer_record(value, "fuzz")["summary"]["payload"]
    summary[field].append(summary[field][0])
    resign_summary(value, "fuzz")
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("target set" in reason for reason in verdict["reasons"])


def test_model_partial_set_and_replay_drift_are_rejected() -> None:
    r = qualified_receipt()
    summary = producer_record(r, "modelcheck")["summary"]["payload"]
    summary["models"].pop()
    summary["replay"]["result"] = "different_failure"
    resign_summary(r, "modelcheck")
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("model summary set differs" in reason for reason in verdict["reasons"])
    assert any("replay identity/result differs" in reason for reason in verdict["reasons"])


def test_duplicate_model_cannot_overwrite_a_zero_exploration_row() -> None:
    value = qualified_receipt()
    summary = producer_record(value, "modelcheck")["summary"]["payload"]
    duplicate = deepcopy(summary["models"][0])
    duplicate["completed"] = 0
    summary["models"].insert(0, duplicate)
    resign_summary(value, "modelcheck")
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("model rows" in reason for reason in verdict["reasons"])


def test_duplicate_loom_manifest_model_id_is_not_qualified() -> None:
    value = qualified_receipt()
    manifest = producer_record(value, "modelcheck")["manifest"]["payload"]
    required = manifest["checkers"]["loom"]["required_models"]
    required.append(required[0])
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("manifest required models" in reason for reason in verdict["reasons"])


@pytest.mark.parametrize("field", ["checker", "replay_artifact"])
def test_model_checker_and_replay_artifact_must_match_envelope(field: str) -> None:
    value = qualified_receipt()
    summary = producer_record(value, "modelcheck")["summary"]["payload"]
    if field == "checker":
        summary["models"][0]["checker"] = "shuttle"
    else:
        summary["replay"]["artifact"]["sha256"] = "0" * 64
    resign_summary(value, "modelcheck")
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any(
        "checker identity" in reason or "replay artifact identity" in reason
        for reason in verdict["reasons"]
    )


@pytest.mark.parametrize("field", ["scheduler", "max_steps"])
def test_shuttle_scheduler_and_step_bound_match_manifest(field: str) -> None:
    value = qualified_receipt()
    summary = producer_record(value, "modelcheck")["summary"]["payload"]
    shuttle = next(item for item in summary["models"] if item["checker"] == "shuttle")
    shuttle[field] = "forged" if field == "scheduler" else 1
    resign_summary(value, "modelcheck")
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("fields differ from manifest" in reason for reason in verdict["reasons"])


@pytest.mark.parametrize(
    ("checker", "field", "bad"),
    [
        ("loom", "checker", "shuttle"),
        ("loom", "model_id", "loom.unknown"),
        ("loom", "completed", 0),
        ("loom", "max_threads", 0),
        ("loom", "max_branches", 0),
        ("loom", "max_permutations", "bounded"),
        ("loom", "preemption_bound", "bounded"),
        ("loom", "unregistered", 1),
        ("shuttle", "checker", "loom"),
        ("shuttle", "model_id", "shuttle.unknown"),
        ("shuttle", "seed", 0),
        ("shuttle", "requested", 0),
        ("shuttle", "completed", 0),
        ("shuttle", "scheduler", "exhaustive"),
        ("shuttle", "max_steps", 1),
        ("shuttle", "unregistered", 1),
    ],
)
def test_each_model_projection_field_is_authoritative(
    checker: str, field: str, bad: object
) -> None:
    value = qualified_receipt()
    summary = producer_record(value, "modelcheck")["summary"]["payload"]
    row = next(item for item in summary["models"] if item["checker"] == checker)
    row[field] = bad
    resign_summary(value, "modelcheck")
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED", (checker, field, verdict)


@pytest.mark.parametrize(
    "field",
    [
        "model_id",
        "seed",
        "result",
        "artifact_path",
        "artifact_sha256",
        "artifact_size",
        "artifact_role",
    ],
)
def test_each_replay_projection_field_is_authoritative(field: str) -> None:
    value = qualified_receipt()
    summary = producer_record(value, "modelcheck")["summary"]["payload"]
    replay = summary["replay"]
    if field == "artifact_path":
        replay["artifact"]["path"] = "target/modelcheck/elsewhere/schedule.txt"
    elif field == "artifact_sha256":
        replay["artifact"]["sha256"] = "0" * 64
    elif field == "artifact_size":
        replay["artifact"]["size"] = 2
    elif field == "artifact_role":
        replay["artifact"]["role"] = "raw"
    else:
        replay[field] = "forged"
    resign_summary(value, "modelcheck")
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED", (field, verdict)


@pytest.fixture
def repo(tmp_path: Path) -> Path:
    """A throwaway git repository. The digest tests used to write probe files
    into the live tree, which made `just gate` fail whenever anything else was
    touching the checkout at the same time (a concurrent pytest, a mutation
    run, an editor)."""
    root = tmp_path / "repo"
    root.mkdir()
    run = lambda *a: subprocess.run(  # noqa: E731
        ["git", *a], cwd=root, check=True, capture_output=True, text=True
    )
    run("init", "-q")
    run("config", "user.email", "t@example.com")
    run("config", "user.name", "t")
    (root / "src").mkdir()
    (root / "src" / "lib.rs").write_text("pub fn f() {}\n")
    (root / "docs" / "plans" / "sep-16-hardening" / "receipts").mkdir(parents=True)
    run("add", ".")
    run("commit", "-q", "-m", "init")
    return root


def test_source_identity_covers_untracked_files(repo: Path) -> None:
    """An untracked file changes the digest; HEAD alone would not notice it."""
    before = receipt.source_identity(repo)
    assert not before["dirty"]
    probe = repo / "src" / ".digest-probe"
    probe.write_text("probe\n", encoding="utf-8")
    after = receipt.source_identity(repo)
    assert before["paths_digest"] != after["paths_digest"]
    assert after["dirty"], "an untracked file is a dirty tree"
    assert after["head"] == before["head"], "HEAD alone would not have noticed"
    probe.unlink()
    assert receipt.source_identity(repo)["paths_digest"] == before["paths_digest"]


def test_a_content_change_to_a_tracked_file_changes_the_digest(repo: Path) -> None:
    before = receipt.source_identity(repo)["paths_digest"]
    (repo / "src" / "lib.rs").write_text("pub fn f() { let _ = 1; }\n")
    assert receipt.source_identity(repo)["paths_digest"] != before


def test_a_permission_mode_change_to_a_tracked_file_changes_the_digest(repo: Path) -> None:
    source = repo / "src" / "lib.rs"
    source.chmod(0o644)
    before = receipt.source_identity(repo)
    source.chmod(0o600)
    after = receipt.source_identity(repo)
    assert after["head"] == before["head"]
    assert after["dirty"] is False, "Git tracks the execute bit, not read/write permission bits"
    assert after["paths_digest"] != before["paths_digest"]


def test_real_v02_artifacts_share_receipt_source_identity_and_reject_byte_drift(
    repo: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Exercise the actual V02 envelope builder, disk collector, and receipt evaluator."""
    # This fixture is an explicit local producer rooted in a synthetic git
    # repository. A hosted outer pytest process must not rewrite its identity.
    monkeypatch.delenv("GITHUB_ACTIONS", raising=False)
    all_specs = receipt.registrations(receipt.INVENTORY)
    v02_specs = [spec for spec in all_specs if spec["surface"] in {"curated", "generated"}]
    fixture_inputs = {
        "tools/verification/mutation-gate.json",
        "tools/verification/mutations.json",
        ".github/workflows/ci.yml",
    }
    fixture_inputs.update(path for spec in v02_specs for path in spec["required_configs"])
    for relative in sorted(fixture_inputs):
        destination = repo / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        source = REPO / relative
        destination.write_bytes(source.read_bytes() if source.is_file() else b"name: ci\n")
    (repo / ".gitignore").write_text("/target/\n/receipt*.json\n", encoding="utf-8")
    subprocess.run(["git", "add", "."], cwd=repo, check=True)
    subprocess.run(["git", "commit", "-q", "-m", "v02-fixture"], cwd=repo, check=True)

    identity = receipt.source_identity(repo)
    campaign = v02_campaign.create_isolated_campaign(repo)
    try:
        assert campaign.source_before == identity["paths_digest"]
        source = {
            **identity,
            "isolated_checkout": True,
        }
        required = json.loads((REPO / "tools" / "gates" / "required.json").read_text())["required"]
        results = passing_gate_results(required)
        synthetic = _producer_records(source, results)
        by_surface = {record["surface"]: record for record in synthetic}
        for spec in v02_specs:
            summary = by_surface[spec["surface"]]["summary"]["payload"]
            summary_path = repo / spec["summary"]
            summary_path.parent.mkdir(parents=True, exist_ok=True)
            summary_path.write_text(json.dumps(summary), encoding="utf-8")
            raw_path = summary_path.parent / "raw" / "producer.log"
            raw_path.parent.mkdir()
            raw_path.write_text("producer completed\n", encoding="utf-8")
            envelope = v02_campaign.evidence_envelope(
                repo=repo,
                campaign=campaign,
                producer_id=spec["producer_id"],
                command=(
                    summary["command"]["argv"]
                    if spec["surface"] == "generated"
                    else ["just", spec["gate_id"]]
                ),
                environment={},
                tools=[
                    {
                        "name": "cargo",
                        "version": "cargo 1.95.0",
                        "identity_sha256": receipt.canonical_digest(
                            {"name": "cargo", "version": "cargo 1.95.0"}
                        ),
                    }
                ],
                config_paths=[repo / path for path in spec["required_configs"]],
                artifact_paths=[(raw_path, "raw"), (summary_path, "summary")],
                job="local-v02",
                status="PASS",
                exit_code=0,
                started_at="2026-09-21T00:00:00Z",
                finished_at="2026-09-21T00:01:00Z",
                selected_count=summary.get("total", summary.get("denominator")),
                executed_count=summary.get("total", summary.get("denominator")),
            )
            envelope["action"] = hosted_action(source, spec["hosted_job"])
            (repo / spec["envelope"]).write_text(json.dumps(envelope), encoding="utf-8")

        collected = receipt.collect_records(repo, v02_specs)
        collected_by_registration = {record["registration"]: record for record in collected}
        records = [
            collected_by_registration.get(record["registration"], record) for record in synthetic
        ]
        for record in records:
            gate = next(item for item in results if item["id"] == record["gate_id"])
            gate.update(
                {
                    "evidence_kind": "producer",
                    "producer_registration": record["registration"],
                    "evidence_digest": record["envelope"]["payload_sha256"],
                    "imported_producer": True,
                    "derived_from": record["envelope"]["identity"]["path"],
                    "producer_started_at": record["envelope"]["payload"]["result"]["started_at"],
                    "producer_finished_at": record["envelope"]["payload"]["result"]["finished_at"],
                }
            )
        summaries = {record["surface"]: record["summary"]["payload"] for record in records}
        value = {
            "schema_version": receipt.SCHEMA_VERSION,
            "generated_at": "2026-09-21T00:00:00Z",
            "finished_at": "2026-09-21T00:01:00Z",
            "environment": {},
            "gate_runner": hosted_gate_runner(results),
            "source": source,
            "source_after": {
                "head": identity["head"],
                "tree": identity["tree"],
                "paths_digest": identity["paths_digest"],
                "dirty": False,
            },
            "attestation": {
                "kind": "github-actions",
                "hosted_qualification_eligible": True,
                "head": identity["head"],
                "checks": {name: True for name in receipt.ATTESTATION_CHECK_NAMES},
                "action": hosted_action(source),
                "producer_jobs": {spec["hosted_job"]: "success" for spec in all_specs},
            },
            "gates": {
                "schema_version": receipt.GATE_SCHEMA_VERSION,
                **receipt.summarize_gate_results(results),
                "results": results,
            },
            "mutations": summaries["curated"],
            "generated_mutation_sweep": summaries["generated"],
            "producers": records,
        }
        current = {
            "head": identity["head"],
            "dirty": False,
            "paths_digest": identity["paths_digest"],
        }
        matching = receipt.evaluate(value, current)
        assert matching["status"] == "QUALIFIED", matching

        source_file = repo / "src" / "lib.rs"
        source_file.write_bytes(source_file.read_bytes() + b" ")
        drifted = receipt.source_identity(repo)
        verdict = receipt.evaluate(value, drifted)
        assert verdict["status"] == "NOT_QUALIFIED"
        assert any("source digest does not match" in reason for reason in verdict["reasons"])
    finally:
        campaign.cleanup()


def test_tracked_historic_receipts_are_in_both_source_digests(repo: Path) -> None:
    historic = repo / "docs" / "plans" / "sep-16-hardening" / "receipts" / "local.json"
    historic.write_text("{}\n", encoding="utf-8")
    subprocess.run(["git", "add", "."], cwd=repo, check=True)
    subprocess.run(["git", "commit", "-q", "-m", "historic-receipt"], cwd=repo, check=True)
    paths = v02_campaign.git_source_paths(repo)
    before = receipt.source_identity(repo)["paths_digest"]
    assert before == v02_campaign.tree_digest(repo, paths)
    historic.write_text('{"changed":true}\n', encoding="utf-8")
    after = receipt.source_identity(repo)["paths_digest"]
    assert after != before
    assert after == v02_campaign.tree_digest(repo, paths)


def test_build_output_is_not_part_of_the_source_identity(repo: Path) -> None:
    (repo / ".gitignore").write_text("/target/\n", encoding="utf-8")
    subprocess.run(["git", "add", ".gitignore"], cwd=repo, check=True)
    subprocess.run(["git", "commit", "-q", "-m", "ignore-target"], cwd=repo, check=True)
    before = receipt.source_identity(repo)["paths_digest"]
    (repo / "target").mkdir()
    (repo / "target" / "artifact.o").write_bytes(b"\x00")
    assert receipt.source_identity(repo)["paths_digest"] == before


def test_the_live_validator_rejects_a_forged_clean_receipt_on_a_dirty_tree(
    repo: Path, tmp_path: Path, monkeypatch
) -> None:
    # End to end through `validate`, in a throwaway repo made dirty on purpose.
    (repo / "src" / "scratch.rs").write_text("// uncommitted\n")
    identity = receipt.source_identity(repo)
    forged = qualified_receipt()
    forged["source"] = {**identity, "dirty": False}
    forged["source_after"] = {
        "head": identity["head"],
        "tree": identity["tree"],
        "paths_digest": identity["paths_digest"],
        "dirty": False,
    }
    path = tmp_path / "forged.json"
    path.write_text(json.dumps(forged))
    monkeypatch.setattr(receipt, "REPO", repo)
    code, reasons = receipt.validate(path, check_tree=True)
    assert code == 1
    assert any("claims a clean tree but the current working tree is dirty" in r for r in reasons)
    # The digest itself matched (it was copied from the real identity): the
    # dirtiness re-derivation is what caught it.
    assert not any("source digest" in r for r in reasons)


def test_receipt_files_written_at_the_repo_root_are_not_source(repo: Path) -> None:
    """CI collects with `--out receipt.json` at the repository root. The three
    files `collect` writes there must be outside both the identity digest and
    the dirtiness check, or the very run that produces the receipt reads its
    own output as an uncommitted change and declares itself NOT_QUALIFIED.
    The live `.gitignore` carries the rule; the fixture repo copies it."""
    live_ignore = (REPO / ".gitignore").read_text(encoding="utf-8")
    assert "/receipt*.json" in live_ignore, "the live .gitignore must ignore root receipts"
    (repo / ".gitignore").write_text("/receipt*.json\n")
    subprocess.run(["git", "add", ".gitignore"], cwd=repo, check=True)
    subprocess.run(
        [
            "git",
            "-c",
            "user.email=t@example.com",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "ignore",
        ],
        cwd=repo,
        check=True,
    )
    before = receipt.source_identity(repo)
    for name in ("receipt.json", "receipt.gates.json", "receipt.mutations.json"):
        (repo / name).write_text("{}\n")
    after = receipt.source_identity(repo)
    assert after["paths_digest"] == before["paths_digest"]
    assert not after["dirty"]


def test_collect_qualifies_on_a_clean_tree_with_receipts_written_at_the_root(
    repo: Path, monkeypatch
) -> None:
    """End to end through `collect` with gates and producers stubbed PASS."""
    (repo / ".gitignore").write_text("/receipt*.json\n")
    workflow = repo / ".github" / "workflows" / "ci.yml"
    workflow.parent.mkdir(parents=True)
    workflow.write_text("name: ci\n", encoding="utf-8")
    subprocess.run(["git", "add", ".gitignore", ".github/workflows/ci.yml"], cwd=repo, check=True)
    subprocess.run(
        [
            "git",
            "-c",
            "user.email=t@example.com",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "ignore",
        ],
        cwd=repo,
        check=True,
    )
    required = json.loads((REPO / "tools" / "gates" / "required.json").read_text())["required"]
    monkeypatch.setattr(receipt, "REPO", repo)
    identity = receipt.source_identity(repo)
    monkeypatch.setenv("GITHUB_ACTIONS", "true")
    monkeypatch.setenv("CI", "true")
    monkeypatch.setenv("GITHUB_SHA", identity["head"])
    monkeypatch.setenv("GITHUB_WORKSPACE", str(repo))
    monkeypatch.setenv(
        "GITHUB_WORKFLOW_REF", "taskmesh/taskmesh/.github/workflows/ci.yml@refs/heads/main"
    )
    monkeypatch.setenv("GITHUB_JOB", "qualification")
    monkeypatch.setenv("GITHUB_EVENT_NAME", "push")
    monkeypatch.setenv(
        "TASKMESH_PRODUCER_NEEDS_JSON",
        json.dumps(
            {
                spec["hosted_job"]: {"result": "success"}
                for spec in receipt.registrations(receipt.INVENTORY)
            }
        ),
    )

    results = passing_gate_results(required)
    producer_source = {
        "head": identity["head"],
        "paths_digest": identity["paths_digest"],
        "dirty": False,
    }
    specs = receipt.registrations(receipt.INVENTORY)
    producer_records = _producer_records(producer_source, results)
    collector_action = receipt.collection_attestation(producer_source, hosted_ci=True)["action"]
    for record in producer_records:
        record["envelope"]["payload"]["action"] = deepcopy(collector_action)
        spec = next(item for item in specs if item["registration"] == record["registration"])
        record["envelope"]["payload"]["action"]["job"] = spec["hosted_job"]
        record["envelope"]["payload_sha256"] = receipt.canonical_digest(
            record["envelope"]["payload"]
        )
        gate = next(item for item in results if item["id"] == record["gate_id"])
        gate["evidence_digest"] = record["envelope"]["payload_sha256"]
    monkeypatch.setattr(receipt, "registrations", lambda _path: specs)
    monkeypatch.setattr(receipt, "clear_registered_outputs", lambda _root, _specs: None)
    monkeypatch.setattr(
        receipt, "collect_records", lambda _root, _specs: deepcopy(producer_records)
    )

    calls: list[list[str]] = []
    producer_gate_ids = {spec["gate_id"] for spec in specs}

    def fake_run_json(
        cmd: list[str], receipt_path: Path, *, timeout_seconds: float
    ) -> receipt.JsonRun:
        calls.append(cmd)
        payload = {
            "results": [
                deepcopy(result) for result in results if result["id"] not in producer_gate_ids
            ]
        }
        exit_code = 1  # skipped producer gates make the raw runner NOT_QUALIFIED
        receipt_path.write_text(json.dumps(payload))
        return receipt.JsonRun(payload, exit_code, "2026-09-16T00:00:00+00:00", 0.1)

    monkeypatch.setattr(receipt, "run_json", fake_run_json)
    real_producer_problems = receipt.producer_records_problems
    collection_roots: list[Path | None] = []

    def fixture_producer_problems(*args, **kwargs):
        collection_roots.append(kwargs.get("root"))
        return real_producer_problems(*args, **{**kwargs, "root": None})

    monkeypatch.setattr(receipt, "producer_records_problems", fixture_producer_problems)
    out = repo / "receipt.json"
    code = receipt.collect(
        out,
        ["fast", "matrix", "nightly"],
        skip_mutations=False,
        hosted_ci=True,
        consume_producers=True,
    )
    written = json.loads(out.read_text())
    assert written["verdict"]["reasons"] == [], written["verdict"]
    assert written["verdict"]["status"] == "QUALIFIED"
    assert code == 0
    assert (repo / "receipt.gates.json").exists() and (repo / "receipt.mutations.json").exists()
    # The mutation gate result was derived from the sub-receipt, once.
    derived = [r for r in written["gates"]["results"] if r["id"] == receipt.MUTATION_GATE_ID]
    assert derived and derived[0]["status"] == "PASS" and derived[0]["exit_code"] == 0
    gates_sidecar = json.loads((repo / "receipt.gates.json").read_text())
    assert gates_sidecar == written["gates"]
    assert gates_sidecar["required_not_run"] == []
    assert gates_sidecar["qualified"] is True
    assert calls and calls[0].count("--skip") == len(producer_gate_ids)
    assert "--deadline-seconds" in calls[0]
    assert repo in collection_roots, "collect must validate artifacts against its live disk"
    assert not receipt.source_identity(repo)["dirty"]


def test_collect_rejects_a_corrupt_raw_artifact_before_writing_qualified_receipt(
    repo: Path, monkeypatch
) -> None:
    # The full collect fixture stubs producer records; this probe asserts the
    # collector uses disk-aware producer validation rather than only metadata.
    (repo / ".gitignore").write_text("/receipt*.json\n", encoding="utf-8")
    subprocess.run(["git", "add", ".gitignore"], cwd=repo, check=True)
    subprocess.run(["git", "commit", "-q", "-m", "ignore"], cwd=repo, check=True)
    monkeypatch.setattr(receipt, "REPO", repo)
    monkeypatch.setattr(receipt, "registrations", lambda _path: [])
    monkeypatch.setattr(receipt, "clear_registered_outputs", lambda *_args: None)
    monkeypatch.setattr(receipt, "collect_records", lambda *_args: [])
    monkeypatch.setattr(receipt, "environment", lambda: {})
    monkeypatch.setattr(
        receipt,
        "run_json",
        lambda _cmd, sidecar, **_kwargs: receipt.JsonRun(
            {"results": []}, 0, "2026-09-21T00:00:00Z", 0.0
        ),
    )

    def artifact_check(*_args, **kwargs):
        return ["artifact raw.log digest mismatch"] if kwargs.get("root") == repo else []

    monkeypatch.setattr(receipt, "producer_records_problems", artifact_check)
    out = repo / "receipt.json"
    assert receipt.collect(out, ["fast"], skip_mutations=True) == 1
    written = json.loads(out.read_text(encoding="utf-8"))
    assert any(
        "artifact raw.log digest mismatch" in reason for reason in written["verdict"]["reasons"]
    )


def test_local_collection_cannot_import_hosted_producer_artifacts(repo: Path) -> None:
    with pytest.raises(ValueError, match="only in hosted CI"):
        receipt.collect(
            repo / "receipt.json",
            ["fast"],
            skip_mutations=False,
            hosted_ci=False,
            consume_producers=True,
        )


def test_collect_cli_defaults_to_the_exact_required_set(tmp_path: Path, monkeypatch) -> None:
    captured: dict[str, object] = {}

    def fake_collect(out, tiers, skip_mutations, hosted_ci, consume_producers, local_qualified):
        captured.update(out=out, tiers=tiers, local_qualified=local_qualified)
        return 0

    monkeypatch.setattr(receipt, "collect", fake_collect)
    out = tmp_path / "receipt.json"
    assert receipt.main(["collect", "--local-qualified", "--out", str(out)]) == 0
    assert captured == {"out": out, "tiers": None, "local_qualified": True}


@pytest.mark.parametrize("mode", ["missing", "stale"])
def test_validate_rejects_missing_or_stale_combined_sidecars(
    tmp_path: Path, monkeypatch, mode: str
) -> None:
    value = qualified_receipt()
    value["verdict"] = receipt.evaluate(value, CLEAN)
    path = tmp_path / "receipt.json"
    path.write_text(json.dumps(value), encoding="utf-8")
    path.with_suffix(".gates.json").write_text(json.dumps(value["gates"]), encoding="utf-8")
    producer_sidecar = path.with_suffix(".producers.json")
    if mode == "stale":
        producer_sidecar.write_text("[]\n", encoding="utf-8")
    monkeypatch.setattr(receipt, "producer_records_problems", lambda *_args, **_kwargs: [])
    code, reasons = receipt.validate(path, check_tree=False)
    assert code == 1
    assert any("producers sidecar" in reason for reason in reasons)


@pytest.mark.parametrize("suffix", [".mutations.json", ".generated-mutations.json"])
@pytest.mark.parametrize("mode", ["missing", "stale"])
def test_validate_rejects_missing_or_stale_mutation_sidecars(
    tmp_path: Path, monkeypatch, suffix: str, mode: str
) -> None:
    value = qualified_receipt()
    value["verdict"] = receipt.evaluate(value, CLEAN)
    path = tmp_path / "receipt.json"
    path.write_text(json.dumps(value), encoding="utf-8")
    path.with_suffix(".gates.json").write_text(json.dumps(value["gates"]), encoding="utf-8")
    path.with_suffix(".producers.json").write_text(json.dumps(value["producers"]), encoding="utf-8")
    for current_suffix, field in (
        (".mutations.json", "mutations"),
        (".generated-mutations.json", "generated_mutation_sweep"),
    ):
        if current_suffix != suffix or mode == "stale":
            payload = [] if current_suffix == suffix else value[field]
            path.with_suffix(current_suffix).write_text(json.dumps(payload), encoding="utf-8")
    monkeypatch.setattr(receipt, "producer_records_problems", lambda *_args, **_kwargs: [])
    code, reasons = receipt.validate(path, check_tree=False)
    assert code == 1
    assert any("sidecar differs" in reason or "cannot read" in reason for reason in reasons)


@pytest.mark.parametrize("field", ["gates", "attestation"])
def test_validate_malformed_nested_receipt_fails_closed_without_exception(
    tmp_path: Path, field: str
) -> None:
    value = qualified_receipt()
    value[field] = ["not-an-object"]
    path = tmp_path / "receipt.json"
    path.write_text(json.dumps(value), encoding="utf-8")
    code, reasons = receipt.validate(path, check_tree=False)
    assert code == 1
    assert any("not an object" in reason for reason in reasons)


@pytest.mark.parametrize("field", ["artifacts", "producer", "configs", "action", "result"])
@pytest.mark.parametrize("bad", [None, "bad", [], {}])
def test_malformed_envelope_nested_json_is_not_qualified(field: str, bad: object) -> None:
    value = qualified_receipt()
    producer_record(value, "fuzz")["envelope"]["payload"][field] = bad
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert verdict["reasons"]


@pytest.mark.parametrize(
    ("surface", "field", "bad"),
    [
        ("fuzz", "required_targets", None),
        ("fuzz", "executed_targets", None),
        ("modelcheck", "models", [{"model_id": []}]),
        ("curated", "results", [{"mutation_id": [], "status": []}]),
        ("generated", "outcomes", None),
    ],
)
def test_malformed_producer_semantic_json_is_not_qualified(
    surface: str, field: str, bad: object
) -> None:
    value = qualified_receipt()
    producer_record(value, surface)["summary"]["payload"][field] = bad
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"


@pytest.mark.parametrize("field", ["identity", "payload"])
def test_malformed_record_shape_is_not_qualified(field: str) -> None:
    value = qualified_receipt()
    producer_record(value, "curated")["summary"][field] = None
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"


def test_malformed_nested_config_and_model_manifest_fail_closed() -> None:
    value = qualified_receipt()
    producer_record(value, "fuzz")["envelope"]["payload"]["configs"][0]["path"] = []
    producer_record(value, "modelcheck")["manifest"]["payload"]["checkers"] = None
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"


def test_malformed_gate_id_and_producer_surface_fail_closed() -> None:
    value = qualified_receipt()
    value["gates"]["results"][0]["id"] = []
    producer_record(value, "fuzz")["surface"] = []
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"


def test_validate_malformed_producer_envelope_returns_reasons(tmp_path: Path) -> None:
    value = qualified_receipt()
    producer_record(value, "fuzz")["envelope"]["payload"]["artifacts"] = None
    path = tmp_path / "receipt.json"
    path.write_text(json.dumps(value), encoding="utf-8")
    code, reasons = receipt.validate(path, check_tree=False)
    assert code == 1
    assert reasons


def test_valid_json_shape_mutation_matrix_never_raises_or_qualifies() -> None:
    baseline = qualified_receipt()
    bad_values = (None, [], {}, "invalid")
    envelope_fields = (
        "producer",
        "source",
        "command",
        "tools",
        "configs",
        "action",
        "artifacts",
        "result",
    )
    for surface in ("curated", "generated", "fuzz", "modelcheck"):
        for field in envelope_fields:
            for bad in bad_values:
                value = deepcopy(baseline)
                producer_record(value, surface)["envelope"]["payload"][field] = bad
                verdict = receipt.evaluate(value, CLEAN)
                assert verdict["status"] == "NOT_QUALIFIED", (surface, field, bad)
    for surface, part, field in (
        ("curated", "summary", "payload"),
        ("generated", "summary", "payload"),
        ("fuzz", "manifest", "payload"),
        ("modelcheck", "manifest", "payload"),
    ):
        for bad in bad_values:
            value = deepcopy(baseline)
            producer_record(value, surface)[part][field] = bad
            verdict = receipt.evaluate(value, CLEAN)
            assert verdict["status"] == "NOT_QUALIFIED", (surface, part, field, bad)


@pytest.mark.parametrize("result", ["failure", "cancelled", "skipped", None])
def test_hosted_prerequisite_job_must_have_succeeded(result: object) -> None:
    value = qualified_receipt()
    value["attestation"]["producer_jobs"]["fuzz"] = result
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("prerequisite jobs" in reason for reason in verdict["reasons"])


def test_attestation_reads_actual_prerequisite_job_results(monkeypatch) -> None:
    source = qualified_receipt()["source"]
    specs = receipt.registrations(receipt.INVENTORY)
    results = {spec["hosted_job"]: {"result": "success"} for spec in specs}
    monkeypatch.setenv("TASKMESH_PRODUCER_NEEDS_JSON", json.dumps(results))
    assert receipt.collection_attestation(source, hosted_ci=True)["checks"]["producer_jobs_success"]
    results["fuzz"]["result"] = "failure"
    monkeypatch.setenv("TASKMESH_PRODUCER_NEEDS_JSON", json.dumps(results))
    attestation = receipt.collection_attestation(source, hosted_ci=True)
    assert attestation["producer_jobs"]["fuzz"] == "failure"
    assert attestation["checks"]["producer_jobs_success"] is False


@pytest.mark.parametrize("field", ["imported_producer", "derived_from", "producer_started_at"])
def test_hosted_producer_gate_provenance_is_required(field: str) -> None:
    value = qualified_receipt()
    gate = next(item for item in value["gates"]["results"] if item["id"] == "fuzz")
    gate.pop(field)
    value["gates"].update(receipt.summarize_gate_results(value["gates"]["results"]))
    verdict = receipt.evaluate(value, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("hosted gate" in reason for reason in verdict["reasons"])


def test_collect_writes_not_qualified_for_malformed_producer_record(
    repo: Path, monkeypatch
) -> None:
    (repo / ".gitignore").write_text("/receipt*.json\n", encoding="utf-8")
    subprocess.run(["git", "add", ".gitignore"], cwd=repo, check=True)
    subprocess.run(["git", "commit", "-q", "-m", "ignore"], cwd=repo, check=True)
    monkeypatch.setattr(receipt, "REPO", repo)
    monkeypatch.setattr(receipt, "registrations", lambda _path: [])
    monkeypatch.setattr(receipt, "clear_registered_outputs", lambda *_args: None)
    monkeypatch.setattr(receipt, "environment", lambda: {})
    monkeypatch.setattr(
        receipt,
        "collect_records",
        lambda *_args: [
            {
                "registration": "malformed",
                "gate_id": "fuzz",
                "surface": "curated",
                "summary": None,
                "envelope": {"payload": {"result": None}},
            }
        ],
    )
    monkeypatch.setattr(
        receipt,
        "run_json",
        lambda _cmd, _sidecar, **_kwargs: receipt.JsonRun(
            {"results": []}, 1, "2026-09-21T00:00:00Z", 0.0
        ),
    )
    out = repo / "receipt.json"
    assert receipt.collect(out, ["fast"], skip_mutations=True) == 1
    assert json.loads(out.read_text())["verdict"]["status"] == "NOT_QUALIFIED"


def test_skip_mutations_really_skips_the_mutation_runner(repo: Path, monkeypatch) -> None:
    (repo / ".gitignore").write_text("/receipt*.json\n")
    subprocess.run(["git", "add", ".gitignore"], cwd=repo, check=True)
    subprocess.run(["git", "commit", "-q", "-m", "ignore"], cwd=repo, check=True)
    monkeypatch.setattr(receipt, "REPO", repo)
    calls: list[list[str]] = []

    def fake_run_json(
        cmd: list[str], receipt_path: Path, *, timeout_seconds: float
    ) -> receipt.JsonRun:
        calls.append(cmd)
        payload = {"results": []}
        receipt_path.write_text(json.dumps(payload))
        return receipt.JsonRun(payload, 1, "2026-09-16T00:00:00+00:00", 0.1)

    monkeypatch.setattr(receipt, "run_json", fake_run_json)
    code = receipt.collect(repo / "receipt.json", ["fast"], skip_mutations=True)
    assert code == 1
    assert len(calls) == 1 and "run_mutations.py" not in calls[0][1]
