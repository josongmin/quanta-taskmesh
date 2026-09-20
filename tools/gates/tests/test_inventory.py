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
            return subprocess.CompletedProcess(
                args=["just", "consumer-msrv"],
                returncode=returncode,
                stdout=stdout,
                stderr="x" * 4000,  # enough stderr to push stdout out of the tail
            )

        return _run

    misleading = (
        "taskmesh-consumer-msrv surface=default toolchain=1.81 status=PASS\n"
        "taskmesh-consumer-msrv status=NOT_RUN msrv=1.81 reason=toolchain-missing\n"
    )
    monkeypatch.setattr(module.subprocess, "run", fake_run(misleading))
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
    monkeypatch.setattr(module.subprocess, "run", fake_run(passing))
    result = module.run_gate(gate)
    assert result["status"] == "PASS"
    assert result["status_line"] == "taskmesh-consumer-msrv status=PASS msrv=1.81 toolchain=1.81"
    assert "status=PASS msrv=1.81" not in result["output_tail"], (
        "the tail is not where the verdict survives; the status_line field is"
    )


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
