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
from functools import cache
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]
VALIDATOR = REPO / "tools" / "gates" / "validate_inventory.py"

_spec = importlib.util.spec_from_file_location("validate_inventory", VALIDATOR)
assert _spec and _spec.loader
vi = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(vi)


@cache
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


def test_workflow_trust_requires_full_action_pins_and_read_default(tmp_path: Path) -> None:
    document = {
        "permissions": {"contents": "write"},
        "jobs": {"gate": {"steps": [{"uses": "actions/checkout@v4"}]}},
    }
    problems = vi.workflow_trust_problems(tmp_path / "ci.yml", document)
    assert any("top-level permissions" in problem for problem in problems)
    assert any("mutable action ref" in problem for problem in problems)


def test_workflow_trust_rejects_pr_reachable_write_or_token(tmp_path: Path) -> None:
    document = {
        "permissions": {"contents": "read"},
        "jobs": {
            "trend": {
                "permissions": {"contents": "write"},
                "steps": [
                    {"uses": "example/action@" + "1" * 40, "with": {"github-token": "secret"}}
                ],
            }
        },
    }
    problems = vi.workflow_trust_problems(tmp_path / "bench.yml", document)
    assert any("write permissions" in problem for problem in problems)
    assert any("PR-reachable action receives github-token" in problem for problem in problems)


def test_workflow_trust_allows_trusted_main_write_job(tmp_path: Path) -> None:
    document = {
        "permissions": {"contents": "read"},
        "jobs": {
            "publish": {
                "if": "github.event_name == 'push' && github.ref == 'refs/heads/main'",
                "permissions": {"contents": "write"},
                "steps": [
                    {"uses": "example/action@" + "1" * 40, "with": {"github-token": "secret"}}
                ],
            }
        },
    }
    assert vi.workflow_trust_problems(tmp_path / "bench.yml", document) == []


def test_committed_workflows_are_manual_only() -> None:
    assert vi.all_workflow_trigger_problems(vi.WORKFLOWS) == []


def test_automatic_hosted_triggers_are_rejected(tmp_path: Path) -> None:
    path = tmp_path / "ci.yml"
    assert vi.workflow_trigger_problems(path, {"on": {"workflow_dispatch": None}, "jobs": {}}) == []

    for trigger in ("push", "pull_request", "schedule"):
        problems = vi.workflow_trigger_problems(
            path, {"on": {"workflow_dispatch": None, trigger: None}, "jobs": {}}
        )
        assert any(trigger in problem for problem in problems), problems


def test_trigger_guard_handles_pyyaml_boolean_on_key(tmp_path: Path) -> None:
    path = tmp_path / "ci.yml"
    document = vi.yaml.safe_load("on:\n  workflow_dispatch:\n")
    assert True in document
    assert vi.workflow_trigger_problems(path, document) == []


def test_yaml_extension_cannot_bypass_hosted_trigger_guard(tmp_path: Path) -> None:
    (tmp_path / "bypass.yaml").write_text("on:\n  push:\njobs: {}\n", encoding="utf-8")
    problems = vi.all_workflow_trigger_problems(tmp_path)
    assert any("bypass.yaml" in problem and "push" in problem for problem in problems), problems


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


def test_v02_v03_producers_are_registered_once_with_complete_identity() -> None:
    inventory, required, *_ = real_inputs()
    producers = [
        (gate["id"], gate["producer"]) for gate in inventory["gates"] if "producer" in gate
    ]
    assert {producer["registration"] for _, producer in producers} == {
        "mutation-campaign-curated-v1",
        "mutation-campaign-generated-v1",
        "taskmesh-fuzz-v03",
        "taskmesh-modelcheck-v03",
    }
    assert len(producers) == 4
    assert all(set(producer) == vi.PRODUCER_FIELDS for _, producer in producers)
    assert required["required_producers"] == {
        producer["registration"]: gate_id for gate_id, producer in producers
    }


def test_missing_or_duplicate_producer_registration_is_reported() -> None:
    inventory, *_ = real_inputs()
    inventory = copy.deepcopy(inventory)
    producers = [gate for gate in inventory["gates"] if "producer" in gate]
    del producers[0]["producer"]["manifest"]
    producers[1]["producer"]["registration"] = producers[0]["producer"]["registration"]
    problems = problems_with(inventory=inventory)
    assert any("misses fields ['manifest']" in problem for problem in problems)
    assert any("duplicate producer registration" in problem for problem in problems)


def test_producer_artifacts_upload_even_when_the_gate_fails() -> None:
    workflow = vi.yaml.safe_load((vi.WORKFLOWS / "ci.yml").read_text(encoding="utf-8"))
    expected = {
        "mutation-gate": ("target/sep21/v02/curated",),
        "mutation-generated": ("target/sep21/v02/generated",),
        "fuzz": ("target/sep21/v03/fuzz/local",),
        "modelcheck": ("target/modelcheck",),
        "qualification": ("receipt.producers.json",),
    }
    for job_name, required_paths in expected.items():
        uploads = [
            step
            for step in workflow["jobs"][job_name]["steps"]
            if isinstance(step, dict)
            and str(step.get("uses", "")).startswith("actions/upload-artifact@")
        ]
        assert uploads, job_name
        assert any(step.get("if") == "always()" for step in uploads), job_name
        paths = "\n".join(str(step.get("with", {}).get("path", "")) for step in uploads)
        assert all(path in paths for path in required_paths), (job_name, paths)


def test_qualification_consumes_authorized_producer_jobs_with_bounded_critical_path(
    tmp_path: Path,
) -> None:
    inventory, *_ = real_inputs()
    workflow = vi.WORKFLOWS / "ci.yml"
    assert vi.producer_handoff_problems(inventory, workflow) == []
    assert inventory["qualification_budget_seconds"] == 18000

    over_budget = copy.deepcopy(inventory)
    over_budget["qualification_budget_seconds"] = 21600
    assert any(
        "critical-path deadline" in problem
        for problem in vi.producer_handoff_problems(over_budget, workflow)
    )

    broken = tmp_path / "ci.yml"
    broken.write_text(
        workflow.read_text(encoding="utf-8")
        .replace(
            "needs: [mutation-gate, mutation-generated, fuzz, modelcheck]",
            "needs: [mutation-gate]",
        )
        .replace("--consume-producers", "--rerun-producers"),
        encoding="utf-8",
    )
    problems = vi.producer_handoff_problems(inventory, broken)
    assert any("needs misses producer jobs" in problem for problem in problems)
    assert any("must consume prerequisite producer artifacts" in problem for problem in problems)

    failure_path = tmp_path / "failure-path.yml"
    failure_path.write_text(
        workflow.read_text(encoding="utf-8")
        .replace(
            "needs: [mutation-gate, mutation-generated, fuzz, modelcheck]\n    if: always()",
            "needs: [mutation-gate, mutation-generated, fuzz, modelcheck]",
        )
        .replace("continue-on-error: true", "continue-on-error: false")
        .replace(
            "name: modelcheck-evidence\n          path: target/modelcheck",
            "name: modelcheck-evidence\n          path: .",
        ),
        encoding="utf-8",
    )
    problems = vi.producer_handoff_problems(inventory, failure_path)
    assert any("must run under if: always()" in problem for problem in problems)
    assert any("would skip receipt" in problem for problem in problems)
    assert any("does not reconstruct" in problem for problem in problems)

    serialized = copy.deepcopy(inventory)
    generated = next(gate for gate in serialized["gates"] if gate["id"] == "mutants-generated")
    generated["producer"]["hosted_job"] = "mutation-gate"
    assert any(
        "serial producer gate timeouts exceed" in problem
        for problem in vi.producer_handoff_problems(serialized, workflow)
    )

    collector_drift = tmp_path / "collector-drift.yml"
    collector_drift.write_text(
        workflow.read_text(encoding="utf-8").replace(
            "name: Collect the receipt\n        if: always()", "name: Collect the receipt"
        ),
        encoding="utf-8",
    )
    collector_problems = vi.producer_handoff_problems(inventory, collector_drift)
    assert any("collector step must run under if: always()" in p for p in collector_problems)

    needs_drift = tmp_path / "needs-drift.yml"
    needs_drift.write_text(
        workflow.read_text(encoding="utf-8").replace(
            "TASKMESH_PRODUCER_NEEDS_JSON: ${{ toJSON(needs) }}",
            "TASKMESH_PRODUCER_NEEDS_JSON: '{}'",
        ),
        encoding="utf-8",
    )
    collector_problems = vi.producer_handoff_problems(inventory, needs_drift)
    assert any("bind actual prerequisite job results" in p for p in collector_problems)

    malformed_env = tmp_path / "malformed-env.yml"
    malformed_env.write_text(
        workflow.read_text(encoding="utf-8").replace(
            "env:\n          TASKMESH_PRODUCER_NEEDS_JSON: ${{ toJSON(needs) }}",
            "env: []",
        ),
        encoding="utf-8",
    )
    assert any(
        "bind actual prerequisite job results" in p
        for p in vi.producer_handoff_problems(inventory, malformed_env)
    )


def test_tracked_pre_push_hook_requires_exact_local_receipt() -> None:
    hook = REPO / ".githooks" / "pre-push"
    text = hook.read_text(encoding="utf-8")
    assert hook.stat().st_mode & 0o111
    assert "--validate-receipt" in text
    assert "--expected-head" in text
    assert "target/verification/macos-gates.json" in text
    assert "target/qualification/local-receipt.json" in text
    assert "tools/qualification/receipt.py" in text


def test_runner_refuses_to_qualify_when_required_gates_did_not_run() -> None:
    """Running one gate must not produce a qualified receipt."""
    proc = subprocess.run(
        [
            sys.executable,
            str(REPO / "tools" / "gates" / "run.py"),
            "--id",
            "test-architecture",
        ],
        capture_output=True,
        text=True,
        check=False,
        cwd=REPO,
    )
    assert proc.returncode == 1
    assert "NOT_QUALIFIED" in proc.stderr
    assert "required gates not run" in proc.stderr


def test_runner_fail_fast_keeps_the_full_selected_denominator() -> None:
    run = importlib.util.spec_from_file_location("gates_run", REPO / "tools" / "gates" / "run.py")
    assert run and run.loader
    module = importlib.util.module_from_spec(run)
    run.loader.exec_module(module)
    selected = [
        {"id": "first", "recipe": "first", "platforms": ["any"]},
        {"id": "second", "recipe": "second", "platforms": ["any"]},
        {"id": "linux-only", "recipe": "linux-only", "platforms": ["linux"]},
    ]
    calls: list[str] = []

    def fail_first(gate: dict) -> dict:
        calls.append(gate["id"])
        return {
            "id": gate["id"],
            "recipe": gate["recipe"],
            "status": "FAIL",
            "duration_s": 0.0,
        }

    results = module.run_selected_gates(selected, "macos", None, False, fail_first)
    assert calls == ["first"]
    assert [result["id"] for result in results] == ["first", "second", "linux-only"]
    assert results[1]["status"] == "NOT_RUN"
    assert results[1]["blocked_by"] == "first"
    assert results[2]["status"] == "SKIPPED_PLATFORM"
    not_run, not_passed = module.summarize_required({"first", "second", "linux-only"}, results)
    assert not_run == ["second"]
    assert not_passed == ["first", "linux-only", "second"]


def test_runner_keep_going_executes_later_gates() -> None:
    run = importlib.util.spec_from_file_location("gates_run", REPO / "tools" / "gates" / "run.py")
    assert run and run.loader
    module = importlib.util.module_from_spec(run)
    run.loader.exec_module(module)
    selected = [
        {"id": "first", "recipe": "first", "platforms": ["any"]},
        {"id": "second", "recipe": "second", "platforms": ["any"]},
    ]
    calls: list[str] = []

    def run_all(gate: dict) -> dict:
        calls.append(gate["id"])
        return {
            "id": gate["id"],
            "recipe": gate["recipe"],
            "status": "FAIL" if gate["id"] == "first" else "PASS",
            "duration_s": 0.0,
        }

    results = module.run_selected_gates(selected, "macos", None, True, run_all)
    assert calls == ["first", "second"]
    assert [result["status"] for result in results] == ["FAIL", "PASS"]


def test_exhausted_global_deadline_has_one_fail_and_explicit_not_run_tail(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    run = importlib.util.spec_from_file_location("gates_run", REPO / "tools" / "gates" / "run.py")
    assert run and run.loader
    module = importlib.util.module_from_spec(run)
    run.loader.exec_module(module)
    selected = [
        {"id": "first", "recipe": "first", "platforms": ["any"]},
        {"id": "second", "recipe": "second", "platforms": ["any"]},
    ]
    monkeypatch.setattr(module.time, "monotonic", lambda: 10.0)

    results = module.run_selected_gates(selected, "macos", 9.0, True)

    assert [result["status"] for result in results] == ["FAIL", "NOT_RUN"]
    assert results[1]["blocked_by"] == "first"


def test_dirty_source_preflight_starts_no_gate() -> None:
    run = importlib.util.spec_from_file_location("gates_run", REPO / "tools" / "gates" / "run.py")
    assert run and run.loader
    module = importlib.util.module_from_spec(run)
    run.loader.exec_module(module)
    selected = [
        {"id": "first", "recipe": "first", "platforms": ["any"]},
        {"id": "second", "recipe": "second", "platforms": ["any"]},
    ]
    calls: list[str] = []

    def must_not_run(gate: dict) -> dict:
        calls.append(gate["id"])
        raise AssertionError("dirty qualification must not start a gate")

    results = module.run_selected_gates(
        selected,
        "macos",
        None,
        False,
        must_not_run,
        preflight_blocker="dirty-source-preflight",
    )
    assert calls == []
    assert [result["status"] for result in results] == ["NOT_RUN", "NOT_RUN"]
    assert {result["blocked_by"] for result in results} == {"dirty-source-preflight"}


def test_dirty_source_preflight_is_explicit_not_implied_by_full_selection() -> None:
    run = importlib.util.spec_from_file_location("gates_run", REPO / "tools" / "gates" / "run.py")
    assert run and run.loader
    module = importlib.util.module_from_spec(run)
    run.loader.exec_module(module)
    dirty = {"dirty": True}

    assert module.source_preflight_blocker(dirty, require_clean_source=False) is None
    assert (
        module.source_preflight_blocker(dirty, require_clean_source=True)
        == "dirty-source-preflight"
    )


def test_platform_scope_requires_every_applicable_gate_and_keeps_exclusions() -> None:
    run = importlib.util.spec_from_file_location("gates_run", REPO / "tools" / "gates" / "run.py")
    assert run and run.loader
    module = importlib.util.module_from_spec(run)
    run.loader.exec_module(module)

    required = {"fmt-check", "bench-iai"}
    platform_conditional = {"bench-iai": "linux"}
    scoped, excluded = module.platform_scope_qualification(
        required,
        [
            {"id": "fmt-check", "status": "PASS", "exit_code": 0},
            {"id": "bench-iai", "status": "SKIPPED_PLATFORM", "exit_code": None},
        ],
        platform_conditional,
        "macos",
    )
    assert scoped is True
    assert excluded == ["bench-iai"]

    scoped, _ = module.platform_scope_qualification(
        required,
        [
            {"id": "fmt-check", "status": "PASS", "exit_code": 0},
            {"id": "bench-iai", "status": "NOT_RUN", "exit_code": 2},
        ],
        platform_conditional,
        "macos",
    )
    assert scoped is False


def test_local_receipt_is_exact_source_bound() -> None:
    run = importlib.util.spec_from_file_location("gates_run", REPO / "tools" / "gates" / "run.py")
    assert run and run.loader
    module = importlib.util.module_from_spec(run)
    run.loader.exec_module(module)

    source = {
        "head": "a" * 40,
        "tree": "b" * 40,
        "paths_digest": "c" * 64,
        "dirty": False,
    }
    receipt = {
        "schema_version": 2,
        "platform": "macos",
        "source": dict(source),
        "source_after": dict(source),
        "source_problems": [],
        "platform_scope": {
            "qualified": True,
            "excluded_required_gates": ["bench-iai"],
        },
        "results": [
            {"id": "fmt-check", "status": "PASS", "exit_code": 0},
            {"id": "bench-iai", "status": "SKIPPED_PLATFORM", "exit_code": None},
        ],
    }
    required = {"fmt-check", "bench-iai"}
    conditional = {"bench-iai": "linux"}
    assert (
        module.local_receipt_problems(
            receipt, source, required, conditional, "macos", source["head"]
        )
        == []
    )

    drifted = {**source, "paths_digest": "d" * 64}
    problems = module.local_receipt_problems(
        receipt, drifted, required, conditional, "macos", source["head"]
    )
    assert any("paths_digest differs" in problem for problem in problems), problems

    dirty = {**source, "dirty": True}
    problems = module.local_receipt_problems(
        receipt, dirty, required, conditional, "macos", source["head"]
    )
    assert any("must both be clean" in problem for problem in problems), problems


@pytest.mark.parametrize("exit_code", [1, False, None, "0"])
def test_local_receipt_pass_requires_exact_integer_zero(exit_code: object) -> None:
    run = importlib.util.spec_from_file_location("gates_run", REPO / "tools" / "gates" / "run.py")
    assert run and run.loader
    module = importlib.util.module_from_spec(run)
    run.loader.exec_module(module)
    result = {"id": "fmt-check", "status": "PASS", "exit_code": exit_code}

    assert module.platform_scope_qualification({"fmt-check"}, [result], {}, "macos")[0] is False
    assert module.result_record_problems([result]) == [
        "local receipt gate fmt-check PASS requires integer exit_code 0"
    ]


@pytest.mark.parametrize(
    ("status", "exit_code"),
    [("SKIPPED_PLATFORM", 0), ("NOT_RUN", 0), ("NOT_RUN", False), ("FAIL", 0), ("FAIL", True)],
)
def test_local_receipt_non_pass_exit_contract_is_fail_closed(
    status: str, exit_code: object
) -> None:
    run = importlib.util.spec_from_file_location("gates_run", REPO / "tools" / "gates" / "run.py")
    assert run and run.loader
    module = importlib.util.module_from_spec(run)
    run.loader.exec_module(module)
    assert module.result_record_problems([{"id": "gate", "status": status, "exit_code": exit_code}])


def test_local_source_must_stay_clean_and_unchanged() -> None:
    run = importlib.util.spec_from_file_location("gates_run", REPO / "tools" / "gates" / "run.py")
    assert run and run.loader
    module = importlib.util.module_from_spec(run)
    run.loader.exec_module(module)
    source = {
        "head": "a" * 40,
        "tree": "b" * 40,
        "paths_digest": "c" * 64,
        "dirty": False,
    }
    assert module.source_stability_problems(source, dict(source)) == []
    problems = module.source_stability_problems(source, {**source, "tree": "d" * 40, "dirty": True})
    assert any("dirty after" in problem for problem in problems)
    assert any("tree changed" in problem for problem in problems)


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
            "test-architecture",
            "--id",
            "fmt-check",
            "--skip",
            "fmt-check",
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
    assert [r["id"] for r in receipt["results"]] == ["test-architecture"]
    assert "fmt-check" in receipt["required_not_run"]
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


def _real_self_reports() -> dict[str, set[str]]:
    inventory = json.loads(vi.INVENTORY.read_text(encoding="utf-8"))
    return {gate["recipe"]: vi.self_report_markers(gate["recipe"]) for gate in inventory["gates"]}


def test_self_report_markers_are_read_from_the_scripts_a_recipe_runs(tmp_path: Path) -> None:
    """The marker set comes from the *sources* of the scripts named in the
    recipe body — not from a hand-kept list — so a script that starts (or
    stops) self-reporting moves the inventory check with it."""
    (tmp_path / "tools").mkdir()
    (tmp_path / "tools" / "probe.sh").write_text(
        'echo "taskmesh-probe status=CLEAN target=x"\n', encoding="utf-8"
    )
    (tmp_path / "tools" / "quiet.py").write_text("print('done')\n", encoding="utf-8")
    justfile = (
        "probe:\n    bash tools/probe.sh\n\n"
        "quiet:\n    python3 tools/quiet.py\n\n"
        "direct:\n    cargo test --workspace\n"
    )
    assert vi.self_report_markers("probe", justfile, tmp_path) == {"taskmesh-probe"}
    assert vi.self_report_markers("quiet", justfile, tmp_path) == set()
    assert vi.self_report_markers("direct", justfile, tmp_path) == set()


def test_every_self_reporting_recipe_declares_the_status_line_it_prints() -> None:
    """Both directions, on the committed inventory. A self-reporting recipe
    without a `status_line` is the bench-iai defect from round 3B generalised:
    exit 0 alone would be PASS and the receipt's bounded output tail would drop
    the verdict line (it did drop the coverage percentages and the TSan target
    once). A `status_line` whose marker no script prints makes every run
    NOT_RUN — loud, but a lie about which script is the evidence."""
    self_reports = _real_self_reports()
    assert self_reports["tsan"] == {"taskmesh-tsan"}
    assert self_reports["coverage-report"] == {"taskmesh-coverage"}
    assert self_reports["consumer-msrv"] == {"taskmesh-consumer-msrv"}
    assert self_reports["bench-iai"] == {"taskmesh-iai-gate"}
    assert problems_with(self_reports=self_reports) == []

    inventory, *_ = real_inputs()
    inventory = copy.deepcopy(inventory)
    tsan = next(g for g in inventory["gates"] if g["id"] == "tsan")
    del tsan["status_line"]
    problems = problems_with(inventory=inventory, self_reports=self_reports)
    assert any("tsan" in p and "self-reports" in p for p in problems), (
        f"a self-reporting recipe without a status_line must be reported: {problems}"
    )

    inventory, *_ = real_inputs()
    inventory = copy.deepcopy(inventory)
    cov = next(g for g in inventory["gates"] if g["id"] == "coverage-report")
    cov["status_line"] = {"marker": "taskmesh-covrage", "require": "status=REPORTED"}
    problems = problems_with(inventory=inventory, self_reports=self_reports)
    assert any("coverage-report" in p and "not printed" in p for p in problems), (
        f"a marker no script prints must be reported: {problems}"
    )

    inventory, *_ = real_inputs()
    inventory = copy.deepcopy(inventory)
    cov = next(g for g in inventory["gates"] if g["id"] == "coverage-report")
    cov["status_line"] = {"marker": "taskmesh-coverage"}
    problems = problems_with(inventory=inventory, self_reports=self_reports)
    assert any("coverage-report" in p and "must be" in p for p in problems), (
        f"a malformed status_line must be reported: {problems}"
    )


def test_the_recipes_final_status_line_is_the_verdict_the_receipt_keeps(monkeypatch) -> None:
    """consumer-msrv prints one line per surface and then its summary. The
    summary is the verdict: an earlier per-surface PASS must not qualify a run
    whose final line is not PASS, and the receipt keeps the final line — the
    bounded output tail cannot be relied on to contain it."""
    run = importlib.util.spec_from_file_location("gates_run", REPO / "tools" / "gates" / "run.py")
    assert run and run.loader
    module = importlib.util.module_from_spec(run)
    run.loader.exec_module(module)
    gate = {
        "id": "consumer-msrv",
        "recipe": "consumer-msrv",
        "status_line": {"marker": "taskmesh-consumer-msrv", "require": "status=PASS"},
    }

    def fake_run(stdout: str, returncode: int = 0):
        def _run(*_args, **_kwargs):
            return (
                subprocess.CompletedProcess(
                    args=["just", "consumer-msrv"],
                    returncode=returncode,
                    stdout=stdout,
                    stderr="x" * 4000,  # enough stderr to push stdout out of the tail
                ),
                False,
            )

        return _run

    misleading = (
        "taskmesh-consumer-msrv surface=default toolchain=1.81 status=PASS\n"
        "taskmesh-consumer-msrv status=NOT_RUN msrv=1.81 reason=toolchain-missing\n"
    )
    monkeypatch.setattr(module, "execute_gate_process", fake_run(misleading))
    result = module.run_gate(gate)
    assert result["status"] == "NOT_RUN", (
        f"the recipe's final status line is the verdict, not an earlier one: {result}"
    )
    assert result["status_line"].startswith("taskmesh-consumer-msrv status=NOT_RUN")

    passing = (
        "taskmesh-consumer-msrv surface=default toolchain=1.81 status=PASS\n"
        "taskmesh-consumer-msrv surface=rayon toolchain=1.81 status=PASS\n"
        "taskmesh-consumer-msrv status=PASS msrv=1.81 toolchain=1.81\n"
    )
    monkeypatch.setattr(module, "execute_gate_process", fake_run(passing))
    result = module.run_gate(gate)
    assert result["status"] == "PASS"
    assert result["status_line"] == "taskmesh-consumer-msrv status=PASS msrv=1.81 toolchain=1.81"
    assert "status=PASS msrv=1.81" not in result["output_tail"], (
        "the tail is not where the verdict survives; the status_line field is"
    )


def test_generated_mutation_timeout_is_bounded_and_inventory_owned() -> None:
    inventory, *_ = real_inputs()
    generated = next(gate for gate in inventory["gates"] if gate["id"] == "mutants-generated")
    assert generated["timeout_seconds"] == inventory["qualification_budget_seconds"] == 18000
    assert problems_with(inventory=inventory) == []

    invalid = copy.deepcopy(inventory)
    next(gate for gate in invalid["gates"] if gate["id"] == "mutants-generated")[
        "timeout_seconds"
    ] = 18001
    assert any(
        "timeout_seconds exceeds qualification_budget_seconds" in problem
        for problem in problems_with(inventory=invalid)
    )


def test_doc_fixture_cannot_feature_unify_default_workspace_validation() -> None:
    test_body = vi.recipe_body("test")
    assert test_body.count("--workspace --exclude taskmesh-doc-examples") == 2
    assert "cargo test --locked -p taskmesh-doc-examples" in test_body

    clippy_body = vi.recipe_body("clippy")
    assert clippy_body.count("--exclude taskmesh-doc-examples") == 2
    assert "cargo clippy --locked -p taskmesh --features rayon" in clippy_body
    assert "cargo clippy --locked -p taskmesh-doc-examples" in clippy_body

    doctest_body = vi.recipe_body("doctest")
    assert "--workspace --exclude taskmesh-doc-examples --doc" in doctest_body
    assert "-p taskmesh --features rayon --doc" in doctest_body

    rustdoc_body = vi.recipe_body("rustdoc")
    assert "--workspace --exclude taskmesh-doc-examples" in rustdoc_body
    assert "-p taskmesh --features rayon" in rustdoc_body


def test_missing_gate_executable_is_fail_not_missing_receipt(monkeypatch) -> None:
    run = importlib.util.spec_from_file_location("gates_run", REPO / "tools" / "gates" / "run.py")
    assert run and run.loader
    module = importlib.util.module_from_spec(run)
    run.loader.exec_module(module)

    def missing(*_args, **_kwargs):
        raise FileNotFoundError("just is not installed")

    monkeypatch.setattr(module, "execute_gate_process", missing)
    result = module.run_gate({"id": "fmt-check", "recipe": "fmt-check"})
    assert result["status"] == "FAIL"
    assert result["exit_code"] is None
    assert "could not start" in result["output_tail"]


def test_timed_out_gate_terminates_then_kills_the_process_group(monkeypatch) -> None:
    run = importlib.util.spec_from_file_location("gates_run", REPO / "tools" / "gates" / "run.py")
    assert run and run.loader
    module = importlib.util.module_from_spec(run)
    run.loader.exec_module(module)

    class Process:
        pid = 123
        returncode = -9

        def __init__(self) -> None:
            self.calls = 0

        def communicate(self, timeout=None):
            self.calls += 1
            if self.calls < 3:
                raise subprocess.TimeoutExpired(["just", "mutants-generated"], timeout)
            return "partial stdout", "partial stderr"

    process = Process()
    monkeypatch.setattr(module.subprocess, "Popen", lambda *_args, **_kwargs: process)
    signals = []
    monkeypatch.setattr(module.os, "killpg", lambda pid, sig: signals.append((pid, sig)))
    result = module.run_gate(
        {"id": "mutants-generated", "recipe": "mutants-generated", "timeout_seconds": 7}
    )
    assert result["status"] == "FAIL"
    assert result["timed_out"] is True and result["timeout_seconds"] == 7
    assert result["exit_code"] == -9 and result["signal"] == 9
    assert signals == [(123, module.signal.SIGTERM), (123, module.signal.SIGKILL)]


def test_a_recipe_script_git_does_not_track_is_reported(tmp_path: Path) -> None:
    """`tools/coverage/report.sh` sat under an unanchored `coverage/` ignore
    rule: present on the author's disk, absent from every clone, so the CI
    coverage / qualification jobs would have failed on a missing file. The
    check reads git's own view (`ls-files`), so untracked and ignored scripts
    are both caught — a file's presence on disk proves nothing."""
    subprocess.run(["git", "init", "-q", str(tmp_path)], check=True)
    tools = tmp_path / "tools"
    (tools / "a").mkdir(parents=True)
    (tools / "b").mkdir()
    (tools / "c").mkdir()
    (tools / "a" / "tracked.sh").write_text("echo tracked\n", encoding="utf-8")
    (tools / "b" / "untracked.sh").write_text("echo untracked\n", encoding="utf-8")
    (tools / "c" / "ignored.sh").write_text("echo ignored\n", encoding="utf-8")
    (tmp_path / ".gitignore").write_text("c/\n", encoding="utf-8")
    subprocess.run(
        ["git", "-C", str(tmp_path), "add", "tools/a/tracked.sh", ".gitignore"], check=True
    )
    justfile = (
        "one:\n    bash tools/a/tracked.sh\n\n"
        "two:\n    bash tools/b/untracked.sh\n\n"
        "three:\n    bash tools/c/ignored.sh && bash tools/a/tracked.sh\n"
    )
    tracked = vi.tracked_files(tmp_path)
    assert "tools/a/tracked.sh" in tracked
    assert vi.untracked_recipe_scripts("one", tracked, justfile) == []
    assert vi.untracked_recipe_scripts("two", tracked, justfile) == ["tools/b/untracked.sh"]
    assert vi.untracked_recipe_scripts("three", tracked, justfile) == ["tools/c/ignored.sh"], (
        "an ignored file is on disk and still not in any clone"
    )

    # Through validate(): the problem names the gate, the recipe and the file.
    inventory, *_ = real_inputs()
    problems = problems_with(
        inventory=copy.deepcopy(inventory),
        untracked_scripts={"coverage-report": ["tools/coverage/report.sh"]},
    )
    assert any(
        "coverage-report" in p and "tools/coverage/report.sh" in p and "does not track" in p
        for p in problems
    ), f"a recipe script git does not track must be reported: {problems}"
    assert problems_with(untracked_scripts={}) == []


def test_the_committed_inventorys_scripts_are_all_tracked() -> None:
    """The live check, on the real repository: every script every gate recipe
    runs is in git. (This is the test that would have been red for the
    coverage script from the day it was written.)"""
    inventory, *_ = real_inputs()
    tracked = vi.tracked_files()
    missing = {
        gate["id"]: vi.untracked_recipe_scripts(gate["recipe"], tracked)
        for gate in inventory["gates"]
    }
    missing = {gate: scripts for gate, scripts in missing.items() if scripts}
    assert missing == {}, f"gate scripts git does not track: {missing}"


def test_coverage_status_line_discloses_uncollected_and_instantiation_metrics(
    tmp_path: Path,
) -> None:
    summary = tmp_path / "summary.json"
    summary.write_text(
        json.dumps(
            {
                "data": [
                    {
                        "totals": {
                            "lines": {"count": 10, "percent": 90.0},
                            "regions": {"count": 20, "percent": 80.0},
                            "functions": {"count": 5, "percent": 70.0},
                            "instantiations": {"count": 30, "percent": 56.63},
                            "branches": {"count": 0, "percent": 0.0},
                            "mcdc": {"count": 0, "percent": 0.0},
                        }
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    proc = subprocess.run(
        ["bash", "tools/coverage/report.sh", "--summarize", str(summary)],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=False,
    )
    assert proc.returncode == 0, proc.stderr
    assert "instantiations=56.63" in proc.stdout, "instantiation coverage was omitted"
    assert "branches=NOT_COLLECTED branch_count=0" in proc.stdout
    assert "mcdc=NOT_COLLECTED mcdc_count=0" in proc.stdout
