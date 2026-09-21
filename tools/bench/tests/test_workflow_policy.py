"""Authority split between noisy wall-clock trends and deterministic IAI."""

from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parents[3]
WORKFLOW = REPO / ".github" / "workflows" / "bench.yml"


def workflow() -> dict:
    return yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))


def step(job: dict, name: str) -> dict:
    return next(candidate for candidate in job["steps"] if candidate.get("name") == name)


def test_wall_clock_alerts_are_observational_but_not_silenced() -> None:
    jobs = workflow()["jobs"]
    for job_name, step_name in (
        ("trend-pr-compare", "Compare against history (read-only)"),
        ("trend-main-publish", "Publish trusted-main data point"),
    ):
        trend = step(jobs[job_name], step_name)
        assert trend["id"] == "wall_clock_trend"
        assert trend["continue-on-error"] is True
        assert trend["with"]["fail-on-alert"] is True
        assert trend["with"]["alert-threshold"] == "150%"


def test_wall_clock_alert_still_triggers_pr_diagnostics() -> None:
    job = workflow()["jobs"]["trend-pr-compare"]
    expected = "steps.wall_clock_trend.outcome == 'failure'"
    assert step(job, "Flamegraph + perf counters on regression")["if"] == expected
    assert step(job, "Upload regression profile")["if"] == expected


def test_iai_comparison_remains_an_enforced_job_step() -> None:
    job = workflow()["jobs"]["instruction-count"]
    iai = step(job, "Instruction-count benchmark (thresholds from tools/bench/perf-gate.json)")
    assert iai.get("continue-on-error") is not True
    report = step(job, "Report qualification status")
    assert report.get("continue-on-error") is not True
