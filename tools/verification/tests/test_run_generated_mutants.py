"""Generated cargo-mutants artifact and denominator tests."""

from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest

REPO = Path(__file__).resolve().parents[3]
RUNNER = REPO / "tools/verification/run_generated_mutants.py"
V27_FIXTURE = Path(__file__).parent / "fixtures/cargo-mutants-v27"
_spec = importlib.util.spec_from_file_location("run_generated_mutants", RUNNER)
assert _spec and _spec.loader
gm = importlib.util.module_from_spec(_spec)
sys.modules["run_generated_mutants"] = gm
_spec.loader.exec_module(gm)


def write_outcomes(root: Path, values: dict[str, list[str]]) -> None:
    root.mkdir()
    all_mutants = [mutant for category in gm.CATEGORIES for mutant in values.get(category, [])]
    for category in gm.CATEGORIES:
        (root / f"{category}.txt").write_text(
            "\n".join(values.get(category, [])) + ("\n" if values.get(category) else ""),
            encoding="utf-8",
        )
    (root / "mutants.json").write_text(
        json.dumps([{"name": mutant} for mutant in all_mutants]), encoding="utf-8"
    )
    (root / "outcomes.json").write_text(
        json.dumps(
            {
                "outcomes": [
                    {"scenario": "Baseline", "summary": "Success"},
                    *[
                        {
                            "scenario": {"Mutant": {"name": mutant}},
                            "summary": {
                                "caught": "CaughtMutant",
                                "missed": "MissedMutant",
                                "unviable": "Unviable",
                                "timeout": "Timeout",
                            }[category],
                        }
                        for category in gm.CATEGORIES
                        for mutant in values.get(category, [])
                    ],
                ]
            }
        ),
        encoding="utf-8",
    )


def test_real_cargo_mutants_v27_fixture_parses_exact_identity_sets() -> None:
    outcomes = gm.parse_outcomes(V27_FIXTURE)
    assert outcomes == {
        "caught": ["src/lib.rs:2:5: replace positive -> bool with false"],
        "missed": ["src/lib.rs:2:5: replace positive -> bool with true"],
        "unviable": [],
        "timeout": [],
    }


def test_exact_generated_denominator_and_non_green_categories(tmp_path: Path) -> None:
    root = tmp_path / "mutants.out"
    write_outcomes(
        root,
        {
            "caught": ["m1", "m2"],
            "missed": ["m3"],
            "unviable": ["m4"],
            "timeout": ["m5"],
        },
    )
    outcomes = gm.parse_outcomes(root)
    counts, status, problems = gm.classify_generated(outcomes, [])
    assert counts == {
        "caught": 2,
        "missed": 1,
        "unviable": 1,
        "timeout": 1,
        "equivalent": 0,
    }
    assert sum(counts.values()) == 5
    assert status == "FAIL"
    assert problems == ["missed", "unviable", "timeout"]


def test_all_caught_is_the_only_automatic_pass(tmp_path: Path) -> None:
    root = tmp_path / "mutants.out"
    write_outcomes(root, {"caught": ["m1", "m2"]})
    counts, status, problems = gm.classify_generated(gm.parse_outcomes(root), [])
    assert counts["caught"] == 2 and status == "PASS" and problems == []


def test_equivalent_requires_id_reachability_and_reviewer_and_stays_non_pass(
    tmp_path: Path,
) -> None:
    review = tmp_path / "equivalents.json"
    review.write_text(
        json.dumps(
            {
                "equivalents": [
                    {
                        "mutant_id": "m2",
                        "reachability_evidence": "validate rejects class before this branch",
                        "reviewer": "reviewer@example.invalid",
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    equivalents = gm.load_equivalents(review, ["m2"])
    counts, status, problems = gm.classify_generated(
        {"caught": ["m1"], "missed": ["m2"], "unviable": [], "timeout": []}, equivalents
    )
    assert counts == {
        "caught": 1,
        "missed": 0,
        "unviable": 0,
        "timeout": 0,
        "equivalent": 1,
    }
    assert status == "FAIL" and problems == ["equivalent"]


@pytest.mark.parametrize(
    "payload",
    [
        {"equivalents": [{"mutant_id": "m", "reviewer": "r"}]},
        {"equivalents": [{"mutant_id": "unknown", "reachability_evidence": "x", "reviewer": "r"}]},
    ],
)
def test_incomplete_or_detached_equivalent_review_is_rejected(
    tmp_path: Path, payload: dict
) -> None:
    path = tmp_path / "equivalents.json"
    path.write_text(json.dumps(payload), encoding="utf-8")
    with pytest.raises(ValueError):
        gm.load_equivalents(path, ["m"])


def test_missing_zero_or_overlapping_raw_categories_fail_closed(tmp_path: Path) -> None:
    missing = tmp_path / "missing"
    missing.mkdir()
    with pytest.raises(ValueError, match="missing"):
        gm.parse_outcomes(missing)
    zero = tmp_path / "zero"
    write_outcomes(zero, {})
    with pytest.raises(ValueError, match="zero-mutant"):
        gm.parse_outcomes(zero)
    overlap = tmp_path / "overlap"
    write_outcomes(overlap, {"caught": ["m"], "missed": ["m"]})
    with pytest.raises(ValueError, match="both"):
        gm.parse_outcomes(overlap)


def test_red_baseline_and_partial_completion_fail_closed(tmp_path: Path) -> None:
    red = tmp_path / "red"
    write_outcomes(red, {"caught": ["m"]})
    data = json.loads((red / "outcomes.json").read_text(encoding="utf-8"))
    data["outcomes"][0]["summary"] = "Failure"
    (red / "outcomes.json").write_text(json.dumps(data), encoding="utf-8")
    with pytest.raises(ValueError, match="baseline"):
        gm.parse_outcomes(red)

    partial = tmp_path / "partial"
    write_outcomes(partial, {"caught": ["m"]})
    (partial / "mutants.json").write_text(json.dumps([{"name": "m"}, {"name": "n"}]))
    with pytest.raises(ValueError, match="identity sets differ"):
        gm.parse_outcomes(partial)


def test_equal_count_identity_mismatches_fail_closed(tmp_path: Path) -> None:
    planned_mismatch = tmp_path / "planned-mismatch"
    write_outcomes(planned_mismatch, {"caught": ["category-name"]})
    (planned_mismatch / "mutants.json").write_text(json.dumps([{"name": "planned-name"}]))
    with pytest.raises(ValueError, match="identity sets differ"):
        gm.parse_outcomes(planned_mismatch)

    executed_mismatch = tmp_path / "executed-mismatch"
    write_outcomes(executed_mismatch, {"caught": ["planned-name"]})
    data = json.loads((executed_mismatch / "outcomes.json").read_text(encoding="utf-8"))
    data["outcomes"][1]["scenario"]["Mutant"]["name"] = "executed-name"
    (executed_mismatch / "outcomes.json").write_text(json.dumps(data), encoding="utf-8")
    with pytest.raises(ValueError, match="identity sets differ"):
        gm.parse_outcomes(executed_mismatch)


def test_outcome_summary_must_match_category_file(tmp_path: Path) -> None:
    root = tmp_path / "summary-mismatch"
    write_outcomes(root, {"caught": ["m"]})
    data = json.loads((root / "outcomes.json").read_text(encoding="utf-8"))
    data["outcomes"][1]["summary"] = "MissedMutant"
    (root / "outcomes.json").write_text(json.dumps(data), encoding="utf-8")
    with pytest.raises(ValueError, match="caught identities disagree"):
        gm.parse_outcomes(root)


def test_duplicate_planned_or_executed_identity_fails_closed(tmp_path: Path) -> None:
    planned = tmp_path / "planned-duplicate"
    write_outcomes(planned, {"caught": ["m"]})
    (planned / "mutants.json").write_text(
        json.dumps([{"name": "m"}, {"name": "m"}]), encoding="utf-8"
    )
    with pytest.raises(ValueError, match="duplicate mutant names in mutants.json"):
        gm.parse_outcomes(planned)

    executed = tmp_path / "executed-duplicate"
    write_outcomes(executed, {"caught": ["m"]})
    data = json.loads((executed / "outcomes.json").read_text(encoding="utf-8"))
    data["outcomes"].append(data["outcomes"][1])
    (executed / "outcomes.json").write_text(json.dumps(data), encoding="utf-8")
    with pytest.raises(ValueError, match="duplicate mutant name in outcomes.json"):
        gm.parse_outcomes(executed)


def test_all_caught_nonzero_process_cannot_pass() -> None:
    status, problems = gm.apply_process_truth(
        "PASS", [], SimpleNamespace(exit_code=2, signal=None, timed_out=False)
    )
    assert status == "FAIL"
    assert problems == ["invalid_success_process"]


def test_raw_manifest_binds_every_file(tmp_path: Path) -> None:
    (tmp_path / "a").write_text("one", encoding="utf-8")
    (tmp_path / "b").write_text("two", encoding="utf-8")
    manifest = gm.raw_manifest(tmp_path)
    assert [entry["path"] for entry in manifest] == ["a", "b"]
    assert all(entry["size"] == 3 and len(entry["sha256"]) == 64 for entry in manifest)


def test_parallel_generated_jobs_are_rejected_before_campaign_creation() -> None:
    with pytest.raises(SystemExit, match="2"):
        gm.main(["--jobs", "2"])
    manifest = json.loads((REPO / "tools/verification/mutation-gate.json").read_text())
    command = manifest["generated"]["command"]
    assert command[command.index("--jobs") + 1] == "1"
