"""`tools/bench/validate_bencher_output.py` (bench.yml `trend`).

TM16-019: `cargo bench | tee file` can leave a partial capture behind — the
bench failed mid-run, or only warnings were printed — and a trend store that
accepts it records a bogus data point. Every expected bench group must have
at least one well-formed bencher line, or the step fails before the store.
"""

from __future__ import annotations

import importlib.util
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
VALIDATOR = REPO / "tools" / "bench" / "validate_bencher_output.py"

_spec = importlib.util.spec_from_file_location("validate_bencher_output", VALIDATOR)
assert _spec and _spec.loader
vbo = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(vbo)

FULL = (
    "warning: unused variable `x`\n"
    "test admit_release_success ... bench:       1,234 ns/iter (+/- 56)\n"
    "test admit_unknown_class_reject ... bench:         987 ns/iter (+/- 12)\n"
    "test governance_tax/fifo ... bench:      10,000 ns/iter (+/- 1,000)\n"
)


def check(tmp_path: Path, content: str, *expect: str) -> tuple[int, str]:
    out = tmp_path / "bench-output.txt"
    out.write_text(content, encoding="utf-8")
    import contextlib
    import io

    stderr = io.StringIO()
    with contextlib.redirect_stderr(stderr):
        code = vbo.main([str(out), *[f"--expect={e}" for e in expect]])
    return code, stderr.getvalue()


def test_a_complete_capture_passes(tmp_path: Path) -> None:
    code, err = check(tmp_path, FULL, "admit_release", "governance_tax")
    assert code == 0, err


def test_a_capture_with_no_result_lines_is_refused(tmp_path: Path) -> None:
    """Only warnings (the bench never ran) or an empty file."""
    for content in ("", "warning: something\n   Compiling taskmesh-bench\n"):
        code, err = check(tmp_path, content, "admit_release")
        assert code == 1 and "no bencher result lines" in err, (content, code, err)


def test_a_partial_capture_missing_an_expected_group_is_refused(tmp_path: Path) -> None:
    """The first bench finished, the second died: its group has no line."""
    partial = "\n".join(line for line in FULL.splitlines() if "governance_tax" not in line) + "\n"
    code, err = check(tmp_path, partial, "admit_release", "governance_tax")
    assert code == 1, "a partial capture must not become a data point"
    assert "governance_tax" in err and "admit_release" not in err.split("expected benches")[1], (
        f"the missing group must be named, and only it: {err}"
    )


def test_a_malformed_result_line_does_not_count(tmp_path: Path) -> None:
    """A truncated line for the expected group is not a measurement."""
    truncated = FULL.replace(
        "test governance_tax/fifo ... bench:      10,000 ns/iter (+/- 1,000)",
        "test governance_tax/fifo ... bench:      10,0",
    )
    code, err = check(tmp_path, truncated, "governance_tax")
    assert code == 1 and "governance_tax" in err, err


def test_an_unreadable_capture_is_refused(tmp_path: Path) -> None:
    import contextlib
    import io

    stderr = io.StringIO()
    with contextlib.redirect_stderr(stderr):
        code = vbo.main([str(tmp_path / "does-not-exist.txt"), "--expect=admit_release"])
    assert code == 1 and "cannot read" in stderr.getvalue()
