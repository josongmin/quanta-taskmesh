"""Dependency features belong to the crate that uses them, not workspace defaults."""

from __future__ import annotations

from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - exercised by the Python 3.9 lane
    import tomli as tomllib

REPO = Path(__file__).resolve().parents[3]


def manifest(path: str) -> dict:
    return tomllib.loads((REPO / path).read_text(encoding="utf-8"))


def features(dependency: object) -> set[str]:
    assert isinstance(dependency, dict)
    value = dependency.get("features", [])
    assert isinstance(value, list)
    return set(value)


def test_tokio_multithread_scheduler_is_not_a_shipped_workspace_default() -> None:
    root_tokio = manifest("Cargo.toml")["workspace"]["dependencies"]["tokio"]
    assert root_tokio == "1", "workspace defaults must not silently widen every consumer graph"

    taskmesh = manifest("crates/taskmesh/Cargo.toml")
    normal = features(taskmesh["dependencies"]["tokio"])
    assert normal == {"rt", "sync", "time", "macros"}
    assert "rt-multi-thread" not in normal
    assert features(taskmesh["dev-dependencies"]["tokio"]) == {"rt-multi-thread"}


def test_tooling_declares_only_the_tokio_features_its_code_uses() -> None:
    bench = features(manifest("crates/taskmesh-bench/Cargo.toml")["dev-dependencies"]["tokio"])
    assert bench == {"rt-multi-thread", "sync"}

    docs = features(manifest("tools/doc-examples/Cargo.toml")["dependencies"]["tokio"])
    assert docs == {"rt", "macros"}

    rayon = manifest("crates/taskmesh-rayon/Cargo.toml")
    assert "tokio" not in rayon.get("dev-dependencies", {})
