"""Negative fixtures for the crate-boundary checker (TM16-016).

Every fixture below returned **zero violations** from the previous checker. They
are the regression: a boundary checker that cannot fail is indistinguishable
from no boundary checker, and `just test-architecture` was green the whole time.

The checks are driven through the real `check_*` functions with synthetic
`cargo metadata`, so the fixtures exercise the same code path the CLI does.
"""

from __future__ import annotations

import importlib.util
import subprocess
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]
CHECKER = REPO / "tools" / "arch" / "check_crate_boundaries.py"

_spec = importlib.util.spec_from_file_location("check_crate_boundaries", CHECKER)
assert _spec and _spec.loader
arch = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(arch)


def member_id(name: str) -> str:
    return f"path+file:///workspace/crates/{name}#0.1.0"


def package(name: str, deps: list[dict]) -> dict:
    return {"name": name, "id": member_id(name), "dependencies": deps}


def dep(name: str, *, kind: str | None = None, optional: bool = False) -> dict:
    return {"name": name, "kind": kind, "optional": optional}


def metadata(packages: list[dict]) -> dict:
    return {
        "packages": packages,
        "workspace_members": [pkg["id"] for pkg in packages],
    }


def known_good() -> dict:
    return metadata(
        [
            package("taskmesh-contract", [dep("serde")]),
            package("taskmesh-engine", [dep("taskmesh-contract"), dep("parking_lot")]),
            package(
                "taskmesh",
                [
                    dep("taskmesh-contract"),
                    dep("taskmesh-engine"),
                    dep("tokio"),
                    dep("tokio-util"),
                    dep("taskmesh-rayon", optional=True),
                ],
            ),
            package("taskmesh-rayon", [dep("taskmesh-contract"), dep("rayon")]),
            package(
                "taskmesh-bench",
                [
                    dep("taskmesh"),
                    dep("taskmesh-contract"),
                    dep("taskmesh-engine"),
                    dep("hdrhistogram"),
                    dep("rand"),
                    dep("rand_distr"),
                    dep("iai-callgrind", optional=True),
                ],
            ),
            package("taskmesh-doc-examples", [dep("taskmesh"), dep("tokio")]),
        ]
    )


def test_the_known_good_graph_passes() -> None:
    """Positive control: the negatives below fail for their own reason."""
    assert arch.check_edges(known_good()) == []
    assert arch.check_contract_deps(known_good()) == []


# ---- the four fixtures that used to return [] -----------------------------


def test_an_unregistered_workspace_crate_is_reported() -> None:
    graph = known_good()
    graph["packages"].append(package("taskmesh-surprise", [dep("tokio")]))
    graph["workspace_members"].append(member_id("taskmesh-surprise"))
    violations = arch.check_edges(graph)
    assert any(
        "taskmesh-surprise" in v and "no declared boundary policy" in v for v in violations
    ), violations


def test_engine_depending_on_tokio_is_reported() -> None:
    graph = known_good()
    for pkg in graph["packages"]:
        if pkg["name"] == "taskmesh-engine":
            pkg["dependencies"].append(dep("tokio"))
    violations = arch.check_edges(graph)
    assert any("'taskmesh-engine' -> 'tokio'" in v for v in violations), violations


def test_engine_depending_on_the_bench_harness_is_reported() -> None:
    graph = known_good()
    for pkg in graph["packages"]:
        if pkg["name"] == "taskmesh-engine":
            pkg["dependencies"].append(dep("taskmesh-bench"))
    violations = arch.check_edges(graph)
    assert any("'taskmesh-engine' -> 'taskmesh-bench'" in v for v in violations), violations


def test_a_non_optional_host_to_rayon_edge_is_reported() -> None:
    """The edge is allowed, but only behind a feature.

    Optionality was never recorded, so making the adapter unconditional looked
    identical to the intended configuration.
    """
    graph = known_good()
    for pkg in graph["packages"]:
        if pkg["name"] == "taskmesh":
            for d in pkg["dependencies"]:
                if d["name"] == "taskmesh-rayon":
                    d["optional"] = False
    violations = arch.check_edges(graph)
    assert any("must stay feature-optional" in v for v in violations), violations


# ---- surfaces the checker used to treat as "nothing to report" ------------


def test_a_contract_dependency_outside_the_allowlist_is_reported() -> None:
    graph = known_good()
    for pkg in graph["packages"]:
        if pkg["name"] == "taskmesh-contract":
            pkg["dependencies"].append(dep("parking_lot"))
    violations = arch.check_contract_deps(graph)
    assert any("parking_lot" in v for v in violations), violations


def test_a_policy_entry_for_a_removed_crate_is_reported() -> None:
    graph = known_good()
    graph["packages"] = [p for p in graph["packages"] if p["name"] != "taskmesh-rayon"]
    graph["workspace_members"] = [p["id"] for p in graph["packages"]]
    violations = arch.check_edges(graph)
    assert any("not a workspace member" in v and "taskmesh-rayon" in v for v in violations), (
        violations
    )


def test_metadata_with_no_matchable_members_is_refused() -> None:
    with pytest.raises(RuntimeError, match="could not match any workspace member"):
        arch.workspace_member_names({"packages": [], "workspace_members": []})


def test_a_missing_required_source_directory_is_reported(tmp_path: Path) -> None:
    (tmp_path / "crates" / "taskmesh-contract" / "src").mkdir(parents=True)
    violations = arch.check_source_usages(tmp_path)
    assert any("required source directory is missing" in v for v in violations), violations
    assert any("taskmesh-engine/src" in v for v in violations), violations


def test_an_unreadable_source_file_is_reported(tmp_path: Path) -> None:
    """A file the checker could not open is a file it did not check."""
    for relative in arch.REQUIRED_SOURCE_DIRS:
        (tmp_path / "crates" / relative).mkdir(parents=True)
    unreadable = tmp_path / "crates" / "taskmesh-engine" / "src" / "locked.rs"
    unreadable.write_text("pub fn f() {}\n", encoding="utf-8")
    unreadable.chmod(0o000)
    try:
        violations = arch.check_source_usages(tmp_path)
    finally:
        unreadable.chmod(0o644)
    if not violations:
        pytest.skip("filesystem grants read access regardless of mode (running as root?)")
    assert any("could not read" in v for v in violations), violations


def _tree(tmp_path: Path) -> Path:
    for relative in arch.REQUIRED_SOURCE_DIRS:
        (tmp_path / "crates" / relative).mkdir(parents=True, exist_ok=True)
    return tmp_path / "crates"


def test_a_forbidden_import_in_engine_source_is_reported(tmp_path: Path) -> None:
    crates = _tree(tmp_path)
    (crates / "taskmesh-engine" / "src" / "bad.rs").write_text(
        "use tokio::task;\npub fn f() {}\n", encoding="utf-8"
    )
    violations = arch.check_source_usages(tmp_path)
    assert any("forbidden usage of crate 'tokio'" in v for v in violations), violations


def test_every_spelling_of_a_forbidden_crate_is_reported(tmp_path: Path) -> None:
    """The scan used to look for the substring `use tokio`, so a fully
    qualified `tokio::runtime::Handle::current()` with no `use` — and the
    `::rayon::join` spelling — passed. Every spelling counts."""
    crates = _tree(tmp_path)
    cases = {
        "spawn.rs": "pub fn f() { tokio::runtime::Handle::current(); }\n",
        "join.rs": "pub fn g() { ::rayon::join(|| 1, || 2); }\n",
        "ext.rs": "extern crate tokio;\npub fn h() {}\n",
        "host.rs": "pub fn k() -> taskmesh::Builder { taskmesh::Builder::new() }\n",
    }
    for name, text in cases.items():
        (crates / "taskmesh-engine" / "src" / name).write_text(text, encoding="utf-8")
    violations = arch.check_source_usages(tmp_path)
    for name, crate in [
        ("spawn.rs", "tokio"),
        ("join.rs", "rayon"),
        ("ext.rs", "tokio"),
        ("host.rs", "taskmesh"),
    ]:
        assert any(f"crate '{crate}'" in v and name in v for v in violations), (name, violations)


def test_a_comment_mentioning_a_crate_is_not_a_usage(tmp_path: Path) -> None:
    crates = _tree(tmp_path)
    (crates / "taskmesh-engine" / "src" / "doc.rs").write_text(
        "// The host wires tokio::spawn; the engine never does.\n"
        "/// See `tokio::task` in the host crate.\n"
        "pub fn f() {}\n",
        encoding="utf-8",
    )
    assert arch.check_source_usages(tmp_path) == []


def test_test_modules_inside_src_are_scanned_and_must_be_cfg_test(tmp_path: Path) -> None:
    """`*_tests.rs` and `src/tests/` are compiled into the crate. A forbidden
    crate there is a violation, and the module must be `#[cfg(test)]` so the
    compiler — not a filename convention — keeps it out of the shipped binary."""
    crates = _tree(tmp_path)
    engine = crates / "taskmesh-engine" / "src"
    (engine / "thing_tests.rs").write_text(
        "use tokio::task;\n#[test]\nfn t() {}\n", encoding="utf-8"
    )
    (engine / "tests").mkdir()
    (engine / "tests" / "mod.rs").write_text("use rayon::prelude::*;\n", encoding="utf-8")
    (engine / "lib.rs").write_text(
        "mod thing_tests;\n#[cfg(test)]\nmod tests;\npub fn f() {}\n", encoding="utf-8"
    )
    violations = arch.check_source_usages(tmp_path)
    assert any("crate 'tokio'" in v and "thing_tests.rs" in v for v in violations), violations
    assert any("crate 'rayon'" in v and "tests/mod.rs" in v for v in violations), violations
    assert any("`mod thing_tests;`" in v and "not under #[cfg(test)]" in v for v in violations), (
        violations
    )
    assert not any("`mod tests;`" in v for v in violations), "the gated one is fine"


def test_a_gated_test_module_with_stacked_attributes_is_accepted(tmp_path: Path) -> None:
    crates = _tree(tmp_path)
    (crates / "taskmesh" / "src" / "lib.rs").write_text(
        '#[cfg(test)]\n#[allow(clippy::unwrap_used, reason = "tests")]\nmod claim_tests;\n',
        encoding="utf-8",
    )
    assert arch.check_source_usages(tmp_path) == []


# ---- the real CLI ----------------------------------------------------------


def test_the_real_cli_passes_on_this_repository() -> None:
    proc = subprocess.run(
        [sys.executable, str(CHECKER)],
        capture_output=True,
        text=True,
        check=False,
        cwd=REPO,
    )
    assert proc.returncode == 0, proc.stdout + proc.stderr
    assert "no violations" in proc.stdout


def test_the_real_cli_reports_a_broken_workspace(tmp_path: Path) -> None:
    """`cargo metadata` failing must be an error, not an empty violation list."""
    (tmp_path / "Cargo.toml").write_text("this is not valid toml [[[\n", encoding="utf-8")
    proc = subprocess.run(
        [sys.executable, str(CHECKER), "--manifest-path", str(tmp_path)],
        capture_output=True,
        text=True,
        check=False,
    )
    assert proc.returncode == 1
    assert "error:" in proc.stderr
