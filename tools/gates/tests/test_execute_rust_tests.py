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


def test_selected_cases_excludes_ignored_and_rejects_filtered() -> None:
    case = lambda ignored, status: {  # noqa: E731
        "ignored": ignored, "filter-match": {"status": status}
    }
    targets, selected = selected_cases(listed({
        "runs": case(False, "matches"),
        "ignored": case(True, "matches"),
    }))
    assert targets == {("pkg", "test", "target")}
    assert selected == {"pkg::target$runs"}
    with pytest.raises(ValueError, match="filtered or malformed"):
        selected_cases(listed({
            "runs": case(False, "matches"),
            "filtered": case(False, "mismatch"),
        }))


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
    with pytest.raises(ValueError, match="filtered or malformed"):
        selected_cases(value)


def test_only_registered_zero_case_libraries_stay_in_target_denominator() -> None:
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
    value["rust-suites"]["empty"].update({
        "package-name": "unknown",
        "kind": "test",
    })
    with pytest.raises(ValueError, match="no runnable cases"):
        selected_cases(value)


def test_execution_requires_started_and_ok_for_every_case() -> None:
    lines = '\n'.join([
        '{"type":"suite","event":"started","test_count":1,'
        '"nextest":{"crate":"pkg","test_binary":"target"}}',
        '{"type":"test","event":"started","name":"pkg::target$runs"}',
        '{"type":"test","event":"ok","name":"pkg::target$runs"}',
        '{"type":"suite","event":"ok","passed":1,"failed":0,"ignored":0,'
        '"filtered_out":0,"nextest":{"crate":"pkg","test_binary":"target"}}',
    ])
    cases = {"pkg::target$runs"}
    targets = {("pkg", "test", "target")}
    started, passed = executed_cases(lines, cases, targets)
    assert started == passed == {"pkg::target$runs"}
    with pytest.raises(ValueError, match="did not pass"):
        executed_cases(
            lines.replace('"event":"ok","name"', '"event":"failed","name"'),
            cases,
            targets,
        )
    with pytest.raises(ValueError, match="unmatched test pass"):
        executed_cases(
            lines.replace(
                '{"type":"test","event":"started","name":"pkg::target$runs"}\n', ""
            ),
            cases,
            targets,
        )


def test_execution_accepts_omitted_or_paired_zero_case_suite() -> None:
    cases = {"pkg::target$runs"}
    targets = {
        ("pkg", "test", "target"),
        ("taskmesh-contract", "lib", "taskmesh_contract"),
    }
    nonempty = '\n'.join([
        '{"type":"suite","event":"started","test_count":1,'
        '"nextest":{"crate":"pkg","test_binary":"target"}}',
        '{"type":"test","event":"started","name":"pkg::target$runs"}',
        '{"type":"test","event":"ok","name":"pkg::target$runs"}',
        '{"type":"suite","event":"ok","passed":1,"failed":0,"ignored":0,'
        '"filtered_out":0,"nextest":{"crate":"pkg","test_binary":"target"}}',
    ])
    assert executed_cases(nonempty, cases, targets) == (cases, cases)
    zero = '\n'.join([
        '{"type":"suite","event":"started","test_count":0,'
        '"nextest":{"crate":"taskmesh-contract","test_binary":"taskmesh_contract"}}',
        '{"type":"suite","event":"ok","passed":0,"failed":0,"ignored":0,'
        '"filtered_out":0,'
        '"nextest":{"crate":"taskmesh-contract","test_binary":"taskmesh_contract"}}',
    ])
    assert executed_cases(nonempty + "\n" + zero, cases, targets) == (cases, cases)
    with pytest.raises(ValueError, match="unexpected nextest suite"):
        executed_cases(
            nonempty.replace('"crate":"pkg"', '"crate":"unknown"'),
            cases,
            targets,
        )
    with pytest.raises(ValueError, match="unregistered zero-case target"):
        executed_cases(nonempty, cases, targets | {("unknown", "test", "empty")})


def test_rayon_zero_case_target_has_checked_suite_identity() -> None:
    selection = {
        "test-count": 1,
        "rust-suites": {
            "pkg::case": {
                "package-name": "pkg",
                "binary-name": "case",
                "kind": "test",
                "status": "listed",
                "testcases": {
                    "runs": {
                        "ignored": False,
                        "filter-match": {"status": "matches"},
                    }
                },
            },
            "taskmesh-contract::taskmesh_contract": {
                "package-name": "taskmesh-contract",
                "binary-name": "taskmesh_contract",
                "kind": "lib",
                "status": "listed",
                "testcases": {},
            },
        },
    }
    cases, targets = rayon_evidence.listed_cases(selection)
    assert cases == {("pkg", "case", "runs")}
    assert targets == {"pkg/test/case", "taskmesh-contract/lib/taskmesh_contract"}
    nonempty = '\n'.join([
        '{"type":"suite","event":"started","test_count":1,'
        '"nextest":{"crate":"pkg","test_binary":"case"}}',
        '{"type":"test","event":"started","name":"pkg::case$runs"}',
        '{"type":"test","event":"ok","name":"pkg::case$runs"}',
        '{"type":"suite","event":"ok","passed":1,"failed":0,"ignored":0,'
        '"filtered_out":0,"nextest":{"crate":"pkg","test_binary":"case"}}',
    ])
    assert rayon_evidence.executed_cases(nonempty, cases, targets) == cases
    zero = '\n'.join([
        '{"type":"suite","event":"started","test_count":0,'
        '"nextest":{"crate":"taskmesh-contract","test_binary":"taskmesh_contract"}}',
        '{"type":"suite","event":"ok","passed":0,"failed":0,"ignored":0,'
        '"filtered_out":0,'
        '"nextest":{"crate":"taskmesh-contract","test_binary":"taskmesh_contract"}}',
    ])
    assert rayon_evidence.executed_cases(nonempty + "\n" + zero, cases, targets) == cases
    with pytest.raises(ValueError, match="unexpected nextest suite"):
        rayon_evidence.executed_cases(
            nonempty.replace('"crate":"pkg"', '"crate":"unknown"'),
            cases,
            targets,
        )
    with pytest.raises(ValueError, match="unregistered zero-case target"):
        rayon_evidence.executed_cases(
            nonempty,
            cases,
            targets | {"unknown/test/empty"},
        )


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
