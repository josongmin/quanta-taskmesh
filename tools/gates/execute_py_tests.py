#!/usr/bin/env python3
"""Run the CI pytest surface and retain its collected/executed denominator."""

from __future__ import annotations

import hashlib
import json
import sys
from collections import defaultdict
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
if str(REPO) not in sys.path:
    sys.path.insert(0, str(REPO))

from tools.gates.target_catalog import (  # noqa: E402
    RECIPE_FRAGMENTS,
    catalog_digest,
    source_catalog,
)
from tools.gates.validate_inventory import recipe_body  # noqa: E402
from tools.qualification.receipt import source_identity  # noqa: E402

REPORT = REPO / "target/verification/pytest-execution.json"


def digest(value: object) -> str:
    payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(payload).hexdigest()


class Tracker:
    def __init__(self) -> None:
        self.selected: dict[str, dict] = {}
        self.outcomes: dict[str, list[dict]] = defaultdict(list)
        self.collect_errors: list[str] = []

    def pytest_collection_modifyitems(self, items: list[pytest.Item]) -> None:
        for item in items:
            path = Path(str(item.path)).relative_to(REPO).as_posix()
            if item.nodeid in self.selected:
                raise ValueError(f"duplicate pytest case {item.nodeid}")
            self.selected[item.nodeid] = {
                "module": path,
                "slow": item.get_closest_marker("slow") is not None,
                "qualification": item.get_closest_marker("qualification") is not None,
            }

    def pytest_collectreport(self, report: pytest.CollectReport) -> None:
        if report.failed or report.skipped:
            self.collect_errors.append(f"{report.nodeid}: {report.outcome}")

    def pytest_runtest_logreport(self, report: pytest.TestReport) -> None:
        self.outcomes[report.nodeid].append({
            "phase": report.when,
            "outcome": report.outcome,
            "xfail": bool(getattr(report, "wasxfail", False)),
        })


def reconcile(
    selected: dict[str, dict], outcomes: dict[str, list[dict]],
    expected_modules: set[str], collect_errors: list[str],
) -> tuple[list[str], list[dict]]:
    if collect_errors:
        raise ValueError(f"pytest collection errors/skips: {collect_errors}")
    modules = {item["module"] for item in selected.values()}
    if modules != expected_modules:
        raise ValueError(
            f"pytest module denominator mismatch: missing={sorted(expected_modules - modules)} "
            f"extra={sorted(modules - expected_modules)}"
        )
    if not selected:
        raise ValueError("pytest selected no cases")
    passed: list[str] = []
    excluded: list[dict] = []
    by_module: dict[str, int] = defaultdict(int)
    for nodeid, details in sorted(selected.items()):
        reports = outcomes.get(nodeid, [])
        if any(report["outcome"] == "failed" for report in reports):
            raise ValueError(f"pytest case failed: {nodeid}")
        calls = [report for report in reports if report["phase"] == "call"]
        if len(calls) == 1 and calls[0]["outcome"] == "passed" and not calls[0]["xfail"]:
            passed.append(nodeid)
            by_module[details["module"]] += 1
        elif any(report["outcome"] == "skipped" or report["xfail"] for report in reports):
            excluded.append({"case": nodeid, "reason": "skipped_or_xfail"})
        else:
            raise ValueError(f"pytest case has no terminal result: {nodeid}")
    empty = sorted(module for module in expected_modules if by_module[module] == 0)
    if empty:
        raise ValueError(f"pytest module has no passed cases: {empty}")
    return passed, excluded


def main() -> None:
    REPORT.unlink(missing_ok=True)
    before = source_identity(REPO)
    if before["dirty"]:
        raise ValueError("qualified pytest execution requires clean source")
    recipes = {name: recipe_body(name) for name in RECIPE_FRAGMENTS}
    records, problems = source_catalog(REPO, recipes)
    if problems:
        raise ValueError(f"target catalog failed: {problems}")
    expected_modules = {
        record["path"] for record in records if record["executing_gate"] == "py-test"
    }
    tracker = Tracker()
    args = ["tools", "-q", "--strict-markers"]
    result = pytest.main(list(args), plugins=[tracker])
    if result != pytest.ExitCode.OK:
        raise ValueError(f"pytest exited {int(result)}")
    passed, excluded = reconcile(
        tracker.selected, tracker.outcomes, expected_modules, tracker.collect_errors
    )
    after = source_identity(REPO)
    if any(before[key] != after[key] for key in ("head", "tree", "paths_digest", "dirty")):
        raise ValueError("pytest source changed during execution")
    report = {
        "schema_version": 1,
        "source": before,
        "runner": "pytest",
        "runner_version": pytest.__version__,
        "command": ["pytest", *args],
        "catalog_digest": catalog_digest(records),
        "modules": sorted(expected_modules),
        "selected": sorted(tracker.selected),
        "passed": passed,
        "excluded": excluded,
        "slow": sorted(nodeid for nodeid, item in tracker.selected.items() if item["slow"]),
        "qualification": sorted(
            nodeid for nodeid, item in tracker.selected.items() if item["qualification"]
        ),
    }
    report["selection_digest"] = digest(report["selected"])
    report["execution_digest"] = digest({"passed": passed, "excluded": excluded})
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    REPORT.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(
        f"taskmesh-py-test status=PASS runner=pytest runner_version={pytest.__version__} "
        f"modules={len(expected_modules)} selected={len(tracker.selected)} "
        f"passed={len(passed)} excluded={len(excluded)} slow={len(report['slow'])} "
        f"qualification={len(report['qualification'])} "
        f"catalog_digest={report['catalog_digest']} "
        f"selection_digest={report['selection_digest']} "
        f"execution_digest={report['execution_digest']}"
    )


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError) as exc:
        print(f"pytest denominator FAIL: {exc}", file=sys.stderr)
        raise SystemExit(1) from exc
