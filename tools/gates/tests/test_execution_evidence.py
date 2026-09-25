"""Negative witnesses for machine-readable CI execution denominators."""

from __future__ import annotations

import json

import pytest

from tools.gates import rust_test_evidence as rust
from tools.gates.status_line import status_line_qualifies


def listed(case: str = "works", *, ignored: bool = False) -> dict:
    return {
        "test-count": 0 if ignored else 1,
        "rust-suites": {
            "pkg::case": {
                "package-name": "pkg",
                "binary-name": "case",
                "kind": "test",
                "status": "listed",
                "testcases": {case: {"ignored": ignored, "filter-match": {"status": "matches"}}},
            }
        },
    }


def events(*, completed: bool = True) -> str:
    rows = [
        {
            "type": "suite",
            "event": "started",
            "test_count": 1,
            "nextest": {"crate": "pkg", "test_binary": "case"},
        },
        {"type": "test", "event": "started", "name": "pkg::case$works"},
    ]
    if completed:
        rows.extend(
            [
                {"type": "test", "event": "ok", "name": "pkg::case$works"},
                {
                    "type": "suite",
                    "event": "ok",
                    "passed": 1,
                    "failed": 0,
                    "ignored": 0,
                    "filtered_out": 0,
                    "nextest": {"crate": "pkg", "test_binary": "case"},
                },
            ]
        )
    return "\n".join(json.dumps(row) for row in rows)


def test_selected_binary_and_completed_case_are_counted_once() -> None:
    cases, targets = rust.listed_cases(listed())
    assert cases == {("pkg", "case", "works")}
    assert targets == {"pkg/test/case"}
    assert rust.executed_cases(events(), cases) == cases


def test_empty_or_ignored_selection_cannot_pass() -> None:
    with pytest.raises(ValueError, match="empty"):
        rust.listed_cases({"test-count": 0, "rust-suites": {}})
    with pytest.raises(ValueError, match="no runnable"):
        rust.listed_cases(listed(ignored=True))


def test_only_the_two_registered_zero_case_libraries_are_allowed() -> None:
    value = listed()
    value["rust-suites"]["empty"] = {
        "package-name": "taskmesh-contract",
        "binary-name": "taskmesh_contract",
        "kind": "lib",
        "status": "listed",
        "testcases": {},
    }
    cases, targets = rust.listed_cases(value)
    assert len(cases) == 1
    assert "taskmesh-contract/lib/taskmesh_contract" in targets
    value["rust-suites"]["empty"]["package-name"] = "unknown"
    with pytest.raises(ValueError, match="no runnable"):
        rust.listed_cases(value)


def test_unstarted_failed_or_missing_case_cannot_pass() -> None:
    cases, _ = rust.listed_cases(listed())
    with pytest.raises(ValueError, match="mismatch"):
        rust.executed_cases(events(completed=False), cases)
    failed = events().replace('"event": "ok", "name"', '"event": "failed", "name"', 1)
    with pytest.raises(ValueError, match="non-pass test"):
        rust.executed_cases(failed, cases)
    wrong_count = events().replace('"test_count": 1', '"test_count": 0')
    with pytest.raises(ValueError, match="suite start denominator"):
        rust.executed_cases(wrong_count, cases)


def test_edited_or_partial_test_summary_cannot_pass() -> None:
    summary = {
        "schema_version": 1,
        "runner": "nextest",
        "runner_version": "0.9.104",
        "catalog_digest": "a" * 64,
        "selection_digest": "b" * 64,
        "execution_digest": "b" * 64,
        "commands_digest": "c" * 64,
        "targets": 1,
        "cases": 1,
    }
    summary["summary_digest"] = rust.digest(summary)
    line = "taskmesh-test status=PASS " + " ".join(
        f"{key}={value}" for key, value in summary.items()
    )
    gate = {"id": "test", "status_line": {"marker": "taskmesh-test", "require": "status=PASS"}}
    assert status_line_qualifies(gate, line)
    assert not status_line_qualifies(gate, line.replace("cases=1", "cases=2"))
    assert not status_line_qualifies(gate, line.replace("runner=nextest", "runner=cargo"))
    rayon = {
        "id": "test-rayon",
        "status_line": {"marker": "taskmesh-test-rayon", "require": "status=PASS"},
    }
    assert status_line_qualifies(rayon, line.replace("taskmesh-test ", "taskmesh-test-rayon "))
