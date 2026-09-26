"""Focused generated-mutation scope and cache safety tests."""

from __future__ import annotations

import importlib.util
import json
import shutil
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]
RUNNER = REPO / "tools/verification/run_focused_mutants.py"
spec = importlib.util.spec_from_file_location("run_focused_mutants", RUNNER)
assert spec and spec.loader
focused = importlib.util.module_from_spec(spec)
sys.modules["run_focused_mutants"] = focused
spec.loader.exec_module(focused)


def test_command_scopes_mutants_and_keeps_package_tests() -> None:
    argv = focused.command(
        ["taskmesh-engine"],
        ["crates/taskmesh-engine/src/features/admission.rs"],
        Path("/tmp/raw"),
        1,
        300,
        Path("/tmp/changed.diff"),
        "admission_queue",
        "admission_queue::tests::rejects_when_full",
    )
    assert argv[argv.index("--package") + 1] == "taskmesh-engine"
    assert argv[argv.index("--file") + 1] == "crates/taskmesh-engine/src/features/admission.rs"
    assert argv[argv.index("--in-diff") + 1] == "/tmp/changed.diff"
    assert "--cargo-arg=--test" in argv
    assert "--cargo-arg=admission_queue" in argv
    assert "--cargo-test-arg=--" in argv
    assert "--cargo-test-arg=admission_queue::tests::rejects_when_full" in argv
    assert "--cargo-test-arg=--exact" in argv
    assert "--baseline" in argv and argv[argv.index("--baseline") + 1] == "run"
    assert "--workspace" not in argv


def test_successful_mutant_outcomes_cannot_hide_a_failed_producer() -> None:
    assert focused.diagnostic_status("PASS", True, 0) == "DIAGNOSTIC_PASS"
    assert focused.diagnostic_status("PASS", True, 1) == "DIAGNOSTIC_FAIL"
    assert focused.diagnostic_status("PASS", False, 0) == "DIAGNOSTIC_FAIL"


def test_cache_fingerprint_tracks_selected_dependency_closure_only(
    monkeypatch, tmp_path: Path
) -> None:
    (tmp_path / "engine.rs").write_text("before", encoding="utf-8")
    (tmp_path / "unrelated.rs").write_text("unrelated", encoding="utf-8")
    monkeypatch.setattr(focused, "package_dependency_files", lambda *_: ["engine.rs"])
    monkeypatch.setattr(focused, "tool_identity", lambda *_: [{"name": "test", "version": "1"}])
    initial = focused.cache_fingerprint(tmp_path, ["taskmesh-engine"], [], 1, None, 86400, None)
    (tmp_path / "unrelated.rs").write_text("changed", encoding="utf-8")
    assert (
        focused.cache_fingerprint(tmp_path, ["taskmesh-engine"], [], 1, None, 86400, None)
        == initial
    )
    (tmp_path / "engine.rs").write_text("changed", encoding="utf-8")
    assert (
        focused.cache_fingerprint(tmp_path, ["taskmesh-engine"], [], 1, None, 86400, None)
        != initial
    )
    changed_bytes = focused.cache_fingerprint(
        tmp_path, ["taskmesh-engine"], [], 1, None, 86400, None
    )
    (tmp_path / "engine.rs").chmod(0o600)
    assert (
        focused.cache_fingerprint(tmp_path, ["taskmesh-engine"], [], 1, None, 86400, None)
        != changed_bytes
    )
    assert (
        focused.cache_fingerprint(tmp_path, ["taskmesh-engine"], [], 1, None, 300, None) != initial
    )
    assert (
        focused.cache_fingerprint(tmp_path, ["taskmesh-engine"], [], 1, None, 86400, "diff")
        != initial
    )
    assert (
        focused.cache_fingerprint(
            tmp_path, ["taskmesh-engine"], [], 1, None, 86400, None, "admission_queue"
        )
        != initial
    )
    assert focused.cache_fingerprint(
        tmp_path,
        ["taskmesh-engine"],
        [],
        1,
        None,
        86400,
        None,
        "admission_queue",
        "admission_queue::tests::rejects_when_full",
    ) != focused.cache_fingerprint(
        tmp_path, ["taskmesh-engine"], [], 1, None, 86400, None, "admission_queue"
    )


def test_cache_fingerprint_tracks_selected_symlink_target(monkeypatch, tmp_path: Path) -> None:
    (tmp_path / "first.rs").write_text("same bytes", encoding="utf-8")
    (tmp_path / "second.rs").write_text("same bytes", encoding="utf-8")
    link = tmp_path / "selected.rs"
    link.symlink_to("first.rs")
    monkeypatch.setattr(focused, "package_dependency_files", lambda *_: ["selected.rs"])
    monkeypatch.setattr(focused, "tool_identity", lambda *_: [{"name": "test", "version": "1"}])
    first = focused.cache_fingerprint(tmp_path, ["taskmesh-engine"], [], 1, None, 86400, None)
    link.unlink()
    link.symlink_to("second.rs")
    assert (
        focused.cache_fingerprint(tmp_path, ["taskmesh-engine"], [], 1, None, 86400, None) != first
    )


def test_library_test_target_is_forwarded_as_a_cargo_argument() -> None:
    argv = focused.command(["taskmesh-engine"], [], Path("/tmp/raw"), 1, None, test_target="lib")
    assert "--cargo-arg=--lib" in argv


def test_cargo_home_source_configuration_is_part_of_cache_identity(
    monkeypatch, tmp_path: Path
) -> None:
    cargo_home = tmp_path / "cargo-home"
    cargo_home.mkdir()
    config = cargo_home / "config.toml"
    config.write_text("[net]\noffline = true\n", encoding="utf-8")
    monkeypatch.setenv("CARGO_HOME", str(cargo_home))
    first = focused.cargo_user_config_identity()
    config.write_text("[net]\noffline = false\n", encoding="utf-8")
    assert focused.cargo_user_config_identity() != first


def test_cache_dependency_closure_includes_execution_and_digest_owners() -> None:
    files = set(focused.package_dependency_files(REPO, ["taskmesh-engine"]))
    assert "crates/taskmesh-contract/src/lib.rs" in files
    assert "crates/taskmesh-engine/tests/prop_invariants.rs" in files
    assert "tools/process_supervisor.py" in files
    assert "tools/qualification/evidence.py" in files


def test_cache_dependency_closure_accepts_symlinked_workspace_root(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.symlink_to(REPO, target_is_directory=True)

    files = set(focused.package_dependency_files(workspace, ["taskmesh-engine"]))

    assert "crates/taskmesh-engine/src/lib.rs" in files


def test_doc_example_build_inputs_outside_package_are_in_cache_closure() -> None:
    files = set(focused.package_dependency_files(REPO, ["taskmesh-doc-examples"]))
    assert {
        "README.md",
        "docs/taskmesh-external-interface.md",
        "CHANGELOG.md",
    } <= files


def test_doc_example_external_input_changes_invalidate_cache_and_drift_fails_closed(
    monkeypatch, tmp_path: Path
) -> None:
    build_script = tmp_path / "tools/doc-examples/build.rs"
    build_script.parent.mkdir(parents=True)
    build_script.write_text('const SOURCES: &[&str] = &[\n    "README.md",\n];\n', encoding="utf-8")
    readme = tmp_path / "README.md"
    readme.write_text("before", encoding="utf-8")
    monkeypatch.setattr(
        focused,
        "package_dependency_files",
        lambda source, _packages: focused.doc_example_build_inputs(source),
    )
    monkeypatch.setattr(focused, "tool_identity", lambda *_: [{"name": "test", "version": "1"}])
    before = focused.cache_fingerprint(
        tmp_path, ["taskmesh-doc-examples"], [], 1, None, 86400, None
    )
    readme.write_text("after", encoding="utf-8")
    assert (
        focused.cache_fingerprint(tmp_path, ["taskmesh-doc-examples"], [], 1, None, 86400, None)
        != before
    )
    build_script.write_text('const SOURCES: &[&str] = include!("sources.rs");\n', encoding="utf-8")
    with pytest.raises(ValueError, match="declaration is missing"):
        focused.doc_example_build_inputs(tmp_path)


def test_cached_report_requires_matching_key_and_complete_execution(tmp_path: Path) -> None:
    run = tmp_path / "run"
    raw = run / "raw"
    shutil.copytree(REPO / "tools/verification/tests/fixtures/cargo-mutants-v27", raw)
    path = run / "focused-mutation-report.json"
    outcomes = focused.parse_outcomes(raw)
    counts, classified_status, _ = focused.classify_generated(outcomes, [])
    report = {
        "schema_version": 1,
        "kind": "focused-generated-cargo-mutants-diagnostic",
        "cache_key": "right",
        "complete": True,
        "report_path": str(path),
        "status": "DIAGNOSTIC_PASS" if classified_status == "PASS" else "DIAGNOSTIC_FAIL",
        "parse_error": None,
        "source": {
            "source_head": "a" * 40,
            "source_dirty": True,
            "source_before_sha256": "b" * 64,
            "snapshot_sha256": "b" * 64,
        },
        "source_unchanged": True,
        "outcomes": outcomes,
        "counts": counts,
        "denominator": sum(counts.values()),
        "raw_artifacts": focused.raw_manifest(raw),
        "process": {
            "exit_code": 0,
            "signal": None,
            "timed_out": False,
            "interrupted_by_signal": None,
        },
    }
    path.write_text(json.dumps(report), encoding="utf-8")
    assert focused.reusable_report(path, "right")["status"] == report["status"]
    assert focused.reusable_report(path, "other") is None
    missing_source = {key: value for key, value in report.items() if key != "source"}
    path.write_text(json.dumps(missing_source), encoding="utf-8")
    assert focused.reusable_report(path, "right") is None
    inconsistent_source = {**report, "source": {**report["source"], "snapshot_sha256": "c" * 64}}
    path.write_text(json.dumps(inconsistent_source), encoding="utf-8")
    assert focused.reusable_report(path, "right") is None
    path.write_text(json.dumps(report), encoding="utf-8")
    report["process"]["timed_out"] = True
    path.write_text(json.dumps(report), encoding="utf-8")
    assert focused.reusable_report(path, "right") is None
    report["process"]["timed_out"] = False
    report["complete"] = False
    path.write_text(json.dumps(report), encoding="utf-8")
    assert focused.reusable_report(path, "right") is None


def test_cache_lookup_treats_pruned_report_as_a_miss(monkeypatch, tmp_path: Path) -> None:
    report = tmp_path / "cache" / "run" / "focused-mutation-report.json"
    report.parent.mkdir(parents=True)
    report.write_text("{}", encoding="utf-8")
    original_stat = Path.stat

    def stat_with_prune_race(path: Path, *args, **kwargs):
        if path == report:
            raise FileNotFoundError(path)
        return original_stat(path, *args, **kwargs)

    monkeypatch.setattr(Path, "stat", stat_with_prune_race)
    assert focused.find_reusable_report(report.parent.parent, "key") is None


def test_invalid_utf8_cached_report_is_a_cache_miss(tmp_path: Path) -> None:
    report = tmp_path / "focused-mutation-report.json"
    report.write_bytes(b"\xff")
    assert focused.reusable_report(report, "key") is None


def test_focused_cache_rejects_a_symlinked_target_parent_before_preflight(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repo = tmp_path / "repo"
    repo.mkdir()
    external = tmp_path / "external"
    external.mkdir()
    (repo / "target").symlink_to(external, target_is_directory=True)
    monkeypatch.setattr(focused, "REPO", repo)
    monkeypatch.setattr(focused, "CACHE_ROOT", repo / "target/verification/mutations/focused")

    def preflight_must_not_run(*_args, **_kwargs):
        raise AssertionError("preflight ran before cache path validation")

    monkeypatch.setattr(focused, "cache_fingerprint", preflight_must_not_run)

    with pytest.raises(ValueError, match="symlink"):
        focused.main(["--package", "taskmesh-engine"])


def test_cache_lookup_does_not_follow_a_symlinked_entry(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    external = tmp_path / "external"
    report = external / "run/focused-mutation-report.json"
    report.parent.mkdir(parents=True)
    report.write_text("{}", encoding="utf-8")
    entry = tmp_path / "cache-entry"
    entry.symlink_to(external, target_is_directory=True)

    def outside_report_must_not_be_read(*_args, **_kwargs):
        raise AssertionError("symlinked cache entry was followed")

    monkeypatch.setattr(focused, "reusable_report", outside_report_must_not_be_read)
    assert focused.find_reusable_report(entry, "key") is None
