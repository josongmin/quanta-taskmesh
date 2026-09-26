"""`tools/bench-iai.sh` and its helper, exercised end to end with PATH shims.

The script used to ask `iai-callgrind-runner --version`, which the runner does
not support: it exits 1 with empty stdout, and under `set -e` the script died
at that line with no diagnostic at all — every CI run red, nothing printed.
These tests drive the real script through every early exit and both outcomes
with fake `uname` / `valgrind` / `cargo` / `rustc` / `iai-callgrind-runner`
on PATH, and assert the diagnostic each path prints.
"""

from __future__ import annotations

import json
import os
import shutil
import stat
import subprocess
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]
SCRIPT = REPO / "tools" / "bench-iai.sh"
HELPER = REPO / "tools" / "bench" / "iai_gate.py"
sys.path.insert(0, str(HELPER.parent))
import iai_gate  # noqa: E402

RUNNER_VERSION = iai_gate.gate_config()["iai_callgrind_runner"]
EMPTY_BUILD_CONTEXT = {
    "environment": {},
    "cargo_configs": {},
    "runner_sha256": "a" * 64,
    "tool_executables": {
        "build.rustc": {"path": "/fixture/rustc", "sha256": "b" * 64},
        "build.cargo": {"path": "/fixture/cargo", "sha256": "c" * 64},
        "build.default-linker": {"path": "/fixture/cc", "sha256": "d" * 64},
    },
}

# This module validates the Linux-only IAI qualification producer, including
# its shell/toolchain boundary. Full `py-test` retains it; the macOS daily
# feedback loop excludes platform/release qualification tooling.
pytestmark = pytest.mark.qualification


# ---- helper unit tests -----------------------------------------------------


def test_runner_version_is_read_from_cargo_install_list() -> None:
    listing = (
        "cargo-deny v0.16.1:\n    cargo-deny\n"
        "iai-callgrind-runner v0.14.2:\n    iai-callgrind-runner\n"
        "just v1.36.0:\n    just\n"
    )
    assert iai_gate.runner_version_from_install_list(listing) == "0.14.2"
    assert iai_gate.runner_version_from_install_list("just v1.36.0:\n    just\n") is None
    # A git/path install carries a suffix before the colon.
    assert (
        iai_gate.runner_version_from_install_list(
            "iai-callgrind-runner v0.16.1 (/home/x/src):\n    iai-callgrind-runner\n"
        )
        == "0.16.1"
    )


def test_a_prerelease_runner_is_not_collapsed_to_its_release_version() -> None:
    listing = "iai-callgrind-runner v0.14.2-rc.1:\n    iai-callgrind-runner\n"
    assert iai_gate.runner_version_from_install_list(listing) == "0.14.2-rc.1"
    assert iai_gate.runner_version_from_self_report("iai-callgrind-runner (0.14.2-rc.1) is") == (
        "0.14.2-rc.1"
    )


def test_runner_version_is_read_from_its_own_error_report() -> None:
    complaint = (
        "iai_callgrind_runner: Error: No version information found for iai-callgrind "
        "but iai-callgrind-runner (0.14.2) is >= '0.3.0'."
    )
    assert iai_gate.runner_version_from_self_report(complaint) == "0.14.2"
    assert (
        iai_gate.runner_version_from_self_report(
            "Detected version of iai-callgrind-runner is 0.14.2."
        )
        == "0.14.2"
    )
    assert iai_gate.runner_version_from_self_report("") is None


def test_raw_ir_prefers_totals_and_requires_a_real_callgrind_file(tmp_path: Path) -> None:
    raw = tmp_path / "callgrind.out"
    raw.write_text(
        "# callgrind format\nversion: 1\nevents: Dr Ir Dw\nsummary: 1 2 3\ntotals: 4 105 6\n",
        encoding="utf-8",
    )
    assert iai_gate._raw_ir(raw) == 105
    raw.write_text("garbage\n", encoding="utf-8")
    assert iai_gate._raw_ir(raw) is None


@pytest.mark.parametrize(
    ("new", "expected_problem"),
    [(105, False), (106, True), (200, True)],
)
def test_ir_limit_is_exact_and_strict(tmp_path: Path, new: int, expected_problem: bool) -> None:
    store = tmp_path.resolve()
    current = store / "case.out"
    old = store / "case.out.old"
    current.write_text(f"version: 1\nevents: Ir\ntotals: {new}\n")
    old.write_text("version: 1\nevents: Ir\ntotals: 100\n")
    summary = {
        "callgrind_summary": {
            "callgrind_run": {
                "total": {"summary": {"Ir": {"metrics": {"Both": [new, 100]}}}},
                "segments": [{"baseline": {"path": str(old)}}],
            }
        }
    }
    _, problems = iai_gate._ir_semantic_problems(summary, store, ["case.out"], comparison=True)
    assert ("Ir regression exceeds reviewed threshold" in problems) is expected_problem


def test_fingerprint_changes_with_every_compatibility_input(tmp_path: Path) -> None:
    bench = tmp_path / "bench.rs"
    bench.write_text("fn main() {}\n")
    base = dict(
        inputs=[bench],
        schema=2,
        runner="0.14.2",
        valgrind="valgrind-3.22",
        rustc="rustc 1.95.0",
        build_context=EMPTY_BUILD_CONTEXT,
    )
    reference = iai_gate.fingerprint(**base)
    assert reference == iai_gate.fingerprint(**base), "deterministic"
    for key, value in [
        ("schema", 3),
        ("runner", "0.14.3"),
        ("valgrind", "valgrind-3.23"),
        ("rustc", "rustc 1.96.0"),
        (
            "build_context",
            {**EMPTY_BUILD_CONTEXT, "environment": {"RUSTFLAGS": "-C opt-level=0"}},
        ),
    ]:
        assert iai_gate.fingerprint(**{**base, key: value}) != reference, key
    bench.write_text("fn main() { let _ = 1; }\n")
    assert iai_gate.fingerprint(**base) != reference, "bench definition"
    # Empty descriptions would make every valgrind (or compiler) hash alike.
    for key in ("valgrind", "rustc"):
        with pytest.raises(
            ValueError, match="fingerprint needs non-empty valgrind and rustc descriptions"
        ):
            iai_gate.fingerprint(**{**base, key: "  "})
    with pytest.raises(ValueError, match="fingerprint needs at least one input file"):
        iai_gate.fingerprint(**{**base, "inputs": []})


def test_every_configured_fingerprint_input_changes_the_fingerprint(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The CI cache key and the stamp both come from `fingerprint` over the
    configured input list. Each configured file must move it: a dependency
    bump (`Cargo.lock`) or a threshold change (`perf-gate.json`) that left the
    fingerprint alone would compare a new measurement against an old baseline."""
    runner_bin = tmp_path / "bin"
    runner_bin.mkdir()
    _shim(runner_bin, "iai-callgrind-runner", "echo 'Detected version 0.14.2' >&2\nexit 1\n")
    monkeypatch.setenv("PATH", f"{runner_bin}{os.pathsep}{os.environ.get('PATH', '')}")
    config = iai_gate.gate_config()
    root = tmp_path / "root"
    for rel in config["fingerprint_inputs"]:
        target = root / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(f"fixture {rel}\n")
    rustc = tmp_path / "rustc.txt"
    rustc.write_text("rustc 1.95.0\n")
    args = [
        "fingerprint",
        "--runner",
        "0.14.2",
        "--valgrind",
        "valgrind-3.22",
        "--rustc-file",
        str(rustc),
        "--root",
        str(root),
    ]

    def run() -> str:
        import io
        from contextlib import redirect_stdout

        buffer = io.StringIO()
        with redirect_stdout(buffer):
            assert iai_gate.main(args) == 0
        return buffer.getvalue().strip()

    reference = run()
    assert len(reference) == 64
    assert len(config["fingerprint_inputs"]) >= 4, "the four compatibility inputs are configured"
    for rel in config["fingerprint_inputs"]:
        target = root / rel
        original = target.read_text()
        target.write_text(original + "changed\n")
        assert run() != reference, f"{rel} does not move the fingerprint"
        target.write_text(original)
        assert run() == reference, f"{rel} restored"


def test_baseline_artifact_paths_cannot_escape_the_store(tmp_path: Path) -> None:
    store = tmp_path / "store"
    store.mkdir()
    outside = tmp_path / "outside.out"
    outside.write_text("events: Ir\n", encoding="utf-8")
    linked = store / "linked.out"
    linked.symlink_to(outside)

    assert iai_gate.confined_artifact(store, "../outside.out") is None
    assert iai_gate.confined_artifact(store, str(outside)) is None
    assert iai_gate.confined_artifact(store, "linked.out") is None
    assert iai_gate.baseline_artifacts(store) == []


def test_raw_output_cannot_alias_through_an_in_store_symlink_directory(tmp_path: Path) -> None:
    store = tmp_path / "store"
    real = store / "real"
    real.mkdir(parents=True)
    (real / "callgrind.out").write_text("events: Ir\n", encoding="utf-8")
    (store / "alias").symlink_to(real, target_is_directory=True)

    assert iai_gate.confined_artifact(store, "alias/callgrind.out") is None
    assert (
        iai_gate._raw_output_paths(
            {"callgrind_summary": {"out_paths": ["alias/callgrind.out"]}}, store
        )
        is None
    )


def test_manifest_rejects_a_baseline_artifact_outside_the_store(tmp_path: Path) -> None:
    store = tmp_path / "store"
    store.mkdir()
    outside = tmp_path / "outside.out"
    outside.write_text("events: Ir\n", encoding="utf-8")
    manifest = {
        "schema_version": iai_gate.BASELINE_MANIFEST_VERSION,
        "kind": "iai-callgrind-baseline",
        "state": "BASELINE_READY",
        "fingerprint": "f" * 64,
        "runner": "0.14.2",
        "valgrind": "valgrind-3.22",
        "rustc": "rustc 1.95.0",
        "build_context": EMPTY_BUILD_CONTEXT,
        "config_sha256": iai_gate._sha256(iai_gate.CONFIG),
        "artifacts": [
            {
                "path": "../outside.out",
                "size": outside.stat().st_size,
                "sha256": iai_gate._sha256(outside),
            }
        ],
    }
    (store / iai_gate.BASELINE_MANIFEST).write_text(json.dumps(manifest), encoding="utf-8")
    problems = iai_gate.baseline_manifest_problems(
        store,
        fingerprint_value="f" * 64,
        runner="0.14.2",
        valgrind="valgrind-3.22",
        rustc="rustc 1.95.0",
        build_context=EMPTY_BUILD_CONTEXT,
    )
    assert "baseline artifact '../outside.out' escapes the baseline store" in problems


def test_symlinked_summary_is_not_comparison_evidence(tmp_path: Path) -> None:
    store = tmp_path / "store"
    (store / "fake").mkdir(parents=True)
    outside = tmp_path / "summary.json"
    outside.write_text(
        json.dumps({"callgrind_summary": {"callgrind_run": {"total": {"summary": {}}}}}),
        encoding="utf-8",
    )
    (store / "fake" / "summary.json").symlink_to(outside)
    result = iai_gate.inspect_summaries(store)
    assert result["selected_count"] == 1
    assert result["executed_count"] == 0
    assert result["comparison_count"] == 0
    assert result["invalid_summaries"] == ["fake/summary.json"]


@pytest.mark.parametrize(
    "metrics",
    [
        {"Dr": {"metrics": {"Both": [101, 100]}}},
        {"Ir": {"metrics": {"Both": [None, None]}}},
        {"Ir": {"metrics": {"Both": ["101", "100"]}}},
        {"Ir": {"metrics": {"Both": [0, 100]}}},
    ],
)
def test_comparison_requires_real_instruction_counts_and_keeps_baseline_on_failure(
    tmp_path: Path, metrics: dict[str, object]
) -> None:
    store = tmp_path / "store"
    (store / "fake").mkdir(parents=True)
    (store / "fake" / "summary.json").write_text(
        json.dumps({"callgrind_summary": {"callgrind_run": {"total": {"summary": metrics}}}}),
        encoding="utf-8",
    )
    (store / "fake" / "callgrind.fake.out").write_text("events: Ir\n", encoding="utf-8")
    (store / "benchmark-output.log").write_text("benchmark finished\n", encoding="utf-8")
    baseline = store / iai_gate.BASELINE_MANIFEST
    baseline.write_text("existing baseline\n", encoding="utf-8")

    status, problems = iai_gate.finalize_run(
        store,
        fingerprint_value="f" * 64,
        runner="0.14.2",
        valgrind="valgrind-3.22",
        rustc="rustc 1.95.0",
        build_context=EMPTY_BUILD_CONTEXT,
        expected_comparison=True,
    )
    assert status == "NOT_RUN"
    assert "verified old-vs-new comparison is incomplete" in problems
    assert baseline.read_text(encoding="utf-8") == "existing baseline\n"


def test_first_run_without_raw_callgrind_output_cannot_create_a_baseline(tmp_path: Path) -> None:
    store = tmp_path / "store"
    (store / "fake").mkdir(parents=True)
    (store / "fake" / "summary.json").write_text(
        json.dumps(
            {
                "callgrind_summary": {
                    "callgrind_run": {"total": {"summary": {"Ir": {"metrics": {"Left": 101}}}}}
                }
            }
        ),
        encoding="utf-8",
    )
    (store / "benchmark-output.log").write_text("benchmark finished\n", encoding="utf-8")
    status, problems = iai_gate.finalize_run(
        store,
        fingerprint_value="f" * 64,
        runner="0.14.2",
        valgrind="valgrind-3.22",
        rustc="rustc 1.95.0",
        build_context=EMPTY_BUILD_CONTEXT,
        expected_comparison=False,
    )
    assert status == "NOT_RUN"
    assert "raw callgrind .out artifact is missing" in problems
    assert not (store / iai_gate.BASELINE_MANIFEST).exists()


@pytest.mark.parametrize(
    "control",
    [
        "RUSTC",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER",
    ],
)
def test_same_path_tool_rewrite_changes_build_context(
    tmp_path: Path, monkeypatch, control: str
) -> None:
    executable = tmp_path / "tool"
    executable.write_text("#!/bin/sh\nexit 0\n")
    executable.chmod(0o755)
    monkeypatch.setenv(control, str(executable))
    before = iai_gate.cargo_build_context(tmp_path)
    executable.write_text("#!/bin/sh\nexit 1\n")
    after = iai_gate.cargo_build_context(tmp_path)
    assert before != after
    assert before["environment"] == after["environment"]
    assert before["tool_executables"] != after["tool_executables"]


@pytest.mark.parametrize(
    "control",
    [
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "CARGO_BUILD_RUSTC",
        "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER",
    ],
)
def test_missing_selected_tool_rejects_build_context(
    tmp_path: Path, monkeypatch, control: str
) -> None:
    monkeypatch.delenv("RUSTC", raising=False)
    monkeypatch.setenv(control, str(tmp_path / "missing"))
    with pytest.raises(ValueError, match="missing or not executable"):
        iai_gate.cargo_build_context(tmp_path)


def test_config_relative_wrapper_bytes_and_environment_override(
    tmp_path: Path, monkeypatch
) -> None:
    monkeypatch.delenv("RUSTC_WRAPPER", raising=False)
    monkeypatch.delenv("CARGO_BUILD_RUSTC_WRAPPER", raising=False)
    cargo_home = tmp_path / "cargo-home"
    cargo_home.mkdir()
    monkeypatch.setenv("CARGO_HOME", str(cargo_home))
    root = tmp_path / "source"
    (root / ".cargo").mkdir(parents=True)
    (root / ".cargo/config.toml").write_text('[build]\nrustc-wrapper = "./wrapper"\n')
    executable = root / "wrapper"
    executable.write_text("#!/bin/sh\nexit 0\n")
    executable.chmod(0o755)
    before = iai_gate.cargo_build_context(root)
    executable.write_text("#!/bin/sh\nexit 1\n")
    after = iai_gate.cargo_build_context(root)
    assert before["cargo_configs"] == after["cargo_configs"]
    assert before["tool_executables"] != after["tool_executables"]
    executable.unlink()
    with pytest.raises(ValueError, match="missing or not executable"):
        iai_gate.cargo_build_context(root)
    monkeypatch.setenv("RUSTC_WRAPPER", "")
    assert "build.rustc-wrapper" not in iai_gate.cargo_build_context(root)["tool_executables"]


@pytest.mark.parametrize("encoded", [False, True])
def test_rustflags_linker_rewrite_and_missing_tool_are_rejected(
    tmp_path: Path, monkeypatch, encoded: bool
) -> None:
    executable = tmp_path / "linker"
    executable.write_text("#!/bin/sh\nexit 0\n")
    executable.chmod(0o755)
    name = "CARGO_ENCODED_RUSTFLAGS" if encoded else "RUSTFLAGS"
    monkeypatch.delenv("CARGO_ENCODED_RUSTFLAGS", raising=False)
    separator = "\x1f" if encoded else " "
    monkeypatch.setenv(name, f"-C{separator}linker={executable}")
    before = iai_gate.cargo_build_context(tmp_path)
    executable.write_text("#!/bin/sh\nexit 1\n")
    assert before != iai_gate.cargo_build_context(tmp_path)
    executable.unlink()
    with pytest.raises(ValueError, match="missing or not executable"):
        iai_gate.cargo_build_context(tmp_path)


def test_path_selected_rustc_bytes_bind_build_context(tmp_path: Path, monkeypatch) -> None:
    monkeypatch.delenv("RUSTC", raising=False)
    monkeypatch.delenv("CARGO_BUILD_RUSTC", raising=False)
    executable = tmp_path / "rustc"
    executable.write_text("#!/bin/sh\nexit 0\n")
    executable.chmod(0o755)
    monkeypatch.setenv("PATH", str(tmp_path) + os.pathsep + os.environ["PATH"])
    before = iai_gate.cargo_build_context(tmp_path)
    executable.write_text("#!/bin/sh\nexit 1\n")
    assert before != iai_gate.cargo_build_context(tmp_path)


def test_rustup_proxy_binds_active_compiler_and_cargo_bytes(tmp_path: Path, monkeypatch) -> None:
    for name in ("RUSTC", "CARGO_BUILD_RUSTC"):
        monkeypatch.delenv(name, raising=False)
    proxy = tmp_path / "rustup"
    proxy.write_text("#!/bin/sh\nexit 0\n")
    proxy.chmod(0o755)
    for tool in ("rustc", "cargo"):
        (tmp_path / tool).symlink_to(proxy)
        executable = tmp_path / ("active-" + tool)
        executable.write_text("#!/bin/sh\nexit 0\n")
        executable.chmod(0o755)
    monkeypatch.setenv("PATH", str(tmp_path) + os.pathsep + os.environ["PATH"])

    def which(command, **kwargs):
        return str(tmp_path / ("active-" + command[-1])).encode()

    monkeypatch.setattr(iai_gate, "metadata_output", which)
    before = iai_gate.cargo_build_context(tmp_path)
    assert before["tool_executables"]["build.rustc"]["path"] == str(tmp_path / "active-rustc")
    assert before["tool_executables"]["build.cargo"]["path"] == str(tmp_path / "active-cargo")
    (tmp_path / "active-rustc").write_text("#!/bin/sh\nexit 1\n")
    after = iai_gate.cargo_build_context(tmp_path)
    assert before != after
    assert (
        before["tool_executables"]["build.rustc.launcher"]
        == after["tool_executables"]["build.rustc.launcher"]
    )


def test_default_linker_bytes_bind_build_context(tmp_path: Path, monkeypatch) -> None:
    executable = tmp_path / "cc"
    executable.write_text("#!/bin/sh\nexit 0\n")
    executable.chmod(0o755)
    monkeypatch.setenv("PATH", str(tmp_path) + os.pathsep + os.environ["PATH"])
    before = iai_gate.cargo_build_context(tmp_path)
    executable.write_text("#!/bin/sh\nexit 1\n")
    assert before != iai_gate.cargo_build_context(tmp_path)


def test_unsupported_target_rejects_default_linker_provenance(tmp_path: Path, monkeypatch) -> None:
    monkeypatch.setenv("CARGO_BUILD_TARGET", "wasm32-unknown-unknown")
    with pytest.raises(ValueError, match="unsupported build target"):
        iai_gate.cargo_build_context(tmp_path)


@pytest.mark.parametrize("as_array", [False, True])
def test_cargo_runner_command_binds_its_first_executable(
    tmp_path: Path, monkeypatch, as_array: bool
) -> None:
    home = tmp_path / "cargo-home"
    home.mkdir()
    monkeypatch.setenv("CARGO_HOME", str(home))
    executable = tmp_path / "runner"
    executable.write_text("#!/bin/sh\nexit 0\n")
    executable.chmod(0o755)
    config = f'[target.x86_64-unknown-linux-gnu]\nrunner = "{executable} --arg"\n'
    if as_array:
        config = f'[target.x86_64-unknown-linux-gnu]\nrunner = ["{executable}", "--arg"]\n'
    (home / "config.toml").write_text(config)
    context = iai_gate.cargo_build_context(tmp_path)
    assert context["tool_executables"]["target.x86_64-unknown-linux-gnu.runner"]["path"] == str(
        executable
    )


def test_ancestor_rustflags_linker_survives_deeper_array_merge(tmp_path: Path, monkeypatch) -> None:
    monkeypatch.delenv("RUSTFLAGS", raising=False)
    monkeypatch.delenv("CARGO_ENCODED_RUSTFLAGS", raising=False)
    monkeypatch.delenv("CARGO_BUILD_RUSTFLAGS", raising=False)
    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setenv("CARGO_HOME", str(home))
    root = tmp_path / "parent/child"
    (root / ".cargo").mkdir(parents=True)
    (root.parent / ".cargo").mkdir()
    linker = tmp_path / "linker"
    linker.write_text("#!/bin/sh\nexit 0\n")
    linker.chmod(0o755)
    (root.parent / ".cargo/config.toml").write_text(
        f'[build]\nrustflags = ["-C", "linker={linker}"]\n'
    )
    (root / ".cargo/config.toml").write_text('[build]\nrustflags = ["-C", "opt-level=3"]\n')
    before = iai_gate.cargo_build_context(root)
    linker.write_text("#!/bin/sh\nexit 1\n")
    after = iai_gate.cargo_build_context(root)
    assert before["cargo_configs"] == after["cargo_configs"]
    assert before != after


@pytest.mark.parametrize("name", ["RUSTC_WRAPPER", "CC", "PATH"])
def test_config_environment_tool_controls_fail_closed(
    tmp_path: Path, monkeypatch, name: str
) -> None:
    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setenv("CARGO_HOME", str(home))
    executable = tmp_path / "wrapper"
    executable.write_text("#!/bin/sh\nexit 0\n")
    executable.chmod(0o755)
    (home / "config.toml").write_text(
        f'[env]\n{name} = {{ value = "../wrapper", force = true, relative = true }}\n'
    )
    for content in ("#!/bin/sh\nexit 0\n", "#!/bin/sh\nexit 1\n"):
        executable.write_text(content)
        with pytest.raises(ValueError, match=r"unattested Cargo \[env\] build control"):
            iai_gate.cargo_build_context(tmp_path)


# ---- the shell script ------------------------------------------------------


def _shim(directory: Path, name: str, body: str) -> None:
    path = directory / name
    path.write_text("#!/usr/bin/env bash\n" + body)
    path.chmod(path.stat().st_mode | stat.S_IEXEC)


class Harness:
    """A copy of the repo's gate inputs plus a shim toolchain on PATH."""

    def __init__(self, tmp_path: Path) -> None:
        self.root = tmp_path / "repo"
        (self.root / "tools" / "bench").mkdir(parents=True)
        shutil.copy(SCRIPT, self.root / "tools" / "bench-iai.sh")
        shutil.copy(HELPER, self.root / "tools" / "bench" / "iai_gate.py")
        for dependency in ("inspection.py", "process_supervisor.py"):
            shutil.copy(REPO / "tools" / dependency, self.root / "tools" / dependency)
        shutil.copy(REPO / "tools" / "bench" / "perf-gate.json", self.root / "tools" / "bench")
        for rel in iai_gate.gate_config()["fingerprint_inputs"]:
            target = self.root / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            source = REPO / rel
            target.write_bytes(source.read_bytes() if source.is_file() else b"fixture\n")
        self.bin = tmp_path / "bin"
        self.bin.mkdir()
        self.log = tmp_path / "cargo.log"
        self.uname("Linux")
        self.valgrind("valgrind-3.22.0")
        self.rustc("rustc 1.95.0 (abc 2026-01-01)\nhost: x86_64-unknown-linux-gnu\n")
        self.runner_installed(RUNNER_VERSION)
        self.bench_succeeds(True)

    def uname(self, kernel: str) -> None:
        _shim(self.bin, "uname", f'echo "{kernel}"\n')

    def valgrind(self, version: str) -> None:
        _shim(self.bin, "valgrind", f'echo "{version}"\n')

    def rustc(self, text: str) -> None:
        _shim(self.bin, "rustc", f"cat <<'EOT'\n{text}\nEOT\n")

    def runner_installed(self, version: str | None, self_report: bool = False) -> None:
        listing = ""
        if version and not self_report:
            listing = f"iai-callgrind-runner v{version}:\n    iai-callgrind-runner\n"
        report = "echo 'iai_callgrind_runner: Error: unusable' >&2; exit 1\n"
        if version:
            report = f"echo 'Detected version of iai-callgrind-runner is {version}.' >&2; exit 1\n"
        _shim(self.bin, "iai-callgrind-runner", report)
        self._listing = listing
        self._write_cargo()

    def bench_succeeds(self, ok: bool) -> None:
        self._bench_ok = ok
        self._write_cargo()

    def bench_emits_summary(self, emits: bool) -> None:
        self._emits_summary = emits
        self._write_cargo()

    def bench_comparison_complete(self, complete: bool) -> None:
        self._comparison_complete = complete
        self._write_cargo()

    def bench_case_count(self, count: int) -> None:
        self._case_count = count
        self._write_cargo()

    def bench_raw_case_count(self, count: int) -> None:
        self._raw_case_count = count
        self._write_cargo()

    def bench_share_raw_reference(self, share: bool) -> None:
        self._share_raw_reference = share
        self._write_cargo()

    def _write_cargo(self) -> None:
        listing = getattr(self, "_listing", "")
        ok = getattr(self, "_bench_ok", True)
        emits_summary = getattr(self, "_emits_summary", True)
        comparison_complete = getattr(self, "_comparison_complete", True)
        comparison_flag = 1 if comparison_complete else 0
        cases = iai_gate.gate_config()["expected_cases"]
        case_commands = ""
        for index, case in enumerate(cases[: getattr(self, "_case_count", len(cases))]):
            module_path, case_id = case.split("#", 1)
            function_name = module_path.rsplit("::", 1)[-1]
            directory = f"target/iai/{function_name}.{case_id}"
            raw_directory = (
                "target/iai/admit_release.roundtrip"
                if getattr(self, "_share_raw_reference", False)
                else directory
            )
            payload = {
                "version": "3",
                "kind": "LibraryBenchmark",
                "module_path": module_path,
                "function_name": function_name,
                "id": case_id,
                "callgrind_summary": {
                    "out_paths": [str(self.root / raw_directory / "callgrind.fake.out")],
                    "callgrind_run": {
                        "total": {"summary": {"Ir": {"metrics": "__METRICS__"}}, "regressions": []},
                        "segments": [
                            {
                                "baseline": {
                                    "path": str(
                                        self.root / raw_directory / "callgrind.fake.out.old"
                                    )
                                }
                            }
                        ],
                    },
                },
            }
            encoded = json.dumps(payload, separators=(",", ":")).replace('"__METRICS__"', "%s")
            case_commands += (
                f"  mkdir -p {directory}\n"
                f"  if [[ -f {directory}/callgrind.fake.out && {comparison_flag} -eq 1 ]]; then\n"
                f"    mv {directory}/callgrind.fake.out {directory}/callgrind.fake.out.old\n"
                "    metrics='{\"Both\":[101,100]}'\n"
                "    ir=101\n"
                "  else\n"
                "    metrics='{\"Left\":100}'\n"
                "    ir=100\n"
                "  fi\n"
            )
            if index < getattr(self, "_raw_case_count", len(cases)):
                case_commands += (
                    "  printf '# callgrind format\\nversion: 1\\nevents: Ir\\ntotals: %s\\n' "
                    f'"$ir" >{directory}/callgrind.fake.out\n'
                )
            if emits_summary:
                case_commands += f"  printf '{encoded}\\n' \"$metrics\" >{directory}/summary.json\n"
        _shim(
            self.bin,
            "cargo-fixture",
            f'echo "cargo $*" >>"{self.log}"\n'
            'if [[ "$1" == "install" ]]; then\n'
            f"  printf '%s' \"{listing}\"\n"
            "  exit 0\n"
            "fi\n"
            'if [[ "$1" == "bench" ]]; then\n'
            '  [[ "${IAI_CALLGRIND_SAVE_SUMMARY:-}" == "json" ]] || {\n'
            '    echo "unsupported summary format" >&2; exit 3;\n'
            "  }\n"
            f"  if [[ {0 if ok else 1} -ne 0 ]]; then echo 'benchmark failed'; exit 1; fi\n"
            + case_commands
            + f"  echo 'Iai-Callgrind result: Ok; {len(cases)} benchmarks finished'\n"
            "  exit 0\n"
            "fi\n"
            "exit 0\n",
        )
        # Runtime fixture payload varies; the selected Cargo launcher is stable.
        _shim(self.bin, "cargo", f'exec bash "{self.bin / "cargo-fixture"}" "$@"\n')

    def run(
        self, *args: str, env: dict[str, str] | None = None
    ) -> subprocess.CompletedProcess[str]:
        environment = {
            **os.environ,
            "PATH": f"{self.bin}:{os.environ['PATH']}",
            "GITHUB_OUTPUT": "",
        }
        environment.pop("IAI_CALLGRIND_REGRESSION", None)
        environment.update(env or {})
        return subprocess.run(
            ["bash", "tools/bench-iai.sh", *args],
            cwd=self.root,
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )

    @property
    def baseline_manifest(self) -> Path:
        return self.root / "target" / "iai" / "baseline-manifest.json"

    @property
    def comparison_manifest(self) -> Path:
        return self.root / "target" / "iai" / "comparison-manifest.json"

    @property
    def fingerprint(self) -> str:
        return json.loads(self.baseline_manifest.read_text())["fingerprint"]


@pytest.fixture
def harness(tmp_path: Path) -> Harness:
    return Harness(tmp_path)


def test_non_linux_is_reported_and_refused(harness: Harness) -> None:
    harness.uname("Darwin")
    proc = harness.run()
    assert proc.returncode == 1
    assert "requires Linux" in proc.stderr
    assert "status=UNSUPPORTED_PLATFORM" in proc.stdout


def test_symlinked_baseline_directory_is_refused(harness: Harness, tmp_path: Path) -> None:
    outside = tmp_path / "outside"
    outside.mkdir()
    (harness.root / "target").mkdir()
    (harness.root / "target" / "iai").symlink_to(outside, target_is_directory=True)
    proc = harness.run()
    assert proc.returncode == 1
    assert "baseline directory target/iai is a symlink" in proc.stderr
    assert list(outside.iterdir()) == []


def test_symlinked_target_parent_cannot_delete_an_external_baseline(
    harness: Harness, tmp_path: Path
) -> None:
    outside = tmp_path / "outside"
    baseline = outside / "iai"
    baseline.mkdir(parents=True)
    marker = baseline / "keep.txt"
    marker.write_text("external baseline\n", encoding="utf-8")
    (harness.root / "target").symlink_to(outside, target_is_directory=True)

    proc = harness.run()
    assert proc.returncode == 1
    assert "target parent is a symlink" in proc.stderr
    assert marker.read_text(encoding="utf-8") == "external baseline\n"


def test_a_runner_whose_version_cannot_be_read_says_so_instead_of_dying_silently(
    harness: Harness,
) -> None:
    harness.runner_installed(None)
    proc = harness.run()
    assert proc.returncode == 1
    assert "cannot determine the iai-callgrind-runner version" in proc.stderr
    assert "config requires" in proc.stderr
    assert not harness.baseline_manifest.exists()


def test_multiline_gate_config_is_refused_before_benchmark(harness: Harness) -> None:
    config_path = harness.root / "tools" / "bench" / "perf-gate.json"
    config = json.loads(config_path.read_text(encoding="utf-8"))
    config["instruction_count_gate"]["bench"] = "iai_governance\nextra"
    config_path.write_text(json.dumps(config), encoding="utf-8")

    proc = harness.run()

    assert proc.returncode == 1
    assert "bench must be a nonempty printable string" in proc.stderr
    assert not harness.log.exists(), "invalid config must not invoke cargo"
    assert not harness.baseline_manifest.exists()


def test_the_runner_version_is_accepted_from_its_own_error_report(harness: Harness) -> None:
    # Not installed through cargo (no `cargo install --list` entry): the
    # runner's own usage error is the fallback and must be enough.
    harness.runner_installed(RUNNER_VERSION, self_report=True)
    proc = harness.run()
    assert proc.returncode == 0, proc.stderr
    assert "status=BASELINE_CREATED" in proc.stdout


def test_a_runner_version_mismatch_names_both_versions(harness: Harness) -> None:
    harness.runner_installed("0.9.9")
    proc = harness.run()
    assert proc.returncode == 1
    assert "iai-callgrind-runner 0.9.9 on PATH" in proc.stderr
    assert f"config requires {RUNNER_VERSION}" in proc.stderr


def test_shadowed_path_runner_is_not_misidentified_by_cargo_list(harness: Harness) -> None:
    _shim(
        harness.bin,
        "iai-callgrind-runner",
        "echo 'Detected version of iai-callgrind-runner is 0.15.0.' >&2; exit 1\n",
    )
    proc = harness.run()
    assert proc.returncode == 1
    assert "PATH runner 0.15.0 differs from cargo install 0.14.2" in proc.stderr


def test_same_version_runner_binary_change_moves_fingerprint(harness: Harness) -> None:
    before = harness.run("fingerprint")
    assert before.returncode == 0, before.stderr
    runner = harness.bin / "iai-callgrind-runner"
    runner.write_text(runner.read_text() + "# changed binary\n")
    after = harness.run("fingerprint")
    assert after.returncode == 0, after.stderr
    assert after.stdout != before.stdout


@pytest.mark.parametrize(
    "name",
    [
        "IAI_CALLGRIND_BASELINE",
        "IAI_CALLGRIND_SAVE_BASELINE",
        "IAI_CALLGRIND_LOAD_BASELINE",
        "IAI_CALLGRIND_FILTER",
    ],
)
def test_ambient_baseline_and_filter_controls_are_rejected(harness: Harness, name: str) -> None:
    proc = harness.run(env={name: "unexpected"})
    assert proc.returncode == 1
    assert f"{name} cannot alter" in proc.stderr


def test_first_run_records_a_baseline_and_second_run_qualifies(harness: Harness) -> None:
    first = harness.run()
    assert first.returncode == 0, first.stderr
    assert "status=BASELINE_CREATED" in first.stdout
    assert "NOT regression-qualified" in first.stderr
    fingerprint = harness.fingerprint
    assert len(fingerprint) == 64

    second = harness.run()
    assert second.returncode == 0, second.stderr
    assert f"status=QUALIFIED fingerprint={fingerprint}" in second.stdout
    assert "NOT regression-qualified" not in second.stderr


def test_partial_benchmark_suite_cannot_qualify(harness: Harness) -> None:
    assert harness.run().returncode == 0
    # A zero-exit runner that emits only one summary must not attest the three
    # governance cases defined by iai_governance.rs.
    harness.bench_case_count(1)
    proc = harness.run()
    assert proc.returncode == 1
    assert "benchmark case inventory mismatch" in proc.stderr
    assert "status=QUALIFIED" not in proc.stdout


def test_every_case_requires_its_own_raw_callgrind_output(harness: Harness) -> None:
    harness.bench_raw_case_count(1)
    proc = harness.run()
    assert proc.returncode == 1
    assert "one or more summary artifacts are invalid" in proc.stderr
    assert not harness.baseline_manifest.exists()


def test_two_cases_cannot_claim_the_same_raw_callgrind_output(harness: Harness) -> None:
    assert harness.run().returncode == 0
    harness.bench_share_raw_reference(True)
    proc = harness.run()
    assert proc.returncode == 1
    assert "raw callgrind output is shared across benchmark cases" in proc.stderr
    assert "status=QUALIFIED" not in proc.stdout


def test_stamp_only_cache_never_qualifies(harness: Harness) -> None:
    stamp = harness.root / "target" / "iai" / "taskmesh-fingerprint"
    stamp.parent.mkdir(parents=True)
    stamp.write_text("0" * 64 + "\n")
    proc = harness.run()
    assert proc.returncode == 0, proc.stderr
    assert "status=BASELINE_CREATED" in proc.stdout
    assert "status=QUALIFIED" not in proc.stdout
    assert "baseline manifest is missing" in proc.stderr


@pytest.mark.parametrize("damage", ["missing", "corrupt"])
def test_missing_or_corrupt_raw_baseline_restarts_without_qualifying(
    harness: Harness, damage: str
) -> None:
    assert harness.run().returncode == 0
    raw = harness.root / "target" / "iai" / "admit_release.roundtrip" / "callgrind.fake.out"
    if damage == "missing":
        raw.unlink()
    else:
        raw.write_text("forged\n")
    proc = harness.run()
    assert proc.returncode == 0, proc.stderr
    assert "status=BASELINE_CREATED" in proc.stdout
    assert "status=QUALIFIED" not in proc.stdout


def test_exit_zero_without_summary_is_not_evidence(harness: Harness) -> None:
    harness.bench_emits_summary(False)
    proc = harness.run()
    assert proc.returncode == 1
    assert "runner produced no summary.json artifacts" in proc.stderr
    assert "status=QUALIFIED" not in proc.stdout


def test_exit_zero_without_old_vs_new_metrics_is_not_qualified(harness: Harness) -> None:
    assert harness.run().returncode == 0
    harness.bench_comparison_complete(False)
    proc = harness.run()
    assert proc.returncode == 1
    assert "verified old-vs-new comparison is incomplete" in proc.stderr
    comparison = json.loads(harness.comparison_manifest.read_text())
    assert comparison["status"] == "NOT_RUN"


def test_the_fingerprint_subcommand_matches_the_manifest_and_runs_no_benchmark(
    harness: Harness,
) -> None:
    printed = harness.run("fingerprint")
    assert printed.returncode == 0, printed.stderr
    assert "cargo bench" not in harness.log.read_text() if harness.log.exists() else True
    run = harness.run()
    assert run.returncode == 0
    assert harness.fingerprint == printed.stdout.strip()


def test_an_incompatible_baseline_is_discarded_not_compared(harness: Harness) -> None:
    assert harness.run().returncode == 0
    old = harness.fingerprint
    # Something the fingerprint covers changes: the compiler.
    harness.rustc("rustc 1.96.0 (def 2026-02-01)\nhost: x86_64-unknown-linux-gnu\n")
    proc = harness.run()
    assert proc.returncode == 0, proc.stderr
    assert "status=BASELINE_CREATED" in proc.stdout
    assert "cached baseline is incomplete, corrupt, or incompatible" in proc.stderr
    assert harness.fingerprint != old


def test_codegen_environment_change_cannot_reuse_an_iai_baseline(harness: Harness) -> None:
    first = harness.run(env={"RUSTFLAGS": "-C opt-level=0"})
    assert first.returncode == 0, first.stderr
    assert "status=BASELINE_CREATED" in first.stdout
    old = harness.fingerprint

    second = harness.run(env={"RUSTFLAGS": "-C opt-level=3"})
    assert second.returncode == 0, second.stderr
    assert "status=BASELINE_CREATED" in second.stdout
    assert harness.fingerprint != old


def test_workspace_bench_profile_changes_iai_fingerprint(harness: Harness) -> None:
    first = harness.run("fingerprint")
    assert first.returncode == 0, first.stderr
    cargo_manifest = harness.root / "Cargo.toml"
    cargo_manifest.write_text(cargo_manifest.read_text() + "\n[profile.bench]\nopt-level = 0\n")
    second = harness.run("fingerprint")
    assert second.returncode == 0, second.stderr
    assert second.stdout != first.stdout


def test_a_failed_benchmark_leaves_no_manifest_behind(harness: Harness) -> None:
    harness.bench_succeeds(False)
    proc = harness.run()
    assert proc.returncode == 1
    assert "benchmark iai_governance failed" in proc.stderr
    assert "no evidence manifest was accepted" in proc.stderr
    assert not harness.baseline_manifest.exists(), (
        "a failed first run must not look like a baseline"
    )

    # And a failed run against an existing baseline keeps that baseline intact.
    harness.bench_succeeds(True)
    assert harness.run().returncode == 0
    manifest = harness.baseline_manifest.read_text()
    harness.bench_succeeds(False)
    assert harness.run().returncode == 1
    assert harness.baseline_manifest.read_text() == manifest


def test_an_environment_threshold_is_overridden_by_the_config_with_a_warning(
    harness: Harness,
) -> None:
    proc = harness.run(env={"IAI_CALLGRIND_REGRESSION": "Ir=99.0"})
    assert proc.returncode == 0, proc.stderr
    assert "IAI_CALLGRIND_REGRESSION=Ir=99.0 in the environment disagrees" in proc.stderr
    assert "regression=Ir=5.0" in proc.stdout


def test_a_valgrind_that_prints_nothing_on_stdout_fails_closed(harness: Harness) -> None:
    # Two valgrinds that both print nothing would fingerprint alike, so their
    # baselines would be compared as if comparable.
    _shim(harness.bin, "valgrind", 'echo "valgrind-3.22.0" >&2\n')
    proc = harness.run()
    assert proc.returncode == 1
    assert "valgrind --version printed nothing on stdout" in proc.stderr
    assert not harness.baseline_manifest.exists()


def test_stray_arguments_are_refused_instead_of_filtering_the_benchmark(harness: Harness) -> None:
    proc = harness.run("no_such_bench_filter")
    assert proc.returncode == 1
    assert "unexpected argument(s): 'no_such_bench_filter'" in proc.stderr
    assert not harness.baseline_manifest.exists(), "a filtered run must not create a baseline"
    proc = harness.run("fingerprint", "extra")
    assert proc.returncode == 1
    assert "unexpected argument(s)" in proc.stderr


def test_github_output_receives_status_and_fingerprint(harness: Harness, tmp_path: Path) -> None:
    output = tmp_path / "github-output"
    output.write_text("")
    proc = harness.run(env={"GITHUB_OUTPUT": str(output)})
    assert proc.returncode == 0, proc.stderr
    lines = output.read_text().splitlines()
    assert lines[0] == "status=BASELINE_CREATED"
    assert lines[1].startswith("fingerprint=") and len(lines[1]) == len("fingerprint=") + 64
