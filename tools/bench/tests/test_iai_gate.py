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
    assert iai_gate.runner_version_from_self_report("") is None


def test_fingerprint_changes_with_every_compatibility_input(tmp_path: Path) -> None:
    bench = tmp_path / "bench.rs"
    bench.write_text("fn main() {}\n")
    base = dict(
        inputs=[bench], schema=2, runner="0.14.2", valgrind="valgrind-3.22", rustc="rustc 1.95.0"
    )
    reference = iai_gate.fingerprint(**base)
    assert reference == iai_gate.fingerprint(**base), "deterministic"
    for key, value in [
        ("schema", 3),
        ("runner", "0.14.3"),
        ("valgrind", "valgrind-3.23"),
        ("rustc", "rustc 1.96.0"),
    ]:
        assert iai_gate.fingerprint(**{**base, key: value}) != reference, key
    bench.write_text("fn main() { let _ = 1; }\n")
    assert iai_gate.fingerprint(**base) != reference, "bench definition"
    # Empty descriptions would make every valgrind (or compiler) hash alike.
    for key in ("valgrind", "rustc"):
        with pytest.raises(ValueError):
            iai_gate.fingerprint(**{**base, key: "  "})
    with pytest.raises(ValueError):
        iai_gate.fingerprint(**{**base, "inputs": []})


def test_every_configured_fingerprint_input_changes_the_fingerprint(tmp_path: Path) -> None:
    """The CI cache key and the stamp both come from `fingerprint` over the
    configured input list. Each configured file must move it: a dependency
    bump (`Cargo.lock`) or a threshold change (`perf-gate.json`) that left the
    fingerprint alone would compare a new measurement against an old baseline."""
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
        if version and self_report:
            report = (
                'echo "iai_callgrind_runner: Error: ... but iai-callgrind-runner '
                f"({version}) is >= '0.3.0'.\" >&2; exit 1\n"
            )
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

    def _write_cargo(self) -> None:
        listing = getattr(self, "_listing", "")
        ok = getattr(self, "_bench_ok", True)
        emits_summary = getattr(self, "_emits_summary", True)
        comparison_complete = getattr(self, "_comparison_complete", True)
        comparison_flag = 1 if comparison_complete else 0
        summary_command = (
            "  printf '"
            '{"version":"3","callgrind_summary":{"callgrind_run":{"total":'
            '{"summary":{"Ir":{"metrics":%s}},"regressions":[]}}}}'
            '\\n\' "$metrics" >target/iai/fake/summary.json\n'
        )
        _shim(
            self.bin,
            "cargo",
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
            "  mkdir -p target/iai/fake\n"
            "  if [[ -f target/iai/fake/callgrind.fake.out "
            f"&& {comparison_flag} -eq 1 ]]; then\n"
            "    metrics='{\"Both\":[101,100]}'\n"
            "  else\n"
            "    metrics='{\"Left\":101}'\n"
            "  fi\n"
            "  echo 'events: Ir' >target/iai/fake/callgrind.fake.out\n"
            + (summary_command if emits_summary else "")
            + "  echo 'Iai-Callgrind result: Ok; 1 benchmark finished'\n"
            "  exit 0\n"
            "fi\n"
            "exit 0\n",
        )

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


def test_a_runner_whose_version_cannot_be_read_says_so_instead_of_dying_silently(
    harness: Harness,
) -> None:
    harness.runner_installed(None)
    proc = harness.run()
    assert proc.returncode == 1
    assert "cannot determine the iai-callgrind-runner version" in proc.stderr
    assert "config requires" in proc.stderr
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
    raw = harness.root / "target" / "iai" / "fake" / "callgrind.fake.out"
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
