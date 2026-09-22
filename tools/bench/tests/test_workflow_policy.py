"""The retained manual benchmark workflow enforces deterministic IAI."""

from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parents[3]
WORKFLOW = REPO / ".github" / "workflows" / "bench.yml"


def workflow() -> dict:
    return yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))


def step(job: dict, name: str) -> dict:
    return next(candidate for candidate in job["steps"] if candidate.get("name") == name)


def test_iai_comparison_remains_an_enforced_job_step() -> None:
    jobs = workflow()["jobs"]
    assert set(jobs) == {"instruction-count"}
    job = jobs["instruction-count"]
    iai = step(job, "Instruction-count benchmark (thresholds from tools/bench/perf-gate.json)")
    assert iai.get("continue-on-error") is not True
    report = step(job, "Report qualification status")
    assert report.get("continue-on-error") is not True
