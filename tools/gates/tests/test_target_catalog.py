"""Catalog owner/executor drift must be visible before a green gate receipt."""

from __future__ import annotations

import copy
import json
from functools import cache
from pathlib import Path

from tools.gates import target_catalog as tc
from tools.gates.validate_inventory import recipe_body

REPO = Path(__file__).resolve().parents[3]


@cache
def inputs() -> tuple:
    return (
        tc.metadata(REPO),
        tc.metadata(REPO, fuzz=True),
        json.loads((REPO / "tools/fuzz/producer-manifest.json").read_text()),
        json.loads((REPO / "tools/bench/perf-gate.json").read_text()),
        {name: recipe_body(name) for name in tc.RECIPE_FRAGMENTS},
        (REPO / "tools/modelcheck/run.py").read_text(),
        tc.python_test_modules(REPO),
    )


def catalog_with(*, root_metadata=None, fuzz_metadata=None, fuzz_manifest=None, modules=None):
    source = inputs()
    return tc.catalog(
        source[0] if root_metadata is None else root_metadata,
        source[1] if fuzz_metadata is None else fuzz_metadata,
        source[2] if fuzz_manifest is None else fuzz_manifest,
        source[3],
        source[4],
        source[5],
        source[6] if modules is None else modules,
        root=REPO,
    )


def test_new_ordinary_integration_target_gets_test_owner() -> None:
    meta = copy.deepcopy(inputs()[0])
    package = next(package for package in meta["packages"] if package["name"] == "taskmesh")
    target = copy.deepcopy(
        next(target for target in package["targets"] if target["kind"] == ["test"])
    )
    target["name"] = "new_contract"
    target["src_path"] = str(REPO / "crates/taskmesh/tests/new_contract.rs")
    package["targets"].append(target)
    records, problems = catalog_with(root_metadata=meta)
    assert problems == []
    assert any(
        record["target"] == "new_contract" and record["executing_gate"] == "test"
        for record in records
    )


def test_new_feature_required_test_without_executor_fails_closed() -> None:
    meta = copy.deepcopy(inputs()[0])
    package = next(package for package in meta["packages"] if package["name"] == "taskmesh")
    target = copy.deepcopy(
        next(target for target in package["targets"] if target["kind"] == ["test"])
    )
    target["name"] = "hidden_feature_case"
    target["src_path"] = str(REPO / "crates/taskmesh/tests/hidden_feature_case.rs")
    target["required-features"] = ["rayon"]
    package["targets"].append(target)
    _, problems = catalog_with(root_metadata=meta)
    assert any(
        "hidden_feature_case: feature-required test has no registered executor" in p
        for p in problems
    )


def test_registered_rayon_matrix_target_cannot_disappear() -> None:
    meta = copy.deepcopy(inputs()[0])
    package = next(package for package in meta["packages"] if package["name"] == "taskmesh")
    package["targets"] = [
        target for target in package["targets"]
        if target["name"] != "hardening_executor_authority"
    ]
    _, problems = catalog_with(root_metadata=meta)
    assert any(
        "test-rayon: missing or disabled taskmesh/test/hardening_executor_authority" in problem
        for problem in problems
    )


def test_new_fuzz_bin_requires_producer_registration() -> None:
    meta = copy.deepcopy(inputs()[1])
    package = next(package for package in meta["packages"] if package["name"] == "taskmesh-fuzz")
    target = copy.deepcopy(
        next(target for target in package["targets"] if target["kind"] == ["bin"])
    )
    target["name"] = "new_fuzz_target"
    target["src_path"] = str(REPO / "fuzz/fuzz_targets/new_fuzz_target.rs")
    package["targets"].append(target)
    _, problems = catalog_with(fuzz_metadata=meta)
    assert any(
        "new_fuzz_target" in problem and "differ from producer targets" in problem
        for problem in problems
    )


def test_python_module_rename_changes_catalog_denominator() -> None:
    modules = inputs()[6]
    renamed = ["tools/tests/test_renamed.py" if path == modules[0] else path for path in modules]
    original, problems = catalog_with()
    changed, changed_problems = catalog_with(modules=renamed)
    assert problems == changed_problems == []
    assert tc.catalog_digest(original) != tc.catalog_digest(changed)


def test_mod_rs_support_file_is_not_a_python_test_module(tmp_path: Path) -> None:
    tests = tmp_path / "tools/tests"
    tests.mkdir(parents=True)
    (tests / "test_owner.py").write_text("def test_one(): pass\n")
    (tests / "mod.rs").write_text("mod helper;\n")
    (tests / "helper.py").write_text("def helper(): pass\n")
    assert tc.python_test_modules(tmp_path) == ["tools/tests/test_owner.py"]


def test_disabled_autotests_cannot_hide_an_integration_source(tmp_path: Path) -> None:
    package_root = tmp_path / "crates" / "demo"
    (package_root / "tests").mkdir(parents=True)
    (package_root / "Cargo.toml").write_text(
        '[package]\nname = "demo"\nversion = "0.1.0"\nautotests = false\n'
    )
    (package_root / "tests" / "hidden.rs").write_text('compile_error!("hidden");\n')
    package = {"name": "demo", "manifest_path": str(package_root / "Cargo.toml"), "targets": []}
    problems = tc.filesystem_target_problems([package], tmp_path)
    assert any("tests/hidden.rs is absent from Cargo metadata" in problem for problem in problems)


def test_bench_false_cannot_pass_the_smoke_census(tmp_path: Path) -> None:
    package_root = tmp_path / "crates" / "demo"
    (package_root / "benches").mkdir(parents=True)
    bench = package_root / "benches" / "hidden.rs"
    bench.write_text('compile_error!("hidden");\n')
    (package_root / "Cargo.toml").write_text(
        '[package]\nname = "demo"\nversion = "0.1.0"\n'
        '[[bench]]\nname = "hidden"\nharness = false\nbench = false\n'
    )
    package = {
        "name": "demo", "manifest_path": str(package_root / "Cargo.toml"),
        "targets": [{"kind": ["bench"], "src_path": str(bench)}],
    }
    assert any(
        "disables cargo bench execution" in problem
        for problem in tc.filesystem_target_problems([package], tmp_path)
    )
