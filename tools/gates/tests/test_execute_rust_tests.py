"""Selection/execution reconciliation rejects missing and non-passing cases."""

from __future__ import annotations

import pytest

from tools.gates import rust_test_evidence as rayon_evidence
from tools.gates.execute_rust_tests import executed_cases, selected_cases
from tools.gates.status_line import status_line_qualifies


def listed(cases: dict) -> dict:
    return {
        "test-count": sum(
            not case["ignored"] and case["filter-match"]["status"] == "matches"
            for case in cases.values()
        ),
        "rust-suites": {
            "pkg::target": {
                "package-name": "pkg",
                "kind": "test",
                "binary-name": "target",
                "binary-id": "pkg::target",
                "status": "listed",
                "testcases": cases,
            }
        },
    }


def test_selected_cases_excludes_ignored_and_filtered() -> None:
    case = lambda ignored, status: {  # noqa: E731
        "ignored": ignored, "filter-match": {"status": status}
    }
    targets, selected = selected_cases(listed({
        "runs": case(False, "matches"),
        "ignored": case(True, "matches"),
        "filtered": case(False, "mismatch"),
    }))
    assert targets == {("pkg", "test", "target")}
    assert selected == {"pkg::target$runs"}


def test_list_count_must_match_selected_denominator() -> None:
    value = listed({"runs": {"ignored": False, "filter-match": {"status": "matches"}}})
    value["test-count"] = 0
    with pytest.raises(ValueError, match="test-count"):
        selected_cases(value)


def test_library_case_uses_nextest_execution_name() -> None:
    value = listed({"runs": {"ignored": False, "filter-match": {"status": "matches"}}})
    suite = value["rust-suites"]["pkg::target"]
    suite.update({"kind": "lib", "binary-id": "pkg", "binary-name": "pkg_lib"})
    _, cases = selected_cases(value)
    assert cases == {"pkg::pkg_lib$runs"}


def test_empty_test_filter_is_not_execution_evidence() -> None:
    value = listed({"filtered": {"ignored": False, "filter-match": {"status": "mismatch"}}})
    with pytest.raises(ValueError, match="no runnable cases"):
        selected_cases(value)


def test_only_registered_zero_case_libraries_are_allowed() -> None:
    value = listed({"runs": {"ignored": False, "filter-match": {"status": "matches"}}})
    value["rust-suites"]["empty"] = {
        "package-name": "taskmesh-contract",
        "kind": "lib",
        "binary-name": "taskmesh_contract",
        "binary-id": "taskmesh-contract::taskmesh_contract",
        "status": "listed",
        "testcases": {},
    }
    targets, cases = selected_cases(value)
    assert ("taskmesh-contract", "lib", "taskmesh_contract") in targets
    assert cases == {"pkg::target$runs"}
    value["rust-suites"]["empty"]["package-name"] = "unknown"
    with pytest.raises(ValueError, match="no runnable cases"):
        selected_cases(value)


def test_execution_requires_started_and_ok_for_every_case() -> None:
    lines = '\n'.join([
        '{"type":"suite","event":"started","test_count":1}',
        '{"type":"test","event":"started","name":"pkg::target$runs"}',
        '{"type":"test","event":"ok","name":"pkg::target$runs"}',
        '{"type":"suite","event":"ok","passed":1,"failed":0,"ignored":0}',
    ])
    started, passed = executed_cases(lines)
    assert started == passed == {"pkg::target$runs"}
    with pytest.raises(ValueError, match="did not pass"):
        executed_cases(lines.replace('"event":"ok","name"', '"event":"failed","name"'))
    with pytest.raises(ValueError, match="unmatched test pass"):
        executed_cases(lines.replace(
            '{"type":"test","event":"started","name":"pkg::target$runs"}\n', ""
        ))


def test_rayon_status_line_rejects_edited_denominator() -> None:
    summary = {
        "schema_version": 1,
        "runner": "nextest",
        "runner_version": "0.9.104",
        "catalog_digest": "a" * 64,
        "selection_digest": "b" * 64,
        "execution_digest": "b" * 64,
        "commands_digest": "c" * 64,
        "targets": 7,
        "cases": 33,
    }
    summary["summary_digest"] = rayon_evidence.digest(summary)
    line = "taskmesh-test-rayon status=PASS " + " ".join(
        f"{key}={value}" for key, value in summary.items()
    )
    gate = {
        "id": "test-rayon",
        "status_line": {"marker": "taskmesh-test-rayon", "require": "status=PASS"},
    }
    assert status_line_qualifies(gate, line)
    assert not status_line_qualifies(gate, line.replace("cases=33", "cases=32"))
    assert not status_line_qualifies(gate, line.replace("runner=nextest", "runner=cargo"))
