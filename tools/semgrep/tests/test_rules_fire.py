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

CASES: list[tuple[str, str, str, str]] = [
    (
        "taskmesh-no-unchecked-indexing-in-engine",
        "indexing-policy.yml",
        "pub fn f(a: &[u8]) -> u8 {\n    a[0]\n}\n",
        "pub fn f(a: &[u8]) -> Option<&u8> {\n    a.get(0)\n}\n",
    ),
    (
        "taskmesh-no-unannotated-allow",
        "error-handling.yml",
        "#![allow(\n    clippy::indexing_slicing,\n    clippy::as_conversions\n)]\n\npub fn f() {}\n",  # noqa: E501
        '#![allow(\n    clippy::indexing_slicing,\n    reason = "bounds checked above"\n)]\n\npub fn f() {}\n',  # noqa: E501
    ),
    (
        "taskmesh-no-randomness-in-contract-and-engine",
        "determinism.yml",
        "pub fn pick() -> u64 {\n    rand::random::<u64>()\n}\n",
        "pub fn pick(seed: u64) -> u64 {\n    seed\n}\n",
    ),
    (
        "taskmesh-no-tokio-in-contract-engine-rayon",
        "architecture.yml",
        "use tokio::task;\n\npub fn f() {}\n",
        "pub fn f() -> u32 { 1 }\n",
    ),
    (
        "taskmesh-test-len-ge-zero",
        "test-quality.yml",
        "#[test]\nfn t() {\n    let v = vec![1u8];\n    assert!(v.len() >= 0usize);\n}\n",
        "#[test]\nfn t() {\n    let v = vec![1u8];\n    assert_eq!(v.len(), 1);\n}\n",
    ),
]


def _rule_path_for(_rule_id: str) -> str:
    return "crates/taskmesh-contract/src"


def _run_semgrep(config: Path, target_dir: Path) -> set[str]:
    proc = subprocess.run(
        [
            "semgrep",
            "--config",
            str(config),
            "--json",
            "--quiet",
            "--no-git-ignore",
            str(target_dir),
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode not in (0, 1):
        raise RuntimeError(f"semgrep failed ({proc.returncode}): {proc.stderr}")
    data = json.loads(proc.stdout or "{}")
    ids = set()
    for r in data.get("results", []):
        rid = r.get("check_id", "")
        ids.add(rid.split(".")[-1])
    return ids


# Skip only for local convenience when semgrep is absent. Under CI the suite must
# NOT silently skip — a broken runner that drops semgrep would otherwise go green
# without verifying any rule. So in CI a missing binary is a hard failure.
if shutil.which("semgrep") is None and os.environ.get("CI"):
    raise RuntimeError("semgrep is required in CI but was not found on PATH")

pytestmark = pytest.mark.skipif(
    shutil.which("semgrep") is None, reason="semgrep not installed (local only)"
)


@pytest.mark.parametrize("rule_id,rule_file,trigger,clean", CASES, ids=[c[0] for c in CASES])
def test_rule_fires_on_trigger_and_not_on_clean(
    rule_id: str, rule_file: str, trigger: str, clean: str, tmp_path: Path
) -> None:
    config = RULES_DIR / rule_file
    assert config.exists(), f"rule file missing: {config}"

    rel = _rule_path_for(rule_id)

    trig_base = tmp_path / "trig" / rel
    trig_base.mkdir(parents=True, exist_ok=True)
    (trig_base / "trigger.rs").write_text(trigger, encoding="utf-8")
    fired = _run_semgrep(config, tmp_path / "trig")
    assert rule_id in fired, (
        f"{rule_id} did NOT fire on trigger.\nfixture:\n{trigger}\nfired: {sorted(fired)}"
    )

    clean_base = tmp_path / "clean" / rel
    clean_base.mkdir(parents=True, exist_ok=True)
    (clean_base / "clean.rs").write_text(clean, encoding="utf-8")
    fired_clean = _run_semgrep(config, tmp_path / "clean")
    assert rule_id not in fired_clean, (
        f"{rule_id} FALSE-fired on clean.\nfixture:\n{clean}\nfired: {sorted(fired_clean)}"
    )
