"""The mutation runner's own classification logic, driven without cargo.

Everything the runner decides — killed, survived, wrong test, wrong reason,
invalid (compile error, no tests, timeout), control green/broken — is a pure
function of the test process's output, so it is tested here on captured
output shapes. The one side effect that matters (restoring the mutated file
whatever happens, including on a timeout) is tested against a real temporary
file with the test command monkeypatched.
"""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]
RUNNER = REPO / "tools" / "verification" / "run_mutations.py"
_spec = importlib.util.spec_from_file_location("run_mutations", RUNNER)
assert _spec and _spec.loader
rm = importlib.util.module_from_spec(_spec)
# Dataclasses resolve string annotations through `sys.modules`; register the
# module before executing it or `@dataclass` cannot find its own namespace.
sys.modules["run_mutations"] = rm
_spec.loader.exec_module(rm)


def proc(stdout: str = "", stderr: str = "", code: int = 0) -> subprocess.CompletedProcess[str]:
    return subprocess.CompletedProcess(args=["x"], returncode=code, stdout=stdout, stderr=stderr)


def cargo_run(*failed: str, total: int = 3, message: str = "", messages: dict | None = None) -> str:
    """libtest-shaped output: FAILED lines, then a `---- name stdout ----` block
    per failure (message from `messages[name]`, else `message`)."""
    lines = [f"running {total} tests"]
    for name in failed:
        lines.append(f"test {name} ... FAILED")
    if failed:
        lines.append("\nfailures:\n")
        for name in failed:
            text = (messages or {}).get(name, message)
            lines.append(f"---- {name} stdout ----\nthread 'x' panicked at src/x.rs:1:1:\n{text}\n")
        lines.append("\nfailures:")
        lines.extend(f"    {name}" for name in failed)
    lines.append(f"\ntest result: {'FAILED' if failed else 'ok'}. …")
    return "\n".join(lines) + "\n"


MUT = {
    "id": "m",
    "finding": "F",
    "file": "x.rs",
    "find": "a",
    "replace": "b",
    "package": "p",
    "test_target": "t",
    "expect_failing_test": "the_test",
    "expect_message": "the property",
}


def test_killed_requires_the_named_test_and_the_named_reason() -> None:
    assert rm.evaluate(MUT, proc(cargo_run("the_test", message="the property")))[0] == "KILLED"
    assert rm.evaluate(MUT, proc(cargo_run()))[0] == "SURVIVED"
    wrong_test = proc(cargo_run("other_test", message="the property"))
    assert rm.evaluate(MUT, wrong_test)[0] == "WRONG_TEST"
    assert (
        rm.evaluate(MUT, proc(cargo_run("the_test", message="something else")))[0] == "WRONG_REASON"
    )


def test_the_reason_must_come_from_the_named_test_not_a_neighbour() -> None:
    """Several `expect_message`s are shared helper strings; a different failing
    test emitting the message must not credit the named one."""
    out = cargo_run(
        "other",
        "the_test",
        messages={"other": "the property", "the_test": "something else"},
    )
    assert rm.evaluate(MUT, proc(out))[0] == "WRONG_REASON"
    out = cargo_run(
        "other",
        "the_test",
        messages={"other": "something else", "the_test": "the property"},
    )
    assert rm.evaluate(MUT, proc(out))[0] == "KILLED"


def test_pytest_reason_is_scoped_to_the_named_test_too() -> None:
    py = {**MUT, "runner": "pytest", "test_target": "tools/x/tests/test_x.py::the_test"}
    out = (
        "FF\n"
        "=================================== FAILURES ===================================\n"
        "_________________________________ other _________________________________\n"
        "    assert False, 'the property'\n"
        "E   AssertionError: the property\n"
        "________________________________ the_test _______________________________\n"
        "E   AssertionError: something else\n"
        "=========================== short test summary info ============================\n"
        "FAILED tools/x/tests/test_x.py::other - AssertionError: the property\n"
        "FAILED tools/x/tests/test_x.py::the_test - AssertionError: something else\n"
        "2 failed in 0.10s\n"
    )
    assert rm.evaluate(py, proc(out, code=1))[0] == "WRONG_REASON"
    assert (
        rm.evaluate(py, proc(out.replace("something else", "the property"), code=1))[0] == "KILLED"
    )


def test_a_long_pytest_failure_header_keeps_its_assertion_reason() -> None:
    name = "test_coverage_status_line_discloses_uncollected_and_instantiation_metrics"
    out = (
        f"__ {name} ___\n\n"
        "E       AssertionError: instantiation coverage was omitted\n"
        "=========================== short test summary info ============================\n"
        f"FAILED tools/gates/tests/test_inventory.py::{name}\n"
    )
    failure = rm.failure_output(out, name, "pytest")
    assert "instantiation coverage was omitted" in failure, (
        "a long pytest header lost its assertion reason"
    )


def test_pytest_long_runs_and_fixture_errors_are_classified() -> None:
    py = {**MUT, "runner": "pytest", "test_target": "tools/x/tests/test_x.py::the_test"}
    # Past a minute pytest appends a wall-clock suffix; the count must still parse.
    long_run = (
        "FAILED tools/x/tests/test_x.py::the_test - AssertionError: the property\n"
        "1 failed, 194 passed in 80.09s (0:01:20)\n"
    )
    assert rm.collected_tests(long_run, "pytest") == 195
    assert rm.evaluate(py, proc(long_run, code=1))[0] == "KILLED"
    # A fixture ERROR on the named test is a failure to run it, not a survivor.
    errored = (
        "ERROR tools/x/tests/test_x.py::the_test - RuntimeError: the property\n1 error in 0.10s\n"
    )
    assert rm.failing_tests(errored, "pytest") == ["the_test"]
    assert rm.evaluate(py, proc(errored, code=1))[0] == "KILLED"


def test_a_mutant_that_did_not_run_is_invalid_not_killed() -> None:
    status, _, _ = rm.evaluate(
        MUT, proc(stderr="error[E0308]: mismatched types\ncould not compile")
    )
    assert status == "INVALID_COMPILE_ERROR"
    assert rm.evaluate(MUT, proc("running 0 tests\n"))[0] == "INVALID_NO_TESTS"


def test_a_control_must_stay_green() -> None:
    control = {**MUT, "finding": "control", "expect_no_failure": True}
    assert rm.evaluate(control, proc(cargo_run()))[0] == "CONTROL_GREEN"
    assert rm.evaluate(control, proc(cargo_run("the_test")))[0] == "CONTROL_BROKEN"


def test_pytest_output_is_classified_the_same_way() -> None:
    py = {**MUT, "runner": "pytest", "test_target": "tools/x/tests/test_x.py::the_test"}
    killed = (
        "F..\n"
        "FAILED tools/x/tests/test_x.py::the_test - AssertionError: the property\n"
        "1 failed, 2 passed in 0.10s\n"
    )
    assert rm.evaluate(py, proc(killed, code=1))[0] == "KILLED"
    assert rm.evaluate(py, proc("...\n3 passed in 0.10s\n"))[0] == "SURVIVED"
    wrong = "FAILED tools/x/tests/test_x.py::other - x\n1 failed, 2 passed in 0.1s\n"
    assert rm.evaluate(py, proc(wrong, code=1))[0] == "WRONG_TEST"
    assert rm.evaluate(py, proc(stderr="  File x.py\nSyntaxError: invalid syntax"))[0] == (
        "INVALID_COMPILE_ERROR"
    )
    assert rm.evaluate(py, proc("no tests ran in 0.01s\n"))[0] == "INVALID_NO_TESTS"


def test_the_test_command_reflects_runner_and_profile() -> None:
    assert rm.test_command(MUT)[:5] == ["cargo", "test", "-p", "p", "--test"]
    assert "--release" not in rm.test_command(MUT)
    assert "--release" in rm.test_command({**MUT, "profile": "release"})
    assert rm.test_command({**MUT, "test_target": "lib"})[4] == "--lib"
    py = rm.test_command({**MUT, "runner": "pytest", "test_target": "a.py::t"})
    assert py[1:3] == ["-m", "pytest"] and py[-1] == "a.py::t"
    assert "-rfE" in py, "errors must be listed in the summary, not only failures"
    # A model-check target needs its feature and its cfg together.
    shuttle = rm.test_command(
        {**MUT, "cargo_args": ["--features", "shuttle"], "env": {"RUSTFLAGS": "--cfg shuttle"}}
    )
    assert shuttle[shuttle.index("--features") + 1] == "shuttle"
    assert shuttle.index("--features") < shuttle.index("--test"), "cargo args precede the selector"


@pytest.mark.parametrize(
    "broken,reason",
    [
        ({**MUT, "expect_message": ""}, "missing expect_message"),
        ({**MUT, "replace": "a"}, "identical"),
        ({**MUT, "runner": "make"}, "unknown runner"),
        ({**MUT, "profile": "fast"}, "unknown profile"),
        ({**MUT, "expect_no_failure": True}, "labelled finding=control"),
        ({**MUT, "finding": "control"}, "without expect_no_failure"),
        ({k: v for k, v in MUT.items() if k != "expect_failing_test"}, "expect_failing_test"),
        ({**MUT, "env": {"RUSTFLAGS": 1}}, "env must be"),
        ({**MUT, "cargo_args": "--features shuttle"}, "cargo_args must be"),
        ({**MUT, "runner": "pytest", "env": {"X": "1"}}, "cargo runner only"),
    ],
)
def test_an_entry_that_cannot_prove_its_claim_is_refused(broken: dict, reason: str) -> None:
    with pytest.raises(SystemExit) as raised:
        rm.validate_entry(broken)
    assert reason in str(raised.value)


def test_an_inventory_without_a_control_is_refused(tmp_path: Path) -> None:
    path = tmp_path / "m.json"
    path.write_text(json.dumps({"mutations": [MUT]}))
    with pytest.raises(SystemExit, match="no control entry"):
        rm.load_inventory(path)


def test_the_committed_inventory_validates() -> None:
    mutations = rm.load_inventory(rm.INVENTORY)
    assert len(mutations) >= 30
    controls = [m for m in mutations if m.get("expect_no_failure")]
    assert len(controls) == 1 and controls[0]["finding"] == "control"


def test_receipt_names_its_curated_scope_and_separates_the_control() -> None:
    mutations = rm.load_inventory(rm.INVENTORY)
    outcomes = [
        rm.Outcome(
            mutation_id=mutation["id"],
            finding=mutation["finding"],
            status="CONTROL_GREEN" if mutation.get("expect_no_failure") else "KILLED",
            detail="proof",
            duration_s=0.1,
        )
        for mutation in mutations
    ]
    receipt = rm.build_receipt(mutations, outcomes, [])
    assert receipt["schema_version"] == 2
    assert receipt["kind"] == "curated-single-edit-inventory"
    assert receipt["inventory_total"] == 105
    assert receipt["selection"] == "full"
    assert len(receipt["inventory_digest"]) == 64
    assert receipt["status_counts"] == {"CONTROL_GREEN": 1, "KILLED": 104}
    assert receipt["scope"] == {
        "runner_counts": {"cargo": 90, "pytest": 15},
        "source_counts": {
            "tooling_faults": 15,
            "rust_crate_src": 89,
            "rust_crate_tests": 1,
        },
        "control_entries": 1,
    }


def test_every_committed_anchor_matches_its_source_exactly_once() -> None:
    """Caught by the fast gate, not by minute 28 of the mutation run: `cargo fmt`
    or `ruff format` rewrapping a line moves an anchor, and a mutation whose
    anchor no longer matches proves nothing."""
    drifted = []
    for m in rm.load_inventory(rm.INVENTORY):
        text = (REPO / m["file"]).read_text(encoding="utf-8")
        count = text.count(m["find"])
        if count != 1:
            drifted.append((m["id"], count))
    assert not drifted, f"anchors that do not match exactly once: {drifted}"


def test_a_timeout_is_invalid_and_the_file_is_still_restored(tmp_path: Path, monkeypatch) -> None:
    """The docstring promised a hang fixture; this is it. A mutant whose tests
    never finish is INVALID_TIMEOUT (never KILLED), and the mutated source is
    restored even though the run never returned normally."""
    target = tmp_path / "src.rs"
    target.write_text("fn f() { a }\n")
    monkeypatch.setattr(rm, "REPO", tmp_path)

    def hang(_mutation: dict) -> subprocess.CompletedProcess[str]:
        assert target.read_text() == "fn f() { b }\n", "the mutant is applied while tests run"
        raise subprocess.TimeoutExpired(cmd="cargo test", timeout=rm.TEST_TIMEOUT_SECONDS)

    monkeypatch.setattr(rm, "run_tests", hang)
    outcome = rm.run_one({**MUT, "file": "src.rs"}, verbose=False)
    assert outcome.status == "INVALID_TIMEOUT"
    assert target.read_text() == "fn f() { a }\n", "restored after the timeout"


def test_an_anchor_that_does_not_match_exactly_once_is_refused(tmp_path: Path, monkeypatch) -> None:
    target = tmp_path / "src.rs"
    target.write_text("a a\n")
    monkeypatch.setattr(rm, "REPO", tmp_path)
    with pytest.raises(SystemExit, match="matched 2 times"):
        rm.apply_mutation({**MUT, "file": "src.rs"})
    assert target.read_text() == "a a\n"


def test_require_clean_refuses_a_dirty_tree(tmp_path: Path, monkeypatch) -> None:
    subprocess.run(["git", "init", "-q"], cwd=tmp_path, check=True)
    (tmp_path / "f.txt").write_text("x")
    monkeypatch.setattr(rm, "REPO", tmp_path)
    assert rm.verify_tree(require_clean=False) == ["f.txt"], "dirtiness is reported, not hidden"
    with pytest.raises(SystemExit, match="dirty tree"):
        rm.verify_tree(require_clean=True)
