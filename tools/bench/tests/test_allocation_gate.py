"""TM16-038 regression: the allocation gate parses, it does not coerce.

The original gate extracted `[0-9.][0-9.]*` with `sed` and compared with
`awk v + 0`. The table below is the actual behaviour that was observed against
the real script; every "false green" row must now be a failure.

These tests drive the real gate entry points (`tools/bench-gate.sh` and the
Python parser it delegates to) with captured producer output, so they exercise
the same code path CI runs — not a re-implementation of the grammar.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]
GATE = REPO / "tools" / "bench-gate.sh"
PARSER = REPO / "tools" / "bench" / "allocation_gate.py"
CONFIG = json.loads((REPO / "tools" / "bench" / "perf-gate.json").read_text())["allocation_gate"]


def metric_line(**overrides: object) -> str:
    fields = {
        "schema": CONFIG["producer_schema"],
        "attempted": 200000,
        "completed": 200000,
        "allocs_per_op": "3.000",
        "final_inflight": 0,
        "counter_check": "1000/1000",
    }
    fields.update(overrides)
    body = " ".join(f"{k}={v}" for k, v in fields.items())
    return f"{CONFIG['producer_marker']} {body}\n"


def run_gate(
    output: str, tmp_path: Path, threshold: str | None = None
) -> subprocess.CompletedProcess[str]:
    captured = tmp_path / "producer.txt"
    captured.write_text(output, encoding="utf-8")
    env = {"PATH": "/usr/bin:/bin:/usr/local/bin", "HOME": str(tmp_path)}
    if threshold is not None:
        env["MAX_ALLOCS_PER_OP"] = threshold
    return subprocess.run(
        ["bash", str(GATE), "--input", str(captured)],
        capture_output=True,
        text=True,
        check=False,
        cwd=REPO,
        env=env,
    )


# ---- controls -----------------------------------------------------------------


def test_below_limit_passes(tmp_path: Path) -> None:
    proc = run_gate(metric_line(allocs_per_op="3.000"), tmp_path, threshold="4")
    assert proc.returncode == 0, proc.stderr
    assert "allocation gate passed" in proc.stdout


def test_equal_to_limit_passes(tmp_path: Path) -> None:
    proc = run_gate(metric_line(allocs_per_op="4.000"), tmp_path, threshold="4")
    assert proc.returncode == 0, proc.stderr


def test_above_limit_fails_with_the_numbers(tmp_path: Path) -> None:
    proc = run_gate(metric_line(allocs_per_op="5.000"), tmp_path, threshold="4")
    assert proc.returncode == 1
    assert "5.0 allocs/op exceeds baseline 4.0" in proc.stderr


# ---- the rows that used to be false green ------------------------------------


@pytest.mark.parametrize(
    "malformed",
    ["...", "3.0.0", "3.", ".5", "3.0junk", "-1", "NaN", "inf", "Infinity", "1e3", "+3", "0x3"],
)
def test_malformed_metric_values_are_rejected(malformed: str, tmp_path: Path) -> None:
    proc = run_gate(metric_line(allocs_per_op=malformed), tmp_path, threshold="4")
    assert proc.returncode == 1, f"{malformed!r} must not pass: {proc.stdout}"
    assert "allocation gate" in proc.stderr


@pytest.mark.parametrize("threshold", ["...", "3.0.0", "NaN", "inf", "-1", "4junk", ""])
def test_malformed_thresholds_are_rejected_even_with_a_valid_metric(
    threshold: str, tmp_path: Path
) -> None:
    proc = run_gate(metric_line(allocs_per_op="3.000"), tmp_path, threshold=threshold)
    assert proc.returncode == 1, f"threshold {threshold!r} must not pass: {proc.stdout}"


def test_missing_marker_fails(tmp_path: Path) -> None:
    proc = run_gate("admit+release allocations/op = 3.000\n", tmp_path, threshold="4")
    assert proc.returncode == 1
    assert "expected exactly one" in proc.stderr


def test_duplicate_marker_fails(tmp_path: Path) -> None:
    proc = run_gate(metric_line() + metric_line(allocs_per_op="1.000"), tmp_path, threshold="4")
    assert proc.returncode == 1
    assert "found 2" in proc.stderr


def test_trailing_junk_on_the_line_fails(tmp_path: Path) -> None:
    proc = run_gate(metric_line().rstrip("\n") + " extra=1\n", tmp_path, threshold="4")
    assert proc.returncode == 1
    assert "unknown fields" in proc.stderr


# ---- measurement contract (TM16-018) ------------------------------------------


def test_incomplete_producer_run_is_not_a_measurement(tmp_path: Path) -> None:
    proc = run_gate(metric_line(completed=199999), tmp_path, threshold="4")
    assert proc.returncode == 1
    assert "completed 199999 of 200000" in proc.stderr


def test_undrained_ledger_is_not_a_measurement(tmp_path: Path) -> None:
    proc = run_gate(metric_line(final_inflight=1), tmp_path, threshold="4")
    assert proc.returncode == 1
    assert "did not drain" in proc.stderr


def test_zero_attempts_is_not_a_measurement(tmp_path: Path) -> None:
    proc = run_gate(metric_line(attempted=0, completed=0), tmp_path, threshold="4")
    assert proc.returncode == 1
    assert "zero operations" in proc.stderr


def test_a_schema_change_breaks_baseline_compatibility(tmp_path: Path) -> None:
    proc = run_gate(metric_line(schema=CONFIG["producer_schema"] + 1), tmp_path, threshold="4")
    assert proc.returncode == 1
    assert "measurement definition changed" in proc.stderr


def test_producer_failure_propagates_its_own_exit_code(tmp_path: Path) -> None:
    """A producer that exits non-zero is a producer failure, not a parse failure."""
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir()
    fake_cargo = fake_bin / "cargo"
    fake_cargo.write_text("#!/usr/bin/env bash\necho 'boom' >&2\nexit 17\n", encoding="utf-8")
    fake_cargo.chmod(0o755)
    proc = subprocess.run(
        [sys.executable, str(PARSER)],
        capture_output=True,
        text=True,
        check=False,
        cwd=REPO,
        env={"PATH": f"{fake_bin}:/usr/bin:/bin", "HOME": str(tmp_path)},
    )
    assert proc.returncode == 17
    assert "boom" in proc.stderr


def test_the_configured_threshold_is_itself_valid() -> None:
    """The committed config must satisfy the grammar it enforces."""
    proc = subprocess.run(
        [sys.executable, str(PARSER), "--input", "/dev/null"],
        capture_output=True,
        text=True,
        check=False,
        cwd=REPO,
    )
    # /dev/null has no marker: the failure must be about the marker, which means
    # the threshold parsed first.
    assert proc.returncode == 1
    assert "expected exactly one" in proc.stderr


# ---- the instrument itself ----------------------------------------------------


def test_a_counter_that_cannot_see_known_allocations_is_not_a_measurement(tmp_path: Path) -> None:
    # A broken counter reporting 0 allocs/op would pass any threshold. The
    # probe's self-check line is required, and it must add up.
    for bad in ["999/1000", "0/1000", "0/0", "1000", "1000/", "a/b"]:
        proc = run_gate(metric_line(counter_check=bad), tmp_path)
        assert proc.returncode == 1, (bad, proc.stdout, proc.stderr)
        assert "FAIL: allocation gate" in proc.stderr, bad
    proc = run_gate(metric_line(allocs_per_op="0.000"), tmp_path)
    assert proc.returncode == 0, "0.000 with a passing self-check is a real (excellent) measurement"


def test_the_threshold_is_the_measured_baseline_not_a_cushion() -> None:
    # The SEP-21 +5 alloc/op rebaseline is explicit. The gate still admits no
    # additional allocation beyond the measured value.
    baseline = 8.0
    assert float(CONFIG["max_allocs_per_op"]) == baseline
    assert "regression" in CONFIG["threshold_note"]


@pytest.mark.slow
def test_the_real_producer_refuses_to_count_a_rejected_admission_as_an_op() -> None:
    # H16-015: a cycle that is not `Admitted` is a different operation, not a
    # cheaper one. The real parser entry point launches the real producer once,
    # propagates its code, and must not manufacture a metric for the refused op.
    proc = subprocess.run(
        [sys.executable, str(PARSER)],
        capture_output=True,
        text=True,
        check=False,
        cwd=REPO,
        env={**os.environ, "TASKMESH_ALLOC_PROBE_FORCE": "reject"},
    )
    assert proc.returncode == 2, (proc.stdout, proc.stderr)
    assert "admission was not Admitted" in proc.stderr
    assert CONFIG["producer_marker"] not in proc.stdout, "no metric line on a refused run"
