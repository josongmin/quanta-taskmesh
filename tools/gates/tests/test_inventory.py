"""Gate inventory negatives (H16-018-A01 / A05).

Each case takes the real inventory, applies one corruption, and checks the
validator names it. The inventory and the required set are separate inputs on
purpose: the last case shows why.
"""

from __future__ import annotations

import copy
import importlib.util
import json
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
VALIDATOR = REPO / "tools" / "gates" / "validate_inventory.py"

_spec = importlib.util.spec_from_file_location("validate_inventory", VALIDATOR)
assert _spec and _spec.loader
vi = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(vi)


def real_inputs():
    inventory = json.loads(vi.INVENTORY.read_text(encoding="utf-8"))
    required = json.loads(vi.REQUIRED.read_text(encoding="utf-8"))
    recipes = vi.just_recipes()
    invocations = vi.workflow_invocations(vi.WORKFLOWS)
    gate_deps = vi.gate_recipe_dependencies()
    return inventory, required, recipes, invocations, gate_deps


def problems_with(**overrides):
    inventory, required, recipes, invocations, gate_deps = real_inputs()
    inventory = copy.deepcopy(inventory)
    required = copy.deepcopy(required)
    invocations = {k: set(v) for k, v in invocations.items()}
    args = {
        "inventory": inventory,
        "required": required,
        "recipes": set(recipes),
        "invocations": invocations,
        "gate_deps": list(gate_deps),
    }
    args.update(overrides)
    return vi.validate(**args)


def test_the_committed_inventory_is_valid() -> None:
    assert problems_with() == []


def test_deleting_a_required_gate_from_the_inventory_is_reported() -> None:
    inventory, *_ = real_inputs()
    inventory = copy.deepcopy(inventory)
    inventory["gates"] = [g for g in inventory["gates"] if g["id"] != "semgrep"]
    problems = problems_with(inventory=inventory)
    assert any("required gate 'semgrep' is missing" in p for p in problems), problems


def test_an_unknown_required_id_is_reported() -> None:
    _, required, *_ = real_inputs()
    required = copy.deepcopy(required)
    required["required"].append("ghost-gate")
    problems = problems_with(required=required)
    assert any("ghost-gate" in p for p in problems), problems


def test_duplicate_ids_are_reported() -> None:
    inventory, *_ = real_inputs()
    inventory = copy.deepcopy(inventory)
    inventory["gates"].append(dict(inventory["gates"][0]))
    problems = problems_with(inventory=inventory)
    assert any("duplicate gate id" in p for p in problems), problems


def test_a_gate_whose_recipe_vanished_is_reported() -> None:
    inventory, _, recipes, *_ = real_inputs()
    recipes = set(recipes) - {"clippy"}
    problems = problems_with(recipes=recipes)
    assert any("clippy" in p and "not a Justfile recipe" in p for p in problems), problems


def test_ci_running_a_gate_the_inventory_does_not_know_is_reported() -> None:
    *_, invocations, _ = real_inputs()
    invocations = {k: set(v) for k, v in invocations.items()}
    invocations["secret-gate"] = {"ci.yml"}
    problems = problems_with(invocations=invocations)
    assert any("secret-gate" in p and "not in the inventory" in p for p in problems), problems


def test_a_ci_gate_no_workflow_invokes_is_reported() -> None:
    *_, invocations, _ = real_inputs()
    invocations = {k: set(v) for k, v in invocations.items()}
    invocations.pop("deny")
    problems = problems_with(invocations=invocations)
    assert any("deny" in p and "no workflow invokes" in p for p in problems), problems


def test_just_gate_drifting_from_the_fast_tier_is_reported() -> None:
    *_, gate_deps = real_inputs()
    gate_deps = [d for d in gate_deps if d != "bench-gate"]
    problems = problems_with(gate_deps=gate_deps)
    assert any("`just gate` chains" in p for p in problems), problems


def test_just_matrix_drifting_from_the_matrix_tier_is_reported() -> None:
    matrix = vi.recipe_dependencies("matrix")
    problems = problems_with(matrix_deps=[d for d in matrix if d != "doctest"])
    assert any("`just matrix` chains" in p for p in problems), problems


def test_just_proof_skipping_a_required_gate_is_reported() -> None:
    # The audit's gap: `proof` chained `gate` plus the heavy rails but not the
    # proof-matrix job, so a green local proof skipped four required gates.
    leaves = vi.expand_recipe("proof")
    problems = problems_with(proof_leaves=leaves - {"doctest", "bench-smoke"})
    assert any(
        "`just proof` skips required gate(s) ['bench-smoke', 'doctest']" in p for p in problems
    ), problems
    problems = problems_with(proof_leaves=leaves | {"bench"})
    assert any("`just proof` runs ['bench']" in p for p in problems), problems


def test_the_real_proof_recipe_expands_to_exactly_the_required_set() -> None:
    _, required, *_ = real_inputs()
    assert vi.expand_recipe("proof") == set(required["required"])


def test_recipe_expansion_follows_chains_and_stops_at_leaves() -> None:
    justfile = (
        "a: b c\n    @echo a\nb: d\n    @echo b\nc:\n    cargo test c\n"
        "d *ARGS:\n    bash d.sh {{ARGS}}\n"
    )
    assert vi.recipe_dependencies("a", justfile) == ["b", "c"]
    assert vi.recipe_dependencies("d", justfile) == [], "a body is not a dependency"
    assert vi.expand_recipe("a", justfile) == {"c", "d"}


def test_workflow_invocations_are_read_from_every_run_shape(tmp_path: Path) -> None:
    # The old validator saw only `run: just <word>` on one line. Everything CI
    # can actually write must count, or parity is a coincidence of style.
    (tmp_path / "w.yml").write_text(
        "jobs:\n"
        "  a:\n"
        "    steps:\n"
        "      - run: just fmt-check\n"
        "      - name: block\n"
        "        run: |\n"
        "          echo start\n"
        "          just secret-gate\n"
        "      - run: just other --verbose\n"
        "      - run: cd x && just third; just fourth\n"
        "      - run: (just fifth)\n"
        "      - run: echo not-a-recipe adjust nothing\n"
        "      - uses: some/action@v1\n"
    )
    found = vi.workflow_invocations(tmp_path)
    assert set(found) == {"fmt-check", "secret-gate", "other", "third", "fourth", "fifth"}
    assert found["secret-gate"] == {"w.yml"}


def test_a_gate_step_that_cannot_fail_the_job_is_reported(tmp_path: Path) -> None:
    """Presence of `just <gate>` is not enforcement. Every way a step can run a
    gate and stay green regardless is a parity hole."""
    (tmp_path / "w.yml").write_text(
        "jobs:\n"
        "  a:\n"
        "    steps:\n"
        "      - run: just loom || true\n"
        "      - run: just shuttle\n"
        "        continue-on-error: true\n"
        "      - run: just mutants-critical\n"
        "        if: false\n"
        "      - run: |\n"
        "          set +e\n"
        "          just deny\n"
        "          exit 0\n"
        "      - run: just fmt-check\n"
        "      - run: cargo build || true\n"
    )
    problems = vi.workflow_enforcement_problems(
        tmp_path, {"loom", "shuttle", "mutants-critical", "deny", "fmt-check"}
    )
    text = "\n".join(problems)
    assert "['loom']" in text and "masks failure" in text
    assert "['shuttle']" in text and "continue-on-error" in text
    assert "['mutants-critical']" in text and "conditional" in text
    assert "['deny']" in text
    assert "fmt-check" not in text, "a plain `run: just fmt-check` is enforced"
    assert "cargo build" not in text, "non-gate steps are not policed"
    # And the real workflows have none of these.
    _, _, _, _, gate_deps = real_inputs()
    inventory = json.loads(vi.INVENTORY.read_text(encoding="utf-8"))
    recipes = {g["recipe"] for g in inventory["gates"]}
    assert vi.workflow_enforcement_problems(vi.WORKFLOWS, recipes) == [], gate_deps


def test_required_and_inventory_are_different_files() -> None:
    """If one file generated both, deleting a gate would delete its requirement."""
    assert vi.INVENTORY != vi.REQUIRED
    assert vi.INVENTORY.is_file() and vi.REQUIRED.is_file()


def test_the_real_cli_passes() -> None:
    proc = subprocess.run(
        [sys.executable, str(VALIDATOR)], capture_output=True, text=True, check=False, cwd=REPO
    )
    assert proc.returncode == 0, proc.stderr
    assert "gate inventory OK" in proc.stdout


def test_runner_refuses_to_qualify_when_required_gates_did_not_run() -> None:
    """Running one gate must not produce a qualified receipt."""
    proc = subprocess.run(
        [sys.executable, str(REPO / "tools" / "gates" / "run.py"), "--id", "fmt-check"],
        capture_output=True,
        text=True,
        check=False,
        cwd=REPO,
    )
    assert proc.returncode == 1
    assert "NOT_QUALIFIED" in proc.stderr
    assert "required gates not run" in proc.stderr


def test_a_skipped_gate_is_absent_from_the_receipt_and_never_passes(tmp_path: Path) -> None:
    """`--skip` exists so the receipt collector can run the mutation gate once,
    through its own runner, and derive the inventory result from that. A
    skipped gate must simply be *absent* — recorded neither as PASS nor as
    anything else — so a receipt that forgets to fill it in is NOT qualified."""
    receipt_path = tmp_path / "gates.json"
    proc = subprocess.run(
        [
            sys.executable,
            str(REPO / "tools" / "gates" / "run.py"),
            "--id",
            "fmt-check",
            "--id",
            "pm-lint",
            "--skip",
            "pm-lint",
            "--receipt",
            str(receipt_path),
        ],
        capture_output=True,
        text=True,
        check=False,
        cwd=REPO,
    )
    assert proc.returncode == 1, proc.stderr
    receipt = json.loads(receipt_path.read_text())
    assert [r["id"] for r in receipt["results"]] == ["fmt-check"]
    assert "pm-lint" in receipt["required_not_run"]
    assert receipt["qualified"] is False


def test_an_unknown_skip_id_is_refused() -> None:
    proc = subprocess.run(
        [
            sys.executable,
            str(REPO / "tools" / "gates" / "run.py"),
            "--id",
            "fmt-check",
            "--skip",
            "nope",
        ],
        capture_output=True,
        text=True,
        check=False,
        cwd=REPO,
    )
    assert proc.returncode != 0
    assert "unknown gate id(s) in --skip" in proc.stderr


def test_a_self_reported_baseline_run_is_not_a_pass() -> None:
    """bench-iai exits 0 on BASELINE_CREATED (a baseline was written, nothing
    was compared). Through the gate runner that must be NOT_RUN, never PASS,
    or a receipt could be QUALIFIED with no instruction-count comparison."""
    run = importlib.util.spec_from_file_location("gates_run", REPO / "tools" / "gates" / "run.py")
    assert run and run.loader
    module = importlib.util.module_from_spec(run)
    run.loader.exec_module(module)
    gate = {
        "id": "bench-iai",
        "status_line": {"marker": "taskmesh-iai-gate", "require": "status=QUALIFIED"},
    }
    assert module.status_line_qualifies(
        gate, "taskmesh-iai-gate status=QUALIFIED fingerprint=abc\n"
    )
    assert not module.status_line_qualifies(
        gate, "taskmesh-iai-gate status=BASELINE_CREATED fingerprint=abc\n"
    )
    assert not module.status_line_qualifies(gate, "no marker line at all\n"), (
        "a recipe that stopped printing its verdict has not proved anything"
    )
    assert module.status_line_qualifies({"id": "plain"}, ""), (
        "gates without a status line keep exit-0 semantics"
    )
    inventory = json.loads(vi.INVENTORY.read_text(encoding="utf-8"))
    bench = next(g for g in inventory["gates"] if g["id"] == "bench-iai")
    assert bench["status_line"] == {"marker": "taskmesh-iai-gate", "require": "status=QUALIFIED"}
