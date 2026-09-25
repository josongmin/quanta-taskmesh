"""Regression tests for the BG25 static scenario mapping validator."""

from __future__ import annotations

import importlib.util
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]
VALIDATOR = (
    REPO
    / "docs"
    / "plans"
    / "bugbash-sep-25-general"
    / "tickets"
    / "validate_scenario_evidence.py"
)
SPEC = importlib.util.spec_from_file_location("validate_scenario_evidence", VALIDATOR)
assert SPEC is not None and SPEC.loader is not None
VALIDATE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATE)


@pytest.mark.parametrize(
    "declaration",
    [
        "#[test]\nfn named_case() {}\n",
        "#[tokio::test]\nasync fn named_case() {}\n",
        '#[tokio::test(flavor = "multi_thread")]\nasync fn named_case() {}\n',
        '#[cfg(not(feature = "rayon"))]\n#[test]\nfn named_case() {}\n',
    ],
)
def test_test_case_exists_accepts_only_declared_tests(
    tmp_path: Path, declaration: str
) -> None:
    source = tmp_path / "case.rs"
    source.write_text(declaration, encoding="utf-8")
    assert VALIDATE.test_case_exists(source, "named_case")


@pytest.mark.parametrize(
    "declaration",
    [
        "fn named_case() {}\n",
        "async fn named_case() {}\n",
        "// #[test]\nfn named_case() {}\n",
        "#[tokio::test_case]\nasync fn named_case() {}\n",
        "#[test]\nfn different_case() {}\nfn named_case() {}\n",
        "#[test]\n#[ignore]\nfn named_case() {}\n",
        '#[ignore = "flaky"]\n#[test]\nfn named_case() {}\n',
        '#[ignore]\n/// docs\n#[test]\nfn named_case() {}\n',
        '#[cfg(feature = "rayon")]\n#[test]\nfn named_case() {}\n',
        '#[cfg(target_os = "linux")]\n#[test]\nfn named_case() {}\n',
        '#[cfg_attr(test, ignore)]\n#[test]\nfn named_case() {}\n',
        '#![cfg(shuttle)]\n#[test]\nfn named_case() {}\n',
    ],
)
def test_test_case_exists_rejects_helpers_and_misattributed_attributes(
    tmp_path: Path, declaration: str
) -> None:
    source = tmp_path / "case.rs"
    source.write_text(declaration, encoding="utf-8")
    assert not VALIDATE.test_case_exists(source, "named_case")


def test_feature_gated_case_requires_matching_feature(tmp_path: Path) -> None:
    source = tmp_path / "case.rs"
    source.write_text(
        '#[cfg(feature = "rayon")]\n#[tokio::test]\nasync fn named_case() {}\n',
        encoding="utf-8",
    )
    assert VALIDATE.test_case_exists(source, "named_case", feature="rayon")
    assert not VALIDATE.test_case_exists(source, "named_case", feature="default")
    source.write_text(
        '#![cfg(feature = "rayon")]\n#[test]\nfn named_case() {}\n',
        encoding="utf-8",
    )
    assert VALIDATE.test_case_exists(source, "named_case", feature="rayon")
    assert not VALIDATE.test_case_exists(source, "named_case", feature="default")
    source.write_text(
        '#[cfg(not(feature = "rayon"))]\n#[test]\nfn named_case() {}\n',
        encoding="utf-8",
    )
    assert VALIDATE.test_case_exists(source, "named_case", feature="default")
    assert not VALIDATE.test_case_exists(source, "named_case", feature="rayon")


def test_committed_scenario_mapping_uses_real_test_functions() -> None:
    VALIDATE.main()


def test_repository_file_rejects_absolute_traversal_and_external_symlink(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repository = tmp_path / "repo"
    repository.mkdir()
    owned = repository / "owned.rs"
    owned.write_text("#[test]\nfn owned() {}\n", encoding="utf-8")
    external = tmp_path / "external.rs"
    external.write_text("#[test]\nfn external() {}\n", encoding="utf-8")
    (repository / "linked.rs").symlink_to(external)
    monkeypatch.setattr(VALIDATE, "REPO", repository)

    assert VALIDATE.repository_file("owned.rs", "fixture") == owned
    for path in (str(external), "../external.rs", "linked.rs"):
        with pytest.raises(ValueError, match="repository-relative|outside"):
            VALIDATE.repository_file(path, "fixture")


def test_nightly_inventory_rejects_duplicate_gate_rows() -> None:
    rows = [
        {"gate": gate, "status": "NOT_RUN", "reason": "not run"}
        for gate in sorted(VALIDATE.NIGHTLY_GATES)
    ]
    VALIDATE.validate_nightly(rows)
    with pytest.raises(ValueError, match="missing or duplicate"):
        VALIDATE.validate_nightly([*rows, rows[0]])
    with pytest.raises(ValueError, match="unknown or missing fields"):
        VALIDATE.validate_nightly([{**rows[0], "receipt": "forged"}, *rows[1:]])
    for reason in ("", "   ", 1):
        with pytest.raises(ValueError, match="explicit NOT_RUN"):
            VALIDATE.validate_nightly([{**rows[0], "reason": reason}, *rows[1:]])


def test_recipe_selector_requires_an_executed_exact_cargo_test() -> None:
    selector = "--test sample exact_case -- --exact"
    assert VALIDATE.recipe_executes_test(
        f"CARGO_BUILD_JOBS=4 cargo test --locked -p taskmesh --features rayon {selector}\n",
        "sample",
        "exact_case",
        "taskmesh",
    )
    for body in (
        f"# cargo test {selector}\n",
        f"echo cargo test {selector}\n",
        "cargo test -p taskmesh --features rayon --test sample exact_case_extra -- --exact\n",
        "cargo test -p taskmesh --features rayon --test sample exact_case\n",
        f"cargo test -p taskmesh --features rayon {selector} || true\n",
        f"cargo test -p other --features rayon {selector}\n",
        f"cargo test -p taskmesh {selector}\n",
        f"cargo test -p taskmesh --features rayon -- {selector}\n",
        f"cargo test --features rayon {selector} -p taskmesh\n",
    ):
        assert not VALIDATE.recipe_executes_test(body, "sample", "exact_case", "taskmesh")


def test_evidence_cases_are_exact_nonduplicated_pairs() -> None:
    row = {
        "target": "tests/primary.rs",
        "case": "primary",
        "supporting_cases": [{"target": "tests/support.rs", "case": "support"}],
    }
    assert VALIDATE.evidence_cases(row, "H00") == [
        ("tests/primary.rs", "primary"),
        ("tests/support.rs", "support"),
    ]
    with pytest.raises(ValueError, match="duplicate"):
        VALIDATE.evidence_cases(
            {
                **row,
                "supporting_cases": [{"target": "tests/primary.rs", "case": "primary"}],
            },
            "H00",
        )
    with pytest.raises(ValueError, match="exactly target and case"):
        VALIDATE.evidence_cases(
            {
                **row,
                "supporting_cases": [
                    {"target": "tests/support.rs", "case": "support", "claim": "forged"}
                ],
            },
            "H00",
        )
