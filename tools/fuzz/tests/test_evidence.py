from __future__ import annotations

import importlib.util
from pathlib import Path

import pytest

MODULE_PATH = Path(__file__).parents[1] / "evidence.py"
SPEC = importlib.util.spec_from_file_location("fuzz_evidence", MODULE_PATH)
assert SPEC and SPEC.loader
evidence = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(evidence)


def test_positive_target_witness_is_accepted() -> None:
    text = """taskmesh-fuzz-checkpoint target=wire_formats checkpoint=target_entry
taskmesh-fuzz-checkpoint target=wire_formats checkpoint=snapshot_valid
taskmesh-fuzz-checkpoint target=wire_formats checkpoint=topology_valid
taskmesh-fuzz-checkpoint target=wire_formats checkpoint=class_policy_valid
Done 17 runs in 1 second(s)
"""
    result, problems = evidence.parse_target_log(
        "wire_formats",
        text,
        ["target_entry", "snapshot_valid", "topology_valid", "class_policy_valid"],
        1,
    )
    assert problems == []
    assert result["runs"] == 17
    assert result["valid_inputs"] == 1


def test_decode_all_fail_is_not_a_positive_witness() -> None:
    result, problems = evidence.parse_target_log(
        "wire_formats",
        "taskmesh-fuzz-checkpoint target=wire_formats checkpoint=target_entry\n"
        "Done 10 runs in 1 second(s)\n",
        ["target_entry", "snapshot_valid", "topology_valid", "class_policy_valid"],
        1,
    )
    assert result["valid_inputs"] == 0
    assert any("missing semantic checkpoints" in problem for problem in problems)


def test_noop_and_zero_run_are_rejected() -> None:
    result, problems = evidence.parse_target_log(
        "admission_churn", "Done 0 runs in 0 second(s)\n", ["target_entry", "transition_step"], 1
    )
    assert result["runs"] == 0
    assert any("zero runs" in problem for problem in problems)
    assert any("no valid input" in problem for problem in problems)


def test_zero_duration_is_rejected_even_with_runs_and_witnesses() -> None:
    _, problems = evidence.parse_target_log(
        "admission_churn",
        "taskmesh-fuzz-checkpoint target=admission_churn checkpoint=target_entry\n"
        "taskmesh-fuzz-checkpoint target=admission_churn checkpoint=transition_step\n"
        "Done 1 runs in 1 second(s)\n",
        ["target_entry", "transition_step"],
        0,
    )
    assert any("duration_seconds must be positive" in problem for problem in problems)


def test_missing_or_extra_target_is_rejected() -> None:
    required = {"a": [], "b": []}
    assert evidence.validate_target_set(["a"], required)
    assert evidence.validate_target_set(["a", "b", "c"], required)
    assert evidence.validate_target_set(["a", "a"], required)


def test_missing_positive_seed_is_rejected(tmp_path: Path) -> None:
    producer = {"required_targets": {"wire_formats": ["snapshot_valid"]}}
    targets = {
        "wire_formats": [
            {
                "path": "fuzz/seed-corpus/wire_formats/snapshot.json",
                "sha256": "0" * 64,
            }
        ]
    }
    corpus = {"targets": targets, "digest": evidence.corpus_digest(targets)}
    problems = evidence.validate_manifests(producer, corpus, root=tmp_path)
    assert any("missing corpus seed" in problem for problem in problems)


def test_corpus_manifest_digest_drift_is_rejected() -> None:
    producer = {"required_targets": {"wire_formats": ["snapshot_valid"]}}
    corpus = {"targets": {"wire_formats": []}, "digest": "0" * 64}
    problems = evidence.validate_manifests(producer, corpus)
    assert any("manifest digest mismatch" in problem for problem in problems)


def test_relative_artifact_paths_resolve_from_runner_cwd(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.chdir(tmp_path)
    assert evidence.absolute_from_cwd(Path("target/raw.log")) == tmp_path / "target/raw.log"


def test_artifact_layout_rejects_outside_and_root_paths(tmp_path: Path) -> None:
    root = tmp_path / "target/sep21/v03/fuzz"
    output = root / "local"
    with pytest.raises(ValueError, match="output directory must be below"):
        evidence.validate_artifact_layout(
            output_dir=tmp_path / "outside",
            logs_dir=tmp_path / "outside/raw",
            corpus_dir=tmp_path / "outside/seed-corpus",
            root=root,
        )
    with pytest.raises(ValueError, match="must name a directory below"):
        evidence.validate_artifact_layout(
            output_dir=root,
            logs_dir=root / "raw",
            corpus_dir=root / "seed-corpus",
            root=root,
        )
    assert evidence.validate_artifact_layout(
        output_dir=output,
        logs_dir=output / "raw",
        corpus_dir=output / "seed-corpus",
        root=root,
    ) == (output.resolve(), (output / "raw").resolve(), (output / "seed-corpus").resolve())


def test_artifact_layout_rejects_symlink_escape(tmp_path: Path) -> None:
    root = tmp_path / "target/sep21/v03/fuzz"
    output = root / "local"
    outside = tmp_path / "outside"
    output.mkdir(parents=True)
    outside.mkdir()
    (output / "raw").symlink_to(outside, target_is_directory=True)
    with pytest.raises(ValueError, match="logs directory must be below"):
        evidence.validate_artifact_layout(
            output_dir=output,
            logs_dir=output / "raw",
            corpus_dir=output / "seed-corpus",
            root=root,
        )


def test_prepare_corpus_rejects_unsafe_delete_target(tmp_path: Path) -> None:
    root = tmp_path / "target/sep21/v03/fuzz"
    unsafe = tmp_path / "outside"
    unsafe.mkdir()
    sentinel = unsafe / "keep.txt"
    sentinel.write_text("keep")
    with pytest.raises(ValueError, match="corpus directory must be below"):
        evidence.prepare_corpus(
            {"targets": {}},
            unsafe,
            output_dir=root / "local",
            artifact_root=root,
        )
    assert sentinel.read_text() == "keep"


def test_source_change_during_fuzz_is_rejected() -> None:
    before = {"head": "a", "paths_digest": "one", "dirty": True}
    assert evidence.source_drift_problems(before, dict(before)) == []
    after = {**before, "paths_digest": "two"}
    assert evidence.source_drift_problems(before, after) == [
        "source changed during fuzz execution: paths_digest"
    ]
