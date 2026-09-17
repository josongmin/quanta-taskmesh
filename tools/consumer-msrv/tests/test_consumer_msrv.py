"""The consumer-MSRV check must be honest about what it did not do (H16-019)."""

from __future__ import annotations

import importlib.util
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
CHECK = REPO / "tools" / "consumer-msrv" / "check.py"

_spec = importlib.util.spec_from_file_location("consumer_msrv_check", CHECK)
assert _spec and _spec.loader
check = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(check)


def test_fixture_and_workspace_declare_the_same_msrv() -> None:
    assert check.declared_msrv(REPO / "Cargo.toml") == check.declared_msrv(
        REPO / "tools" / "consumer-msrv" / "Cargo.toml"
    )


def test_fixture_is_its_own_workspace() -> None:
    """Root dev-deps and the bench harness must not leak into the consumer graph."""
    manifest = (REPO / "tools" / "consumer-msrv" / "Cargo.toml").read_text(encoding="utf-8")
    assert re.search(r"^\[workspace\]\s*$", manifest, re.MULTILINE), (
        "fixture must declare an empty [workspace]"
    )
    assert "criterion" not in manifest
    assert "iai" not in manifest


def test_fixture_lockfile_is_committed_for_locked_builds() -> None:
    assert (REPO / "tools" / "consumer-msrv" / "Cargo.lock").is_file()


def test_a_missing_toolchain_is_not_run_not_pass(tmp_path: Path) -> None:
    """Exit 2 + NOT_RUN. Never exit 0 without having compiled anything."""
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir()
    # A rustup that knows no toolchains at all.
    (fake_bin / "rustup").write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
    (fake_bin / "rustup").chmod(0o755)
    proc = subprocess.run(
        [sys.executable, str(CHECK)],
        capture_output=True,
        text=True,
        check=False,
        cwd=REPO,
        env={"PATH": f"{fake_bin}:/usr/bin:/bin", "HOME": str(tmp_path)},
    )
    assert proc.returncode == 2, proc.stdout + proc.stderr
    assert "status=NOT_RUN" in proc.stdout
    assert "status=PASS" not in proc.stdout


def test_the_declared_msrv_is_a_real_version_string() -> None:
    assert re.fullmatch(r"\d+\.\d+(\.\d+)?", check.declared_msrv(REPO / "Cargo.toml"))
