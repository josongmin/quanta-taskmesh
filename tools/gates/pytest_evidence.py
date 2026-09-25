#!/usr/bin/env python3
"""Run CI pytest with an explicit collected and completed case denominator."""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO))

import pytest  # noqa: E402

from tools.gates.target_catalog import (  # noqa: E402
    RECIPE_FRAGMENTS,
    catalog_digest,
    recipe_body,
    source_catalog,
)


def digest(value: object) -> str:
    return hashlib.sha256(
        json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


class CaseEvidence:
    def __init__(self) -> None:
        self.collected: set[str] = set()
        self.passed: set[str] = set()
        self.modules: set[str] = set()
        self.slow: set[str] = set()
        self.qualification: set[str] = set()
        self.collection_error: str | None = None

    def pytest_collection_finish(self, session: pytest.Session) -> None:
        ids = [item.nodeid for item in session.items]
        if len(ids) != len(set(ids)):
            self.collection_error = "duplicate collected case ID"
        self.collected = set(ids)
        for item in session.items:
            try:
                path = item.path.relative_to(REPO).as_posix()
            except ValueError:
                self.collection_error = "collected module is outside the repository"
                continue
            self.modules.add(path)
            if item.get_closest_marker("slow"):
                self.slow.add(item.nodeid)
            if item.get_closest_marker("qualification"):
                self.qualification.add(item.nodeid)

    def pytest_runtest_logreport(self, report: pytest.TestReport) -> None:
        if report.when == "call" and report.passed and not getattr(report, "wasxfail", False):
            if report.nodeid in self.passed:
                self.collection_error = "duplicate passing case report"
            self.passed.add(report.nodeid)


def main() -> int:
    records, problems = source_catalog(REPO, {name: recipe_body(name) for name in RECIPE_FRAGMENTS})
    if problems:
        print(f"taskmesh-py-test status=FAIL reason=static-catalog {problems}", file=sys.stderr)
        return 1
    expected_modules = {
        record["path"] for record in records if record["executing_gate"] == "py-test"
    }
    evidence = CaseEvidence()
    exit_code = int(pytest.main(["tools", "-q", "--strict-markers"], plugins=[evidence]))
    if (
        exit_code != 0
        or evidence.collection_error
        or not evidence.collected
        or evidence.modules != expected_modules
        or evidence.passed != evidence.collected
        or not evidence.slow
        or not evidence.slow <= evidence.passed
    ):
        print(
            "taskmesh-py-test status=FAIL "
            f"pytest_exit={exit_code} selected={len(evidence.collected)} "
            f"passed={len(evidence.passed)} "
            f"missing_modules={sorted(expected_modules - evidence.modules)} "
            f"extra_modules={sorted(evidence.modules - expected_modules)} "
            f"error={evidence.collection_error}",
            file=sys.stderr,
        )
        return 1
    summary = {
        "catalog_digest": catalog_digest(records),
        "collection_digest": digest(sorted(evidence.collected)),
        "execution_digest": digest(sorted(evidence.passed)),
        "modules": len(evidence.modules),
        "cases": len(evidence.collected),
        "slow_cases": len(evidence.slow),
        "qualification_cases": len(evidence.qualification),
    }
    summary["summary_digest"] = digest(summary)
    print(
        "taskmesh-py-test status=PASS "
        + " ".join(f"{key}={value}" for key, value in summary.items())
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
