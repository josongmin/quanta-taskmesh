"""The linked bench suite must select every physical integration source."""

from __future__ import annotations

from pathlib import Path

import pytest

from tools.gates import bench_suite, target_catalog

REPO = Path(__file__).resolve().parents[3]
ORIGINAL_MEMBERS = {
    "artifact_publish.rs",
    "fairness_property.rs",
    "hellgate.rs",
    "host_closed_loop.rs",
    "host_composite.rs",
    "host_generator_calibration.rs",
    "host_load_accounting.rs",
    "host_load_integration.rs",
    "host_local.rs",
    "host_minimal_recorder.rs",
    "host_simulator_comparison.rs",
    "host_stability_integration.rs",
    "inferno.rs",
}


def test_repository_keeps_every_original_integration_source_active() -> None:
    assert ORIGINAL_MEMBERS <= set(bench_suite.checked_members(REPO))


def fixture(root: Path) -> tuple[Path, dict]:
    package_root = root / "crates" / "taskmesh-bench"
    tests = package_root / "tests"
    tests.mkdir(parents=True)
    members = ("alpha.rs", "beta.rs")
    manifest = package_root / "Cargo.toml"
    manifest.write_text(
        '[package]\nname = "taskmesh-bench"\nversion = "0.3.0"\n'
        'autotests = false\n'
        '[package.metadata.taskmesh_bench_suite]\n'
        'target = "bench_suite"\n'
        'members = ["alpha.rs", "beta.rs"]\n'
        '[[test]]\nname = "bench_suite"\npath = "tests/bench_suite.rs"\n',
        encoding="utf-8",
    )
    for member in members:
        (tests / member).write_text(f"#[test]\nfn {member[:-3]}_case() {{}}\n")
    (tests / "bench_suite.rs").write_text(bench_suite.render(members))
    package = {
        "name": "taskmesh-bench",
        "manifest_path": str(manifest),
        "targets": [{
            "kind": ["test"],
            "name": "bench_suite",
            "src_path": str(tests / "bench_suite.rs"),
            "test": True,
        }],
    }
    return tests, package


def test_exact_manifest_modules_and_cargo_target_are_selected(tmp_path: Path) -> None:
    tests, package = fixture(tmp_path)
    assert bench_suite.checked_members(tmp_path) == ("alpha.rs", "beta.rs")
    assert bench_suite.check_cargo_targets(tmp_path, package) == ("alpha.rs", "beta.rs")
    assert target_catalog.filesystem_target_problems([package], tmp_path) == []
    assert bench_suite.selected_case(
        tmp_path, "crates/taskmesh-bench/tests/alpha.rs", "alpha_case"
    ) == "alpha::alpha_case"
    assert bench_suite.selected_case(tmp_path, "crates/other/tests/alpha.rs", "alpha_case") is None
    assert (tests / "alpha.rs").is_file()


@pytest.mark.parametrize(
    ("replacement", "message"),
    [
        ('members = ["alpha.rs", "alpha.rs"]', "unique sorted"),
        ('members = ["beta.rs", "alpha.rs"]', "unique sorted"),
        ('members = ["../other.rs", "beta.rs"]', "unique sorted"),
        ('members = ["alpha.rs"]', "physical sources differ"),
        ('target = "other"', "inventory target differs"),
        ('autotests = true', "autotests=false"),
        ('path = "tests/other.rs"', "one explicit active test target"),
    ],
)
def test_inventory_changes_fail_closed(
    tmp_path: Path, replacement: str, message: str
) -> None:
    _, package = fixture(tmp_path)
    manifest = Path(package["manifest_path"])
    text = manifest.read_text()
    old = (
        'members = ["alpha.rs", "beta.rs"]' if replacement.startswith("members")
        else 'target = "bench_suite"' if replacement.startswith("target")
        else 'autotests = false' if replacement.startswith("autotests")
        else 'path = "tests/bench_suite.rs"'
    )
    manifest.write_text(text.replace(old, replacement))
    with pytest.raises(ValueError, match=message):
        bench_suite.checked_members(tmp_path)


def test_orphan_missing_symlink_and_nested_sources_fail_closed(tmp_path: Path) -> None:
    tests, _ = fixture(tmp_path)
    (tests / "orphan.rs").write_text("#[test]\nfn orphan() {}\n")
    with pytest.raises(ValueError, match="physical sources differ"):
        bench_suite.checked_members(tmp_path)
    (tests / "orphan.rs").unlink()
    (tests / "alpha.rs").unlink()
    with pytest.raises(ValueError, match="physical sources differ"):
        bench_suite.checked_members(tmp_path)
    (tests / "alpha.rs").symlink_to(tests / "beta.rs")
    with pytest.raises(ValueError, match="symlink"):
        bench_suite.checked_members(tmp_path)
    (tests / "alpha.rs").unlink()
    (tests / "alpha.rs").write_text("#[test]\nfn alpha_case() {}\n")
    nested = tests / "nested"
    nested.mkdir()
    (nested / "hidden.rs").write_text("#[test]\nfn hidden() {}\n")
    with pytest.raises(ValueError, match="unregistered nested source"):
        bench_suite.checked_members(tmp_path)


def test_conditional_or_duplicate_module_declarations_fail_closed(tmp_path: Path) -> None:
    tests, _ = fixture(tmp_path)
    suite = tests / "bench_suite.rs"
    for content in (
        suite.read_text().replace("mod beta;", "mod alpha;"),
        '#![cfg(feature = "hidden")]\n' + suite.read_text(),
        suite.read_text().replace('#[path = "beta.rs"]\nmod beta;\n', ""),
    ):
        suite.write_text(content)
        with pytest.raises(ValueError, match="module declarations differ"):
            bench_suite.checked_members(tmp_path)


@pytest.mark.parametrize(
    "attribute",
    [
        '#![cfg(feature = "hidden")]\n',
        '#![ cfg(feature = "hidden")]\n',
        '#![\n cfg(feature = "hidden")\n]\n',
        '# ! [cfg(feature = "hidden")]\n',
        '#/* comment */![cfg(feature = "hidden")]\n',
        '#/* nested /* comment */ comment */ ![cfg(feature = "hidden")]\n',
        '#// comment\n![cfg(feature = "hidden")]\n',
        '#![allow(dead_code)]\n',
    ],
)
def test_inner_attribute_cannot_silently_remove_member_tests(
    tmp_path: Path, attribute: str
) -> None:
    tests, _ = fixture(tmp_path)
    member = tests / "alpha.rs"
    member.write_text(attribute + member.read_text())
    with pytest.raises(ValueError, match="inner attribute"):
        bench_suite.checked_members(tmp_path)


def test_symlink_in_root_ancestor_or_crates_directory_fails_closed(tmp_path: Path) -> None:
    actual_root = tmp_path / "actual" / "repo"
    fixture(actual_root)
    linked_parent = tmp_path / "linked-parent"
    linked_parent.symlink_to(actual_root.parent, target_is_directory=True)
    with pytest.raises(ValueError, match="symlink ancestor"):
        bench_suite.checked_members(linked_parent / "repo")

    other_root = tmp_path / "other"
    other_root.mkdir()
    (other_root / "crates").symlink_to(actual_root / "crates", target_is_directory=True)
    with pytest.raises(ValueError, match="symlink"):
        bench_suite.checked_members(other_root)


@pytest.mark.parametrize("change", ["missing", "duplicate", "inactive", "feature", "outside"])
def test_cargo_target_drift_fails_closed(tmp_path: Path, change: str) -> None:
    _, package = fixture(tmp_path)
    if change == "missing":
        package["targets"] = []
    elif change == "duplicate":
        package["targets"].append(package["targets"][0].copy())
    elif change == "inactive":
        package["targets"][0]["test"] = False
    elif change == "feature":
        package["targets"][0]["required-features"] = ["hidden"]
    else:
        package["targets"][0]["src_path"] = str(tmp_path / "other" / "bench_suite.rs")
    with pytest.raises(ValueError, match="Cargo metadata"):
        bench_suite.check_cargo_targets(tmp_path, package)
    assert any(
        "bench suite Cargo metadata" in problem
        for problem in target_catalog.filesystem_target_problems([package], tmp_path)
    )


def test_scenario_binding_rejects_inactive_file_and_noncanonical_case(tmp_path: Path) -> None:
    fixture(tmp_path)
    with pytest.raises(ValueError, match="not an active suite member"):
        bench_suite.selected_case(tmp_path, "crates/taskmesh-bench/tests/hidden.rs", "case")
    with pytest.raises(ValueError, match="invalid identifier"):
        bench_suite.selected_case(tmp_path, "crates/taskmesh-bench/tests/alpha.rs", "other::case")
