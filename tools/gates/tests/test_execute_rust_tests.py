"""Selection/execution reconciliation rejects missing and non-passing cases."""

from __future__ import annotations

import pytest

from tools.gates.execute_rust_tests import executed_cases, selected_cases


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
