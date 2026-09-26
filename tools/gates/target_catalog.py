"""Static target-to-gate census; discovery is not execution evidence."""

from __future__ import annotations

import ast
import hashlib
import json
import subprocess
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # Python 3.9/3.10 tooling floor
    import tomli as tomllib

REPO = Path(__file__).resolve().parents[2]
MODEL_FEATURES = {
    "loom_governance": ("loom",),
    "shuttle_governance": ("shuttle",),
    "model_replay_fixture": ("shuttle",),
}
ZERO_CASE_LIBS = {
    "taskmesh-contract/lib/taskmesh_contract",
    "taskmesh-rayon/lib/taskmesh_rayon",
}
RAYON_SCOPES = (
    ("taskmesh", "lib", "taskmesh", None),
    (
        "taskmesh", "test", "hardening_executor_authority",
        "rayon_cpu_domain_is_separate_and_observable",
    ),
    ("taskmesh", "test", "e2e_scenarios", "rayon_cpu_soak_results_correct_and_drains"),
    (
        "taskmesh", "test", "hardening_dispatch_resolution",
        "direct_and_host_multistage_admission_have_distinct_reservation_contracts",
    ),
    (
        "taskmesh", "test", "runtime_cpu_executor",
        "the_cpu_gate_and_the_rayon_pool_are_sized_from_one_answer",
    ),
    ("taskmesh-rayon", "test", "rayon_smoke", "the_adapter_declares_what_it_can_honestly_promise"),
    ("taskmesh-contract", "test", "contract_roundtrip", "task_spec_roundtrips"),
    ("taskmesh-contract", "test", "contract_roundtrip", "runtime_config_roundtrips_pretty"),
    ("taskmesh-contract", "test", "contract_roundtrip", "snapshot_roundtrips"),
    (
        "taskmesh-contract", "test", "contract_roundtrip",
        "legacy_plan_source_strings_decode_and_reencode_exactly",
    ),
)
RECIPE_FRAGMENTS = {
    "test": (
        "uv run python tools/gates/execute_rust_tests.py",
    ),
    "test-rayon": ("python3 tools/gates/rust_test_evidence.py --rayon",),
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


def filesystem_target_problems(packages: list[dict], root: Path) -> list[str]:
    """Catch Cargo targets hidden by auto-discovery or bench=false settings."""
    problems: list[str] = []
    for package in packages:
        manifest = Path(package["manifest_path"])
        try:
            manifest.relative_to(root)
            document = tomllib.loads(manifest.read_text(encoding="utf-8"))
        except (OSError, ValueError, tomllib.TOMLDecodeError) as exc:
            problems.append(f"{package['name']}: cannot inspect target manifest: {exc}")
            continue
        package_root = manifest.parent
        for directory, kind in (
            ("tests", "test"), ("benches", "bench"),
            ("examples", "example"), ("src/bin", "bin"),
        ):
            discovered = {
                path.absolute()
                for path in (package_root / directory).glob("*.rs")
                if path.is_file()
            }
            discovered.update(
                path.absolute()
                for path in (package_root / directory).glob("*/main.rs")
                if path.is_file()
            )
            metadata_paths = {
                Path(target["src_path"]).absolute()
                for target in package.get("targets", [])
                if target.get("kind") == [kind]
            }
            for missing in sorted(discovered - metadata_paths):
                problems.append(
                    f"{package['name']}: {kind} source {missing.relative_to(root)} "
                    "is absent from Cargo metadata"
                )
        for bench in document.get("bench", []):
            if isinstance(bench, dict) and bench.get("bench") is False:
                problems.append(
                    f"{package['name']}: bench {bench.get('name', '<unnamed>')} "
                    "disables cargo bench execution"
                )
    return problems


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
    problems.extend(filesystem_target_problems(packages, root))
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
                        "owner": package_name,
                        "target": name,
                        "path": relative_path,
                        "required_features": list(features),
                        "executing_gate": gate,
                    }
                )
    package_by_name = {package["name"]: package for package in packages}
    for package_name, kind, target_name, case in RAYON_SCOPES:
        package = package_by_name.get(package_name)
        targets = package.get("targets", []) if isinstance(package, dict) else []
        matches = [
            target for target in targets
            if target.get("kind") == [kind] and target.get("name") == target_name
        ]
        if len(matches) != 1 or matches[0].get("test") is not True:
            problems.append(f"test-rayon: missing or disabled {package_name}/{kind}/{target_name}")
            continue
        if package_name == "taskmesh" and "rayon" not in package.get("features", {}):
            problems.append("test-rayon: taskmesh no longer declares the rayon feature")
            continue
        try:
            path = Path(matches[0]["src_path"]).relative_to(root).as_posix()
        except (KeyError, ValueError):
            problems.append(f"test-rayon: target path is outside the repository: {target_name}")
            continue
        records.append({
            "kind": "matrix-case" if case else "matrix-target",
            "package": package_name,
            "owner": package_name,
            "target": f"{target_name}::{case}" if case else target_name,
            "path": path,
            "required_features": ["rayon"] if package_name == "taskmesh" else [],
            "executing_gate": "test-rayon",
        })
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
                    "owner": package["name"],
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
                "owner": "tools",
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
