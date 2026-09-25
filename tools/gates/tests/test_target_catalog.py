"""Static target ownership must follow Cargo and Python discovery."""

from __future__ import annotations

import copy
import json

import pytest

from tools.gates import target_catalog as tc
from tools.gates.validate_inventory import recipe_body


@pytest.fixture(scope="module")
def inputs() -> tuple:
    root = tc.REPO
    python_modules = sorted(
        path.relative_to(root).as_posix()
        for path in (root / "tools").rglob("*.py")
        if path.name.startswith("test_") or path.name.endswith("_test.py")
    )
    return (
        tc.metadata(root),
        tc.metadata(root, fuzz=True),
        json.loads((root / "tools/fuzz/producer-manifest.json").read_text()),
        json.loads((root / "tools/bench/perf-gate.json").read_text()),
        {name: recipe_body(name) for name in tc.RECIPE_FRAGMENTS},
        (root / "tools/modelcheck/run.py").read_text(),
        python_modules,
    )


def catalog(inputs: tuple) -> tuple[list[dict], list[str]]:
    return tc.catalog(*inputs)


def test_rayon_matrix_cases_have_one_catalog_owner(inputs: tuple) -> None:
    records, problems = catalog(inputs)
    assert problems == []
    matrix = [record for record in records if record["executing_gate"] == "test-rayon"]
    assert len(matrix) == len(tc.RAYON_SCOPES) == 10
    assert all(record["owner"] == record["package"] for record in matrix)


def test_new_ordinary_cargo_test_target_is_selected_by_test_gate(inputs: tuple) -> None:
    changed = list(copy.deepcopy(inputs))
    package = next(p for p in changed[0]["packages"] if p["name"] == "taskmesh")
    package["targets"].append(
        {
            "kind": ["test"],
            "name": "new_integration_case",
            "src_path": str(tc.REPO / "crates/taskmesh/tests/new_integration_case.rs"),
            "test": True,
        }
    )
    records, problems = catalog(tuple(changed))
    assert problems == []
    assert any(
        record["package"] == "taskmesh"
        and record["target"] == "new_integration_case"
        and record["executing_gate"] == "test"
        for record in records
    )


def test_unknown_feature_required_target_fails_closed(inputs: tuple) -> None:
    changed = list(copy.deepcopy(inputs))
    package = next(p for p in changed[0]["packages"] if p["name"] == "taskmesh")
    package["targets"].append(
        {
            "kind": ["test"],
            "name": "unowned_feature_case",
            "src_path": str(tc.REPO / "crates/taskmesh/tests/unowned_feature_case.rs"),
            "test": True,
            "required-features": ["rayon"],
        }
    )
    _, problems = catalog(tuple(changed))
    assert any(
        "unowned_feature_case" in problem and "no registered executor" in problem
        for problem in problems
    )


def test_new_fuzz_bin_requires_producer_registration(inputs: tuple) -> None:
    changed = list(copy.deepcopy(inputs))
    package = next(p for p in changed[1]["packages"] if p["name"] == "taskmesh-fuzz")
    package["targets"].append(
        {
            "kind": ["bin"],
            "name": "new_fuzz_target",
            "src_path": str(tc.REPO / "fuzz/fuzz_targets/new_fuzz_target.rs"),
            "test": False,
        }
    )
    _, problems = catalog(tuple(changed))
    assert any(
        "new_fuzz_target" in problem and "producer targets" in problem for problem in problems
    )


def test_python_module_rename_changes_the_selected_target(inputs: tuple) -> None:
    changed = list(copy.deepcopy(inputs))
    modules = changed[6]
    old = "tools/gates/tests/test_inventory.py"
    new = "tools/gates/tests/test_inventory_renamed.py"
    modules[modules.index(old)] = new
    records, problems = catalog(tuple(changed))
    assert problems == []
    selected = {record["path"] for record in records if record["executing_gate"] == "py-test"}
    assert new in selected and old not in selected
    assert "tools/gates/mod.rs" not in selected
