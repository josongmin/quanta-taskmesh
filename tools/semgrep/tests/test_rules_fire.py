"""Regression tests for the semgrep rule set (trigger must fire, clean must not)."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]
RULES_DIR = REPO / "tools" / "semgrep" / "rules"

# One fire/clean pair per rule, at the path the rule is written for. A rule
# without a pair here is not considered enforced: the semgrep syntax could be
# silently wrong (`$...` once made a rule match nothing) and nobody would know.
#
# (rule id, rule file, fixture path relative to the repo root, trigger, clean)
T = "#[test]\nfn t() {\n"
A = "#[tokio::test]\nasync fn t() {\n"
AF = '#[tokio::test(flavor = "multi_thread", worker_threads = 2)]\nasync fn t() {\n'
ENGINE = "crates/taskmesh-engine/src/probe.rs"
ENGINE_TEST_MOD = "crates/taskmesh-engine/src/scheduler_tests.rs"
ENGINE_TESTS_DIR = "crates/taskmesh-engine/src/tests/mod.rs"
CONTRACT = "crates/taskmesh-contract/src/probe.rs"
RAYON = "crates/taskmesh-rayon/src/probe.rs"
HOST = "crates/taskmesh/src/probe.rs"
DOMAIN = "crates/taskmesh-engine/src/features/admission/domain/probe.rs"
ITEST = "crates/taskmesh-engine/tests/probe.rs"
HOST_ITEST = "crates/taskmesh/tests/probe.rs"

CASES: list[tuple[str, str, str, str, str]] = [
    # ---- architecture --------------------------------------------------------
    (
        "taskmesh-no-tokio-in-contract-engine-rayon",
        "architecture.yml",
        ENGINE,
        "use tokio::task;\n\npub fn f() {}\n",
        "pub fn f() -> u32 { 1 }\n",
    ),
    (
        # The audit's hole: a `use tokio` in an in-`src` test module was
        # excluded from the scan. It is compiled into the crate; it counts.
        "taskmesh-no-tokio-in-contract-engine-rayon",
        "architecture.yml",
        ENGINE_TEST_MOD,
        "use tokio::runtime::Handle;\n\n#[test]\nfn t() { let _h = Handle::current(); }\n",
        "#[test]\nfn t() { assert_eq!(1, 1 + 0); }\n",
    ),
    (
        "taskmesh-no-tokio-in-contract-engine-rayon",
        "architecture.yml",
        ENGINE,
        "pub fn f() { tokio::runtime::Handle::current(); }\n",
        "pub fn f() {}\n",
    ),
    (
        "taskmesh-no-rayon-outside-rayon-adapter",
        "architecture.yml",
        ENGINE_TESTS_DIR,
        "use rayon::prelude::*;\n\npub fn f() {}\n",
        "pub fn f() {}\n",
    ),
    (
        "taskmesh-rayon-no-host-or-engine-import",
        "architecture.yml",
        RAYON,
        "use taskmesh_engine::Governor;\n\npub fn f() {}\n",
        "use taskmesh_contract::CpuExecutor;\n\npub fn f() {}\n",
    ),
    (
        "taskmesh-engine-no-host-import",
        "architecture.yml",
        ENGINE,
        "use taskmesh::Builder;\n\npub fn f() {}\n",
        "use taskmesh_contract::TaskClass;\n\npub fn f() {}\n",
    ),
    (
        "taskmesh-no-adapters-in-engine-domain",
        "architecture.yml",
        DOMAIN,
        "use crate::adapters::tokio_waker;\n\npub fn f() {}\n",
        "use crate::shared::PermitId;\n\npub fn f() {}\n",
    ),
    (
        "taskmesh-no-std-fs-in-contract-engine",
        "architecture.yml",
        CONTRACT,
        'pub fn f() -> bool { std::fs::read("x").is_ok() }\n',
        "pub fn f() -> bool { true }\n",
    ),
    (
        "taskmesh-no-terminal-io-in-library-crates",
        "architecture.yml",
        HOST,
        'pub fn f() { println!("hello"); }\n',
        'pub fn f() -> &\'static str { "hello" }\n',
    ),
    # ---- dependency policy ---------------------------------------------------
    (
        "taskmesh-no-async-std",
        "dependency-policy.yml",
        HOST,
        "use async_std::task;\n\npub fn f() {}\n",
        "use tokio::task;\n\npub fn f() {}\n",
    ),
    (
        "taskmesh-rayon-no-tokio",
        "dependency-policy.yml",
        RAYON,
        "use tokio_util::sync::CancellationToken;\n\npub fn f() {}\n",
        "use rayon::ThreadPool;\n\npub fn f() {}\n",
    ),
    (
        "taskmesh-contract-no-parking-lot",
        "dependency-policy.yml",
        CONTRACT,
        "use parking_lot::Mutex;\n\npub fn f() {}\n",
        "use std::sync::Mutex;\n\npub fn f() {}\n",
    ),
    # ---- determinism ---------------------------------------------------------
    (
        "taskmesh-no-clock-now-outside-host-adapters",
        "determinism.yml",
        ENGINE,
        "pub fn f() -> std::time::Instant { std::time::Instant::now() }\n",
        "pub fn f(now_ms: u64) -> u64 { now_ms }\n",
    ),
    (
        "taskmesh-no-randomness-in-contract-and-engine",
        "determinism.yml",
        ENGINE,
        "pub fn pick() -> u64 {\n    rand::random::<u64>()\n}\n",
        "pub fn pick(seed: u64) -> u64 {\n    seed\n}\n",
    ),
    (
        "taskmesh-no-env-read-in-contract-engine",
        "determinism.yml",
        ENGINE,
        'pub fn f() -> bool { std::env::var("X").is_ok() }\n',
        "pub fn f(x: Option<&str>) -> bool { x.is_some() }\n",
    ),
    # ---- error handling ------------------------------------------------------
    (
        "taskmesh-no-unannotated-allow",
        "error-handling.yml",
        CONTRACT,
        "#![allow(\n    clippy::indexing_slicing,\n    clippy::as_conversions\n)]\n\npub fn f() {}\n",  # noqa: E501
        '#![allow(\n    clippy::indexing_slicing,\n    reason = "bounds checked above"\n)]\n\npub fn f() {}\n',  # noqa: E501
    ),
    (
        "taskmesh-no-unwrap-in-production",
        "error-handling.yml",
        ENGINE,
        "pub fn f(x: Option<u8>) -> u8 { x.unwrap() }\n",
        "pub fn f(x: Option<u8>) -> u8 { x.unwrap_or(0) }\n",
    ),
    (
        "taskmesh-no-todo-unimplemented-in-production",
        "error-handling.yml",
        ENGINE,
        "pub fn f() -> u8 { todo!() }\n",
        "pub fn f() -> u8 { 0 }\n",
    ),
    (
        "taskmesh-no-panic-in-production",
        "error-handling.yml",
        ENGINE,
        'pub fn f(x: u8) -> u8 { if x > 1 { panic!("no") } else { x } }\n',
        "pub fn f(x: u8) -> Result<u8, u8> { if x > 1 { Err(x) } else { Ok(x) } }\n",
    ),
    (
        "taskmesh-expect-needs-context",
        "error-handling.yml",
        ENGINE,
        'pub fn f(x: Option<u8>) -> u8 { x.expect("todo") }\n',
        'pub fn f(x: Option<u8>) -> u8 { x.expect("permit presence checked above") }\n',
    ),
    (
        "taskmesh-no-error-swallowing",
        "error-handling.yml",
        ENGINE,
        "pub fn f(r: Result<u8, u8>) { match r { Ok(_) => {} Err(_) => {} } }\n",
        "pub fn f(r: Result<u8, u8>) -> Option<u8> { match r { Ok(v) => Some(v), Err(_) => None } }\n",  # noqa: E501
    ),
    (
        "taskmesh-no-discarded-result",
        "error-handling.yml",
        ENGINE,
        "pub fn f(r: Result<u8, u8>) { let _ = r; }\n",
        "pub fn f(r: Result<u8, u8>) -> Result<u8, u8> { r }\n",
    ),
    # ---- indexing ------------------------------------------------------------
    (
        "taskmesh-no-unchecked-indexing-in-engine",
        "indexing-policy.yml",
        ENGINE,
        "pub fn f(a: &[u8]) -> u8 {\n    a[0]\n}\n",
        "pub fn f(a: &[u8]) -> Option<&u8> {\n    a.get(0)\n}\n",
    ),
    # ---- test quality (each body shape at least once across the pack) ---------
    (
        "taskmesh-test-trivial-assert-true",
        "test-quality.yml",
        ITEST,
        T + "    assert!(true);\n}\n",
        T + "    assert!(1 < 2);\n}\n",
    ),
    (
        "taskmesh-test-self-equality",
        "test-quality.yml",
        ITEST,
        T + "    let x = 1;\n    assert_eq!(x, x);\n}\n",
        # (`let x = 1; assert_eq!(x, 1)` would be folded to `1 == 1` by
        # semgrep's constant propagation and fire — correctly.)
        T + "    assert_eq!(1 + 1, 2);\n}\n",
    ),
    (
        "taskmesh-test-self-inequality",
        "test-quality.yml",
        ITEST,
        T + "    let x = 1;\n    assert_ne!(x, x);\n}\n",
        T + "    let x = 1;\n    assert_ne!(x, 2);\n}\n",
    ),
    (
        "taskmesh-test-tautological-some-or-none",
        "test-quality.yml",
        ITEST,
        T + "    let x = Some(1);\n    assert!(x.is_some() || x.is_none());\n}\n",
        T + "    let x = Some(1);\n    assert!(x.is_some());\n}\n",
    ),
    (
        "taskmesh-test-len-ge-zero",
        "test-quality.yml",
        ITEST,
        T + "    let v = vec![1u8];\n    assert!(v.len() >= 0usize);\n}\n",
        T + "    let v = vec![1u8];\n    assert_eq!(v.len(), 1);\n}\n",
    ),
    (
        "taskmesh-test-contains-empty-substring",
        "test-quality.yml",
        ITEST,
        T + '    let s = "abc";\n    assert!(s.contains(""));\n}\n',
        T + '    let s = "abc";\n    assert!(s.contains("b"));\n}\n',
    ),
    (
        "taskmesh-test-is-ok-without-value-check",
        "test-quality.yml",
        HOST_ITEST,
        A + "    let r: Result<u8, ()> = Ok(1);\n    assert!(r.is_ok());\n}\n",
        A + "    let r: Result<u8, ()> = Ok(1);\n    assert_eq!(r, Ok(1));\n}\n",
    ),
    (
        "taskmesh-test-discards-fallible-let",
        "test-quality.yml",
        HOST_ITEST,
        AF + "    let r: Result<u8, ()> = Ok(1);\n    let _ = r;\n}\n",
        AF + "    let r: Result<u8, ()> = Ok(1);\n    assert_eq!(r, Ok(1));\n}\n",
    ),
    (
        "taskmesh-test-permissive-exit-code-match",
        "test-quality.yml",
        ITEST,
        T + "    let c = 1;\n    assert!(matches!(c, 0 | 1 | 2));\n}\n",
        T + "    let c = 1;\n    assert_eq!(c, 1);\n}\n",
    ),
    (
        "taskmesh-test-self-fn-equality",
        "test-quality.yml",
        HOST_ITEST,
        "fn g(x: u8) -> u8 { x }\n" + A + "    assert_eq!(g(1), g(1));\n}\n",
        "fn g(x: u8) -> u8 { x }\n" + A + "    assert_eq!(g(1), 1);\n}\n",
    ),
    (
        "taskmesh-test-empty-body",
        "test-quality.yml",
        ITEST,
        "#[test]\nfn t() { }\n",
        T + "    assert_eq!(1 + 1, 2);\n}\n",
    ),
    (
        "taskmesh-ignore-needs-reason",
        "test-quality.yml",
        ITEST,
        "#[ignore]\n" + T + "    assert_eq!(1 + 1, 2);\n}\n",
        '#[ignore = "needs a 64-core host"]\n' + T + "    assert_eq!(1 + 1, 2);\n}\n",
    ),
    (
        "taskmesh-test-uninjected-system-time",
        "test-quality.yml",
        HOST_ITEST,
        A + "    let _now = std::time::SystemTime::now();\n    assert_eq!(1 + 1, 2);\n}\n",
        A + "    let now_ms = 1_000u64;\n    assert_eq!(now_ms, 1_000);\n}\n",
    ),
    (
        "taskmesh-test-thread-sleep",
        "test-quality.yml",
        HOST_ITEST,
        AF
        + "    std::thread::sleep(std::time::Duration::from_millis(5));\n    assert_eq!(1 + 1, 2);\n}\n",  # noqa: E501
        AF
        + '    let (tx, rx) = std::sync::mpsc::channel::<()>();\n    tx.send(()).expect("rx alive");\n    rx.recv().expect("tx alive");\n}\n',  # noqa: E501
    ),
    (
        "taskmesh-test-eprintln-debug-leftover",
        "test-quality.yml",
        HOST_ITEST,
        A + '    println!("debug {}", 1);\n    assert_eq!(1 + 1, 2);\n}\n',
        A + '    assert_eq!(1 + 1, 2, "arithmetic {}", 1);\n}\n',
    ),
    (
        "taskmesh-test-rand-without-seed",
        "test-quality.yml",
        ITEST,
        T + "    let _rng = rand::thread_rng();\n    assert_eq!(1 + 1, 2);\n}\n",
        T + "    let seed = 7u64;\n    assert_eq!(seed, 7);\n}\n",
    ),
    (
        "taskmesh-test-vague-stderr-contains-error",
        "test-quality.yml",
        HOST_ITEST,
        A
        + '    let stderr = String::from("error: x");\n    assert!(stderr.contains("error"));\n}\n',  # noqa: E501
        A
        + '    let stderr = String::from("error: x");\n    assert!(stderr.contains("invalid --from timestamp"));\n}\n',  # noqa: E501
    ),
]


def _semgrep_json(args: list[str]) -> dict:
    proc = subprocess.run(
        ["semgrep", *args],
        capture_output=True,
        text=True,
        check=False,
        cwd=REPO,
    )
    if proc.returncode not in (0, 1):
        raise RuntimeError(f"semgrep failed ({proc.returncode}): {proc.stderr}")
    return json.loads(proc.stdout or "{}")


def _run_semgrep(config: Path, target_dir: Path) -> set[str]:
    # Use the repository's own ignore policy, not semgrep's defaults. A fixture
    # placed in a `tests/` directory is otherwise skipped by the very default
    # this rule pack had to override, and the rule would look enforced while
    # never being reached.
    shutil.copyfile(REPO / ".semgrepignore", target_dir / ".semgrepignore")
    data = _semgrep_json(
        ["--config", str(config), "--json", "--quiet", "--no-git-ignore", str(target_dir)]
    )
    ids = set()
    for r in data.get("results", []):
        rid = r.get("check_id", "")
        ids.add(rid.split(".")[-1])
    return ids


# Real integration tests that MUST be inside the scanned target set. If the
# scanner stops reaching these, the test-quality rules are decorative again.
ENROLLED_PATHS = [
    "crates/taskmesh/tests/e2e_chaos.rs",
    "crates/taskmesh/tests/hardening_deadline_custody.rs",
    "crates/taskmesh-engine/tests/hardening_fairness_reference.rs",
    "crates/taskmesh-engine/tests/prop_invariants.rs",
    "crates/taskmesh-bench/tests/inferno.rs",
    "crates/taskmesh-rayon/tests/rayon_smoke.rs",
]


# Skip only for local convenience when semgrep is absent. Under CI the suite must
# NOT silently skip — a broken runner that drops semgrep would otherwise go green
# without verifying any rule. So in CI a missing binary is a hard failure.
if shutil.which("semgrep") is None and os.environ.get("CI"):
    raise RuntimeError("semgrep is required in CI but was not found on PATH")

pytestmark = pytest.mark.skipif(
    shutil.which("semgrep") is None, reason="semgrep not installed (local only)"
)


def _case_id(case: tuple[str, str, str, str, str]) -> str:
    rule_id, _, path, *_ = case
    return f"{rule_id}@{Path(path).name}"


def _fixture_path(index: int, rel_path: str) -> Path:
    # Several cases share a directory; each gets its own file so one scan
    # covers every fixture and findings can be attributed per case.
    rel = Path(rel_path)
    return rel.with_name(f"{rel.stem}_{index}{rel.suffix}")


@pytest.fixture(scope="module")
def scan(tmp_path_factory: pytest.TempPathFactory) -> dict[str, dict[Path, set[str]]]:
    """Every trigger fixture in one tree, every clean fixture in another, each
    scanned once with the whole rule pack: findings keyed by fixture path.
    (One semgrep run per case took the suite past seven minutes.)"""
    root = tmp_path_factory.mktemp("semgrep")
    trees = {"trig": root / "trig", "clean": root / "clean"}
    for index, (_, _, rel_path, trigger, clean) in enumerate(CASES):
        for kind, text in (("trig", trigger), ("clean", clean)):
            target = trees[kind] / _fixture_path(index, rel_path)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(text, encoding="utf-8")
    findings: dict[str, dict[Path, set[str]]] = {}
    for kind, tree in trees.items():
        shutil.copyfile(REPO / ".semgrepignore", tree / ".semgrepignore")
        data = _semgrep_json(
            ["--config", str(RULES_DIR), "--json", "--quiet", "--no-git-ignore", str(tree)]
        )
        per_file: dict[Path, set[str]] = {}
        for result in data.get("results", []):
            rel = Path(result["path"]).resolve().relative_to(tree.resolve())
            per_file.setdefault(rel, set()).add(result.get("check_id", "").split(".")[-1])
        findings[kind] = per_file
    return findings


@pytest.mark.parametrize("index,case", list(enumerate(CASES)), ids=[_case_id(c) for c in CASES])
def test_rule_fires_on_trigger_and_not_on_clean(
    index: int, case: tuple[str, str, str, str, str], scan: dict[str, dict[Path, set[str]]]
) -> None:
    rule_id, rule_file, rel_path, trigger, clean = case
    assert (RULES_DIR / rule_file).exists(), f"rule file missing: {rule_file}"
    path = _fixture_path(index, rel_path)
    fired = scan["trig"].get(path, set())
    assert rule_id in fired, (
        f"{rule_id} did NOT fire on trigger at {rel_path}.\nfixture:\n{trigger}\nfired: {sorted(fired)}"  # noqa: E501
    )
    fired_clean = scan["clean"].get(path, set())
    assert rule_id not in fired_clean, (
        f"{rule_id} FALSE-fired on clean at {rel_path}.\nfixture:\n{clean}\nfired: {sorted(fired_clean)}"  # noqa: E501
    )


def _every_rule_id() -> set[str]:
    ids: set[str] = set()
    for rule_file in sorted(RULES_DIR.glob("*.yml")):
        for line in rule_file.read_text(encoding="utf-8").splitlines():
            stripped = line.strip()
            if stripped.startswith("- id: "):
                ids.add(stripped[len("- id: ") :].strip())
    return ids


def test_every_rule_has_a_fire_and_clean_fixture() -> None:
    """A rule that nobody has watched fire is not enforcement; it is a hope.
    (`println!($...)` matched nothing for as long as it was unwatched.)"""
    covered = {case[0] for case in CASES}
    uncovered = sorted(_every_rule_id() - covered)
    assert not uncovered, f"rules without a fire/clean fixture: {uncovered}"
    phantom = sorted(covered - _every_rule_id())
    assert not phantom, f"fixtures for rules that no longer exist: {phantom}"


def test_real_integration_tests_are_inside_the_scanned_target_set() -> None:
    """The front door must actually reach the tests the rules are written for.

    Semgrep's built-in ignore list excludes `tests/` directories. With the
    defaults in place this scan covered 50 files and skipped 52 paths, so every
    real integration test was invisible to the test-quality rules while the rule
    unit tests stayed green. This asserts the *target set*, not the findings:
    a clean scan of files that were never opened is not a clean scan.
    """
    data = _semgrep_json(["--config", "tools/semgrep/rules", "--json", "--verbose", "crates"])
    paths = data.get("paths", {})
    scanned = {str(Path(p)) for p in paths.get("scanned", [])}
    assert scanned, "semgrep reported no scanned paths at all"
    missing = [p for p in ENROLLED_PATHS if p not in scanned]
    assert not missing, (
        f"these integration tests are not being scanned: {missing}\nscanned {len(scanned)} files"
    )


def test_the_scan_reaches_every_crate_test_directory() -> None:
    """No crate may fall out of the scanned set unnoticed."""
    data = _semgrep_json(["--config", "tools/semgrep/rules", "--json", "--verbose", "crates"])
    scanned = {str(Path(p)) for p in data.get("paths", {}).get("scanned", [])}
    for crate_tests in sorted((REPO / "crates").glob("*/tests")):
        if not crate_tests.is_dir():
            continue
        expected = {str(f.relative_to(REPO)) for f in crate_tests.rglob("*.rs")}
        if not expected:
            continue
        assert expected & scanned, f"no file under {crate_tests.relative_to(REPO)} was scanned"
