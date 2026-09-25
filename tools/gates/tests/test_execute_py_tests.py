"""Pytest collection and execution are separate evidence sets."""

from __future__ import annotations

import pytest

from tools.gates.execute_py_tests import reconcile


def test_reconcile_counts_skip_as_exclusion_not_pass() -> None:
    selected = {
        "tools/tests/test_a.py::test_ok": {"module": "tools/tests/test_a.py"},
        "tools/tests/test_a.py::test_skip": {"module": "tools/tests/test_a.py"},
    }
    outcomes = {
        "tools/tests/test_a.py::test_ok": [{
            "phase": "call", "outcome": "passed", "xfail": False,
        }],
        "tools/tests/test_a.py::test_skip": [{
            "phase": "setup", "outcome": "skipped", "xfail": False,
        }],
    }
    passed, excluded = reconcile(selected, outcomes, {"tools/tests/test_a.py"}, [])
    assert passed == ["tools/tests/test_a.py::test_ok"]
    assert excluded == [{
        "case": "tools/tests/test_a.py::test_skip", "reason": "skipped_or_xfail",
    }]


def test_reconcile_rejects_missing_module_and_all_skipped_module() -> None:
    selected = {"tools/tests/test_a.py::test_skip": {"module": "tools/tests/test_a.py"}}
    outcomes = {"tools/tests/test_a.py::test_skip": [{
        "phase": "setup", "outcome": "skipped", "xfail": False,
    }]}
    with pytest.raises(ValueError, match="module denominator mismatch"):
        reconcile(selected, outcomes, {"tools/tests/test_other.py"}, [])
    with pytest.raises(ValueError, match="no passed cases"):
        reconcile(selected, outcomes, {"tools/tests/test_a.py"}, [])


def test_reconcile_rejects_unfinished_or_failed_case() -> None:
    selected = {"tools/tests/test_a.py::test_one": {"module": "tools/tests/test_a.py"}}
    with pytest.raises(ValueError, match="no terminal result"):
        reconcile(selected, {}, {"tools/tests/test_a.py"}, [])
    with pytest.raises(ValueError, match="case failed"):
        reconcile(selected, {"tools/tests/test_a.py::test_one": [{
            "phase": "call", "outcome": "failed", "xfail": False,
        }]}, {"tools/tests/test_a.py"}, [])
