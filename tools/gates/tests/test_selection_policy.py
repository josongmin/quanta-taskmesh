"""Mutation selection must not launch implicit work or erase prior evidence."""
from __future__ import annotations

import json
from pathlib import Path

import pytest
import yaml

from tools.gates import run, validate_inventory
from tools.gates.selection_policy import explicit_gate_ids, require_explicit_selection
from tools.qualification import receipt


@pytest.mark.parametrize("selector", [
    ["--all"], ["--required"], ["--profile", "release"],
    ["--profile", "nightly"], ["--tier", "nightly"],
])
def test_bulk_selection_rejects_before_source_or_gate_work(
    selector: list[str], monkeypatch: pytest.MonkeyPatch, tmp_path: Path,
) -> None:
    def forbidden(*args, **kwargs):
        pytest.fail("implicit mutation must reject before source work or gate launch")

    monkeypatch.setattr(run, "source_identity", forbidden)
    monkeypatch.setattr(run, "run_selected_gates", forbidden)
    output = tmp_path / "receipt.json"
    with pytest.raises(SystemExit) as exc:
        run.main([*selector, "--receipt", str(output)])
    assert exc.value.code == 2
    assert not output.exists()
    assert run.main([*selector, "--include-mutation", "--check-selection-only"]) == 0


@pytest.mark.parametrize("gate", ["mutants-critical", "mutants-generated"])
def test_named_mutation_gate_is_explicit_without_launching_a_campaign(gate: str) -> None:
    assert run.main(["--id", gate, "--check-selection-only"]) == 0


def test_ci_and_explicit_skips_do_not_require_mutation_consent() -> None:
    assert run.main(["--profile", "ci", "--check-selection-only"]) == 0
    assert run.main([
        "--required", "--skip", "mutants-critical", "--skip", "mutants-generated",
        "--check-selection-only",
    ]) == 0
    # Selection consent cannot turn a subset into release proof.
    required = set(json.loads(run.REQUIRED.read_text())["required"])
    missing, _ = run.summarize_required(required, [])
    assert {"mutants-critical", "mutants-generated"} <= set(missing)


def test_skipped_mutations_never_reach_execution_or_qualify_release(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path,
) -> None:
    source = {"head": "a" * 40, "tree": "b" * 40, "paths_digest": "c" * 64, "dirty": False}
    monkeypatch.setattr(run, "source_identity", lambda: source)
    executed: list[str] = []

    def execute(selected, *args, **kwargs):
        executed.extend(gate["id"] for gate in selected)
        return [{"id": gate["id"], "status": "PASS", "exit_code": 0} for gate in selected]

    monkeypatch.setattr(run, "run_selected_gates", execute)
    output = tmp_path / "receipt.json"
    assert run.main([
        "--required", "--skip", "mutants-critical", "--skip", "mutants-generated",
        "--receipt", str(output),
    ]) == 1
    protected = {"mutants-critical", "mutants-generated"}
    assert executed and not protected.intersection(executed)
    result = json.loads(output.read_text())
    assert result["qualified"] is False
    assert set(result["required_not_run"]) == protected


@pytest.mark.parametrize("value", [None, [], ["mutants-critical"] * 2, ["unknown"]])
def test_invalid_policy_fails_closed_even_with_opt_in(value: object) -> None:
    required = json.loads(run.REQUIRED.read_text())
    required["explicit_selection_required"] = value
    with pytest.raises(ValueError, match="explicit_selection_required"):
        require_explicit_selection([], required, include_mutation=True)


def test_required_policy_covers_both_mutation_producers() -> None:
    inventory = json.loads(run.INVENTORY.read_text())
    required = json.loads(run.REQUIRED.read_text())
    assert explicit_gate_ids(required) == {"mutants-critical", "mutants-generated"}
    changed = dict(required, explicit_selection_required=["mutants-critical"])
    problems = validate_inventory.validate(
        inventory, changed, validate_inventory.just_recipes(),
        validate_inventory.workflow_invocations(validate_inventory.WORKFLOWS),
        validate_inventory.gate_recipe_dependencies(),
    )
    assert "explicit_selection_required must cover exactly the mutation gates" in problems


def test_collector_rejects_before_clearing_evidence(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path,
) -> None:
    def forbidden(*args, **kwargs):
        pytest.fail("implicit mutation must not erase evidence or launch work")

    monkeypatch.setattr(receipt, "clear_registered_outputs", forbidden)
    monkeypatch.setattr(receipt, "source_identity", forbidden)
    monkeypatch.setattr(receipt, "run_json", forbidden)
    with pytest.raises(ValueError, match="require explicit selection"):
        receipt.collect(tmp_path / "receipt.json", None, skip_mutations=False)


@pytest.mark.parametrize("recipe", ["nightly", "release"])
def test_bulk_recipe_has_one_selection_and_execution_path(recipe: str) -> None:
    commands = [
        line.strip() for line in validate_inventory.recipe_body(recipe).splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]
    assert commands == [(
        f"uv run python tools/gates/run.py --profile {recipe} "
        "--require-clean-source {{ARGS}}"
    )]


def test_hosted_mutation_jobs_default_to_unselected() -> None:
    document = yaml.safe_load((run.REPO / ".github/workflows/ci.yml").read_text())
    trigger = document.get("on", document.get(True))
    option = trigger["workflow_dispatch"]["inputs"]["run_mutation"]
    assert option["type"] == "boolean" and option["default"] is False
    for name in ("mutation-gate", "mutation-generated"):
        assert document["jobs"][name]["if"] == "${{ inputs.run_mutation }}"
    assert validate_inventory.mutation_opt_in_problems(document) == []
    option["default"] = True
    del document["jobs"]["mutation-generated"]["if"]
    assert validate_inventory.mutation_opt_in_problems(document) == [
        "hosted mutation input must be boolean and default false",
        "mutation-generated: hosted mutation must require explicit selection",
    ]
