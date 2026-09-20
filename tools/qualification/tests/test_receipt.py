"""The receipt validator must fail closed (H16-022-A02)."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]
MODULE = REPO / "tools" / "qualification" / "receipt.py"
_spec = importlib.util.spec_from_file_location("receipt", MODULE)
assert _spec and _spec.loader
receipt = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(receipt)


def qualified_receipt() -> dict:
    required = json.loads((REPO / "tools" / "gates" / "required.json").read_text())["required"]
    inventory_path = REPO / "tools" / "verification" / "mutations.json"
    inventory = json.loads(inventory_path.read_text())["mutations"]
    mutation_results = [
        {
            "id": mutation["id"],
            "runner": mutation.get("runner", "cargo"),
            "file": mutation["file"],
            "control": bool(mutation.get("expect_no_failure")),
            "status": "CONTROL_GREEN" if mutation.get("expect_no_failure") else "KILLED",
        }
        for mutation in inventory
    ]
    mutations = {
        "schema_version": receipt.MUTATION_SCHEMA_VERSION,
        "kind": "curated-single-edit-inventory",
        "inventory_digest": hashlib.sha256(inventory_path.read_bytes()).hexdigest(),
        "inventory_total": len(inventory),
        "selection": "full",
        "total": len(inventory),
        "problems": 0,
        "status_counts": {"CONTROL_GREEN": 1, "KILLED": len(inventory) - 1},
        "scope": receipt.mutation_scope_from_results(mutation_results),
        "dirty_tree": False,
        "results": mutation_results,
    }
    results = [{"id": gate, "status": "PASS"} for gate in required]
    mutation_gate = next(result for result in results if result["id"] == receipt.MUTATION_GATE_ID)
    mutation_gate.update(
        {
            "exit_code": 0,
            "evidence_kind": "derived",
            "derived_from": "mutations",
            "evidence_digest": receipt.canonical_digest(mutations),
        }
    )
    return {
        "schema_version": receipt.SCHEMA_VERSION,
        "generated_at": "2026-09-16T00:00:00+00:00",
        "finished_at": "2026-09-16T00:10:00+00:00",
        "environment": {"toolchain": "rustc 1.95.0"},
        "source": {
            "head": "x",
            "tree": "y",
            "dirty": False,
            "isolated_checkout": True,
            "paths_digest": "abc",
        },
        "source_after": {"paths_digest": "abc", "dirty": False},
        "attestation": {
            "kind": "github-actions",
            "hosted_qualification_eligible": True,
            "head": "x",
            "checks": {name: True for name in receipt.ATTESTATION_CHECK_NAMES},
        },
        "gates": {
            "schema_version": receipt.GATE_SCHEMA_VERSION,
            **receipt.summarize_gate_results(results),
            "results": results,
        },
        "mutations": mutations,
        "generated_mutation_sweep": receipt.generated_mutation_disclosure(),
    }


CLEAN = {"head": "x", "dirty": False, "paths_digest": "abc"}


def test_a_complete_clean_receipt_qualifies() -> None:
    assert receipt.evaluate(qualified_receipt(), CLEAN)["status"] == "QUALIFIED"


def test_local_evidence_never_claims_hosted_qualification() -> None:
    r = qualified_receipt()
    r["attestation"] = receipt.collection_attestation(r["source"], hosted_ci=False)
    r["source"]["isolated_checkout"] = False
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("hosted qualification job" in reason for reason in verdict["reasons"])


def test_generated_mutation_sweep_is_explicitly_outside_qualification() -> None:
    r = qualified_receipt()
    assert r["generated_mutation_sweep"] == {
        "tool": "cargo-mutants",
        "status": "NOT_RUN",
        "included_in_qualification": False,
        "reason": "generated mutation sweep is a separate campaign, not a qualification gate",
    }
    assert receipt.evaluate(r, CLEAN)["status"] == "QUALIFIED"


def test_generated_mutation_sweep_cannot_claim_a_pass() -> None:
    r = qualified_receipt()
    r["generated_mutation_sweep"]["status"] = "PASS"
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED", "generated sweep PASS was trusted without evidence"


@pytest.mark.parametrize(
    ("key", "value"),
    [
        ("status", "PASS"),
        ("included_in_qualification", True),
        ("tool", "custom-runner"),
        ("reason", ""),
    ],
)
def test_generated_mutation_sweep_disclosure_fails_closed(key: str, value: object) -> None:
    r = qualified_receipt()
    r["generated_mutation_sweep"][key] = value
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("generated mutation sweep disclosure" in reason for reason in verdict["reasons"])


def test_run_json_never_reuses_a_stale_sidecar(tmp_path: Path, monkeypatch) -> None:
    sidecar = tmp_path / "stale.json"
    sidecar.write_text('{"qualified": true}\n', encoding="utf-8")
    monkeypatch.setattr(
        receipt.subprocess,
        "run",
        lambda *args, **kwargs: subprocess.CompletedProcess(
            args=args[0], returncode=1, stdout="", stderr="crashed"
        ),
    )
    run = receipt.run_json(["broken-runner"], sidecar)
    assert run.exit_code == 1
    assert run.payload["error"] == "runner did not write its receipt"
    assert "qualified" not in run.payload


def test_a_source_mismatch_is_not_qualified() -> None:
    verdict = receipt.evaluate(qualified_receipt(), {**CLEAN, "paths_digest": "different"})
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("source digest" in r for r in verdict["reasons"])


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
    r["mutations"]["dirty_tree"] = True
    verdict = receipt.evaluate(r, CLEAN)
    assert any("mutation receipt's view of the tree disagrees" in r for r in verdict["reasons"])


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


def test_a_surviving_mutation_is_not_qualified() -> None:
    r = qualified_receipt()
    r["mutations"]["results"][0]["status"] = "SURVIVED"
    r["mutations"]["problems"] = 1
    r["mutations"]["status_counts"] = {"SURVIVED": 1}
    assert any("mutation gate" in x for x in receipt.evaluate(r, CLEAN)["reasons"])


def test_a_not_run_msrv_gate_is_not_qualified() -> None:
    r = qualified_receipt()
    result = next(result for result in r["gates"]["results"] if result["id"] == "consumer-msrv")
    result["status"] = "NOT_RUN"
    r["gates"].update(receipt.summarize_gate_results(r["gates"]["results"]))
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
    verdict = receipt.evaluate(r, CLEAN)
    assert any("does not match 0 bad results" in reason for reason in verdict["reasons"])


def test_mutation_scope_must_match_the_results() -> None:
    r = qualified_receipt()
    r["mutations"]["scope"]["runner_counts"] = {"cargo": 100}
    verdict = receipt.evaluate(r, CLEAN)
    assert any("scope does not match results" in reason for reason in verdict["reasons"])


def test_partial_mutation_receipt_cannot_qualify() -> None:
    r = qualified_receipt()
    r["mutations"]["selection"] = "subset"
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("not a full-inventory run" in reason for reason in verdict["reasons"])


def test_mutation_inventory_digest_is_bound_to_the_source() -> None:
    r = qualified_receipt()
    r["mutations"]["inventory_digest"] = "0" * 64
    verdict = receipt.evaluate(r, CLEAN)
    assert verdict["status"] == "NOT_QUALIFIED"
    assert any("inventory digest" in reason for reason in verdict["reasons"])


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
    assert any("evidence digest does not match" in reason for reason in verdict["reasons"])


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


def test_receipts_themselves_are_not_part_of_the_source_identity(repo: Path) -> None:
    """Writing a receipt must not invalidate the collection that produced it."""
    before = receipt.source_identity(repo)["paths_digest"]
    probe = repo / "docs" / "plans" / "sep-16-hardening" / "receipts" / "local.json"
    probe.write_text("{}\n", encoding="utf-8")
    assert receipt.source_identity(repo)["paths_digest"] == before


def test_build_output_is_not_part_of_the_source_identity(repo: Path) -> None:
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
    forged["source_after"] = {"paths_digest": identity["paths_digest"], "dirty": False}
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
    """End to end through `collect`, with the gate runner, the mutation runner
    and the MSRV check stubbed to their passing shapes: the only thing that
    could make this NOT_QUALIFIED is the receipt output itself. Before the
    ignore rule that is exactly what happened on the first CI run."""
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
    required = json.loads((REPO / "tools" / "gates" / "required.json").read_text())["required"]
    monkeypatch.setattr(receipt, "REPO", repo)
    identity = receipt.source_identity(repo)
    monkeypatch.setenv("GITHUB_ACTIONS", "true")
    monkeypatch.setenv("CI", "true")
    monkeypatch.setenv("GITHUB_SHA", identity["head"])
    monkeypatch.setenv("GITHUB_WORKSPACE", str(repo))

    def fake_run_json(cmd: list[str], receipt_path: Path) -> receipt.JsonRun:
        if "run_mutations.py" in cmd[1]:
            payload = qualified_receipt()["mutations"]
            exit_code = 0
        else:
            payload = {
                "results": [
                    {"id": g, "status": "PASS"} for g in required if g != receipt.MUTATION_GATE_ID
                ]
            }
            exit_code = 1  # the generic runner correctly saw the omitted required mutation gate
        receipt_path.write_text(json.dumps(payload))
        return receipt.JsonRun(payload, exit_code, "2026-09-16T00:00:00+00:00", 0.1)

    monkeypatch.setattr(receipt, "run_json", fake_run_json)
    out = repo / "receipt.json"
    code = receipt.collect(out, ["fast", "matrix", "proof"], skip_mutations=False, hosted_ci=True)
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
    assert not receipt.source_identity(repo)["dirty"]


def test_skip_mutations_really_skips_the_mutation_runner(repo: Path, monkeypatch) -> None:
    (repo / ".gitignore").write_text("/receipt*.json\n")
    subprocess.run(["git", "add", ".gitignore"], cwd=repo, check=True)
    subprocess.run(["git", "commit", "-q", "-m", "ignore"], cwd=repo, check=True)
    monkeypatch.setattr(receipt, "REPO", repo)
    calls: list[list[str]] = []

    def fake_run_json(cmd: list[str], receipt_path: Path) -> receipt.JsonRun:
        calls.append(cmd)
        payload = {"results": []}
        receipt_path.write_text(json.dumps(payload))
        return receipt.JsonRun(payload, 1, "2026-09-16T00:00:00+00:00", 0.1)

    monkeypatch.setattr(receipt, "run_json", fake_run_json)
    code = receipt.collect(repo / "receipt.json", ["fast"], skip_mutations=True)
    assert code == 1
    assert len(calls) == 1 and "run_mutations.py" not in calls[0][1]
