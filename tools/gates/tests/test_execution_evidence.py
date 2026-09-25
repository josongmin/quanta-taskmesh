"""A saved PASS must retain the same source-bound selection and execution."""

from __future__ import annotations

import copy
import subprocess

from tools.gates import execution_evidence as evidence
from tools.gates import run as gate_run


def test_rust_report_rejects_edited_case_and_status_summary(monkeypatch) -> None:
    record = {
        "package": "pkg", "kind": "test", "target": "contract",
        "executing_gate": "test", "path": "crates/pkg/tests/contract.rs",
    }
    monkeypatch.setattr(evidence, "source_catalog", lambda root, recipes: ([record], []))
    monkeypatch.setattr(evidence, "recipe_body", lambda name: "")
    source = {"head": "a" * 40, "tree": "b" * 40, "dirty": False, "paths_digest": "c" * 64}
    selected = ["pkg::contract$test_one"]
    commands = []
    for selector in evidence.SELECTORS:
        common = ["--locked", *selector]
        commands.append({
            "list": ["cargo", "nextest", "list", *common, "--message-format", "json"],
            "run": [
                "cargo", "nextest", "run", *common, "--message-format", "libtest-json-plus",
                "--message-format-version", "0.1", "--test-threads", "4",
            ],
        })
    report = {
        "schema_version": 1, "source": source, "runner": "nextest",
        "runner_version": "0.9.104", "commands": commands,
        "experimental_env": {"NEXTEST_EXPERIMENTAL_LIBTEST_JSON": "1"},
        "catalog_digest": evidence.catalog_digest([record]),
        "targets": [["pkg", "test", "contract"]],
        "selected": selected, "passed": selected,
        "selection_digest": evidence.digest(selected),
        "execution_digest": evidence.digest(selected),
    }
    line = (
        "taskmesh-test status=PASS runner=nextest runner_version=0.9.104 "
        "targets=1 selected=1 passed=1 "
        f"catalog_digest={report['catalog_digest']} "
        f"selection_digest={report['selection_digest']} "
        f"execution_digest={report['execution_digest']}"
    )
    assert evidence.report_problems("test", report, line, source) == []
    tampered = copy.deepcopy(report)
    tampered["passed"] = []
    assert any("execution differs" in problem for problem in evidence.report_problems(
        "test", tampered, line, source
    ))
    assert any("selected count" in problem for problem in evidence.report_problems(
        "test", report, line.replace("selected=1", "selected=2"), source
    ))
    assert any("source/schema" in problem for problem in evidence.report_problems(
        "test", report, line, {**source, "head": "d" * 40}
    ))


def test_pytest_report_rejects_all_excluded_and_module_drift(monkeypatch) -> None:
    record = {
        "package": "tools", "kind": "pytest-module", "target": "test_one",
        "executing_gate": "py-test", "path": "tools/tests/test_one.py",
    }
    monkeypatch.setattr(evidence, "source_catalog", lambda root, recipes: ([record], []))
    monkeypatch.setattr(evidence, "recipe_body", lambda name: "")
    source = {"head": "a" * 40, "dirty": False}
    selected = ["tools/tests/test_one.py::test_ok"]
    report = {
        "schema_version": 1, "source": source, "runner": "pytest",
        "runner_version": "9.0.3", "command": ["pytest", "tools", "-q", "--strict-markers"],
        "catalog_digest": evidence.catalog_digest([record]),
        "modules": ["tools/tests/test_one.py"], "selected": selected,
        "passed": selected, "excluded": [], "slow": [], "qualification": [],
        "selection_digest": evidence.digest(selected),
        "execution_digest": evidence.digest({"passed": selected, "excluded": []}),
    }
    line = (
        "taskmesh-py-test status=PASS runner=pytest runner_version=9.0.3 "
        "modules=1 selected=1 passed=1 excluded=0 slow=0 qualification=0 "
        f"catalog_digest={report['catalog_digest']} "
        f"selection_digest={report['selection_digest']} "
        f"execution_digest={report['execution_digest']}"
    )
    assert evidence.report_problems("py-test", report, line, source) == []
    tampered = copy.deepcopy(report)
    tampered["passed"] = []
    tampered["excluded"] = [{"case": selected[0], "reason": "skipped_or_xfail"}]
    assert any("execution digest" in problem for problem in evidence.report_problems(
        "py-test", tampered, line, source
    ))
    tampered = copy.deepcopy(report)
    tampered["modules"] = []
    assert any("module or command" in problem for problem in evidence.report_problems(
        "py-test", tampered, line, source
    ))
    tampered = copy.deepcopy(report)
    tampered["slow"] = [{}]
    assert any("marked-case denominator" in problem for problem in evidence.report_problems(
        "py-test", tampered, line, source
    ))


def test_successful_process_without_execution_report_is_not_a_pass(tmp_path, monkeypatch) -> None:
    monkeypatch.setitem(gate_run.REPORTS, "test", tmp_path / "missing.json")
    line = (
        "taskmesh-test status=PASS runner=nextest runner_version=0.9.104 "
        "targets=1 selected=1 passed=1 catalog_digest=" + "a" * 64
        + " selection_digest=" + "b" * 64 + " execution_digest=" + "b" * 64
    )
    process = subprocess.CompletedProcess(["just", "test"], 0, line + "\n", "")
    result = gate_run.gate_process_result(
        {"id": "test", "recipe": "test", "status_line": {
            "marker": "taskmesh-test", "require": "status=PASS",
        }},
        process, False, "2026-09-26T00:00:00Z", 0.1,
    )
    assert result["status"] == "NOT_RUN"
    assert result["process_exit_code"] == 0
    assert "cannot read execution report" in result["output_tail"]


def test_saved_pass_without_execution_report_is_invalid() -> None:
    source = {"head": "a" * 40, "tree": "b" * 40, "dirty": False, "paths_digest": "c" * 64}
    line = (
        "taskmesh-test status=PASS runner=nextest runner_version=0.9.104 "
        "targets=1 selected=1 passed=1 catalog_digest=" + "a" * 64
        + " selection_digest=" + "b" * 64 + " execution_digest=" + "b" * 64
    )
    receipt = {
        "schema_version": 2, "platform": "macos", "source": source,
        "source_after": source, "source_problems": [], "qualified": True,
        "required_not_run": [], "required_not_passed": [],
        "platform_scope": {"qualified": True, "excluded_required_gates": []},
        "results": [{"id": "test", "status": "PASS", "exit_code": 0, "status_line": line}],
    }
    problems = gate_run.local_receipt_problems(receipt, source, {"test"}, {}, "macos")
    assert any("execution report is missing" in problem for problem in problems)


def test_catalog_failure_invalidates_report_instead_of_raising(monkeypatch) -> None:
    def unavailable(_root, _recipes):
        raise subprocess.CalledProcessError(101, ["cargo", "metadata"])

    monkeypatch.setattr(evidence, "source_catalog", unavailable)
    problems = evidence.report_problems("test", {}, "taskmesh-test status=PASS", {})
    assert any("catalog unavailable" in problem for problem in problems)
