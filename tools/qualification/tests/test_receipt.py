"""The receipt validator must fail closed (H16-022-A02)."""

from __future__ import annotations

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
    return {
        "schema_version": receipt.SCHEMA_VERSION,
        "generated_at": "2026-09-16T00:00:00+00:00",
        "finished_at": "2026-09-16T00:10:00+00:00",
        "environment": {"toolchain": "rustc 1.95.0"},
        "source": {"head": "x", "tree": "y", "dirty": False, "paths_digest": "abc"},
        "source_after": {"paths_digest": "abc", "dirty": False},
        "gates": {"results": [{"id": g, "status": "PASS"} for g in required]},
        "mutations": {"problems": 0},
        "consumer_msrv": {"exit_code": 0},
    }


CLEAN = {"head": "x", "dirty": False, "paths_digest": "abc"}


def test_a_complete_clean_receipt_qualifies() -> None:
    assert receipt.evaluate(qualified_receipt(), CLEAN)["status"] == "QUALIFIED"


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
    r["mutations"] = {"problems": 0, "dirty_tree": True}
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


def test_a_missing_required_gate_is_not_qualified() -> None:
    r = qualified_receipt()
    r["gates"]["results"] = [x for x in r["gates"]["results"] if x["id"] != "semgrep"]
    verdict = receipt.evaluate(r, CLEAN)
    assert any("semgrep was not run" in x for x in verdict["reasons"])


def test_a_skipped_or_failed_required_gate_is_not_qualified() -> None:
    for status in ("FAIL", "SKIPPED_PLATFORM", "NOT_RUN"):
        r = qualified_receipt()
        r["gates"]["results"][0]["status"] = status
        verdict = receipt.evaluate(r, CLEAN)
        assert verdict["status"] == "NOT_QUALIFIED", status
        assert any(status in x for x in verdict["reasons"]), status


def test_a_surviving_mutation_is_not_qualified() -> None:
    r = qualified_receipt()
    r["mutations"] = {"problems": 1}
    assert any("mutation gate" in x for x in receipt.evaluate(r, CLEAN)["reasons"])


def test_a_not_run_msrv_check_is_not_qualified() -> None:
    r = qualified_receipt()
    r["consumer_msrv"] = {"exit_code": 2}
    assert any("NOT_RUN" in x for x in receipt.evaluate(r, CLEAN)["reasons"])


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
