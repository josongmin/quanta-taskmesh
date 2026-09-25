"""Static target-to-gate census; discovery is not execution evidence."""

from __future__ import annotations

import ast
import hashlib
import json
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
MODEL_FEATURES = {
    "loom_governance": ("loom",),
    "shuttle_governance": ("shuttle",),
    "model_replay_fixture": ("shuttle",),
}
RECIPE_FRAGMENTS = {
    "test": (
        "uv run python tools/gates/execute_rust_tests.py",
    ),
    "test-rayon": (
        "cargo test --locked -p taskmesh --features rayon --lib",
        "cargo test --locked -p taskmesh --features rayon --test hardening_executor_authority",
        "rayon_cpu_domain_is_separate_and_observable -- --exact",
    ),
    "doctest": (
        "cargo test --locked --workspace --exclude taskmesh-doc-examples --doc",
        "cargo test --locked -p taskmesh --features rayon --doc",
    ),
    "py-test": ("uv run python tools/gates/execute_py_tests.py",),
    "bench-smoke": ("cargo bench --locked -p taskmesh-bench -- --test",),
    "bench-iai": ("bash tools/bench-iai.sh",),
    "modelcheck": ("tools/modelcheck/run.py all",),
    "fuzz-check": ("bash tools/fuzz/check.sh",),
    "fuzz": ("bash tools/fuzz/run.sh",),
}


def metadata(root: Path, *, fuzz: bool = False) -> dict:
    cmd = ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"]
    if fuzz:
        cmd.extend(["--manifest-path", str(root / "fuzz/Cargo.toml")])
    result = subprocess.run(cmd, cwd=root, capture_output=True, text=True, check=True)
    value = json.loads(result.stdout)
    if not isinstance(value, dict):
        raise ValueError("cargo metadata root must be an object")
    return value


def workspace_packages(value: dict) -> list[dict]:
    members = value.get("workspace_members")
    packages = value.get("packages")
    if not isinstance(members, list) or not isinstance(packages, list):
        raise ValueError("cargo metadata lacks workspace packages")
    by_id = {package["id"]: package for package in packages}
    if len(by_id) != len(packages) or any(member not in by_id for member in members):
        raise ValueError("cargo metadata workspace member identity is invalid")
    return [by_id[member] for member in members]


def model_test_targets(source: str) -> set[str]:
    """Read literal --test selectors from the registered model producer."""
    names: set[str] = set()
    for node in ast.walk(ast.parse(source)):
        if not isinstance(node, (ast.List, ast.Tuple)):
            continue
        for flag, value in zip(node.elts, node.elts[1:]):
            if (
                isinstance(flag, ast.Constant)
                and flag.value == "--test"
                and isinstance(value, ast.Constant)
                and isinstance(value.value, str)
            ):
                names.add(value.value)
    return names


def catalog(
    root_metadata: dict,
    fuzz_metadata: dict,
    fuzz_manifest: dict,
    perf_config: dict,
    recipes: dict[str, str],
    model_source: str,
    python_modules: list[str],
    *,
    root: Path = REPO,
) -> tuple[list[dict], list[str]]:
    """Map discovered targets and reject any surface without an executor."""
    records: list[dict] = []
    problems: list[str] = []
    for recipe, fragments in RECIPE_FRAGMENTS.items():
        # A comment cannot prove a recipe selects a target.
        body = "\n".join(
            line for line in recipes.get(recipe, "").splitlines()
            if not line.lstrip().startswith("#")
        )
        for fragment in fragments:
            if fragment not in body:
                problems.append(f"{recipe}: required selector is missing: {fragment}")

    model_declared: set[str] = set()
    iai_target = perf_config.get("instruction_count_gate", {}).get("bench")
    iai_declared = False
    try:
        packages = workspace_packages(root_metadata)
    except (KeyError, TypeError, ValueError) as exc:
        return [], [*problems, str(exc)]
    for package in packages:
        package_name = package["name"]
        for target in package.get("targets", []):
            kind = target.get("kind")
            name = target.get("name")
            features = tuple(target.get("required-features", ()))
            path = Path(target.get("src_path", ""))
            try:
                relative_path = path.relative_to(root).as_posix()
            except ValueError:
                problems.append(f"{package_name}/{name}: target path is outside the repository")
                continue
            gate: str | None = None
            if kind == ["test"]:
                if target.get("test") is not True:
                    problems.append(f"{package_name}/{name}: integration target disables testing")
                if features:
                    if package_name != "taskmesh-engine" or MODEL_FEATURES.get(name) != features:
                        problems.append(
                            f"{package_name}/{name}: feature-required test has no "
                            "registered executor"
                        )
                    else:
                        gate = "modelcheck"
                        model_declared.add(name)
                else:
                    gate = "test"
            elif kind == ["bench"]:
                if package_name != "taskmesh-bench":
                    problems.append(f"{package_name}/{name}: benchmark has no smoke owner")
                elif name == iai_target and features == ("iai",):
                    gate = "bench-iai"
                    iai_declared = True
                elif not features:
                    gate = "bench-smoke"
                else:
                    problems.append(f"{package_name}/{name}: bench features have no executor")
            elif target.get("test") is True:
                if kind == ["lib"] and not features:
                    gate = "test"
                else:
                    problems.append(
                        f"{package_name}/{name}: testable {kind} target is not selected"
                    )
            if gate:
                records.append(
                    {
                        "kind": kind[0],
                        "package": package_name,
                        "target": name,
                        "path": relative_path,
                        "required_features": list(features),
                        "executing_gate": gate,
                    }
                )
    if model_declared != set(MODEL_FEATURES):
        problems.append(
            f"model target set differs from registered exceptions: {sorted(model_declared)}"
        )
    selected_models = model_test_targets(model_source)
    if selected_models != model_declared:
        problems.append(
            f"modelcheck selectors {sorted(selected_models)} differ from targets "
            f"{sorted(model_declared)}"
        )
    if not iai_declared:
        problems.append(f"configured IAI target {iai_target!r} is missing or not feature-gated")

    try:
        fuzz_packages = workspace_packages(fuzz_metadata)
    except (KeyError, TypeError, ValueError) as exc:
        return records, [*problems, f"fuzz: {exc}"]
    fuzz_bins: set[str] = set()
    for package in fuzz_packages:
        if package.get("name") != "taskmesh-fuzz":
            problems.append(f"fuzz: unknown workspace package {package.get('name')!r}")
        for target in package.get("targets", []):
            if target.get("kind") != ["bin"] or target.get("test") is not False:
                problems.append(f"fuzz: unexpected target shape {target.get('name')!r}")
                continue
            name = target["name"]
            fuzz_bins.add(name)
            try:
                relative_path = Path(target["src_path"]).relative_to(root).as_posix()
            except ValueError:
                problems.append(f"fuzz/{name}: target path is outside the repository")
                continue
            records.append(
                {
                    "kind": "fuzz-bin",
                    "package": package["name"],
                    "target": name,
                    "path": relative_path,
                    "required_features": [],
                    "executing_gate": ["fuzz-check", "fuzz"],
                }
            )
    expected_fuzz = fuzz_manifest.get("required_targets")
    if not isinstance(expected_fuzz, dict) or set(expected_fuzz) != fuzz_bins or not fuzz_bins:
        problems.append(
            f"fuzz bins {sorted(fuzz_bins)} differ from producer targets "
            f"{sorted(expected_fuzz) if isinstance(expected_fuzz, dict) else expected_fuzz!r}"
        )

    if not python_modules:
        problems.append("py-test: no Python test modules discovered")
    for path in python_modules:
        records.append(
            {
                "kind": "pytest-module",
                "package": "tools",
                "target": Path(path).stem,
                "path": path,
                "required_features": [],
                "executing_gate": "py-test",
            }
        )
    records.sort(key=lambda item: (item["kind"], item["package"], item["target"], item["path"]))
    return records, problems


def python_test_modules(root: Path) -> list[str]:
    """Discover test entrypoints; support modules are not execution targets."""
    return sorted(
        path.relative_to(root).as_posix()
        for path in (root / "tools").rglob("*.py")
        if path.name.startswith("test_") or path.name.endswith("_test.py")
    )


def source_catalog(root: Path, recipes: dict[str, str]) -> tuple[list[dict], list[str]]:
    python_modules = python_test_modules(root)
    return catalog(
        metadata(root),
        metadata(root, fuzz=True),
        json.loads((root / "tools/fuzz/producer-manifest.json").read_text(encoding="utf-8")),
        json.loads((root / "tools/bench/perf-gate.json").read_text(encoding="utf-8")),
        recipes,
        (root / "tools/modelcheck/run.py").read_text(encoding="utf-8"),
        python_modules,
        root=root,
    )


def catalog_digest(records: list[dict]) -> str:
    payload = json.dumps(records, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(payload).hexdigest()
