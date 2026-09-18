"""`tools/bench/history_branch.sh` (bench.yml `trend`) with a `git` shim.

TM16-019: the workflow used to turn *every* non-zero `git ls-remote` into
"history branch missing" and skip the trend compare — including transport and
auth failures (exit 128). Only exit 2 (`--exit-code`: ref not found) means
missing; anything else must fail the step with git's own diagnostic.
"""

from __future__ import annotations

import os
import stat
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
SCRIPT = REPO / "tools" / "bench" / "history_branch.sh"
WORKFLOW = REPO / ".github" / "workflows" / "bench.yml"


def git_shim(tmp_path: Path, exit_code: int, stderr: str = "") -> Path:
    """A `git` that records its argv, prints `stderr`, and exits `exit_code`."""
    fake_bin = tmp_path / "bin"
    fake_bin.mkdir(exist_ok=True)
    shim = fake_bin / "git"
    shim.write_text(
        "#!/bin/sh\n"
        f'printf "%s\\n" "$@" > "{tmp_path}/git.argv"\n'
        f'printf "%s" "{stderr}" >&2\n'
        f"exit {exit_code}\n",
        encoding="utf-8",
    )
    shim.chmod(shim.stat().st_mode | stat.S_IXUSR)
    return fake_bin


def run(tmp_path: Path, exit_code: int, stderr: str = "") -> subprocess.CompletedProcess[str]:
    fake_bin = git_shim(tmp_path, exit_code, stderr)
    return subprocess.run(
        ["bash", str(SCRIPT), "origin", "refs/heads/gh-pages"],
        capture_output=True,
        text=True,
        check=False,
        env={"PATH": f"{fake_bin}:/usr/bin:/bin", "HOME": str(tmp_path), "TMPDIR": str(tmp_path)},
    )


def test_an_existing_branch_reads_exists_true(tmp_path: Path) -> None:
    proc = run(tmp_path, 0)
    assert proc.returncode == 0, proc.stderr
    assert proc.stdout == "exists=true\n"
    argv = (tmp_path / "git.argv").read_text(encoding="utf-8").split()
    assert argv == ["ls-remote", "--exit-code", "origin", "refs/heads/gh-pages"], (
        "the ref must be asked for with --exit-code, or 'missing' has no distinct code"
    )


def test_a_missing_ref_reads_exists_false_and_is_not_an_error(tmp_path: Path) -> None:
    proc = run(tmp_path, 2)
    assert proc.returncode == 0, proc.stderr
    assert proc.stdout == "exists=false\n"


def test_an_infrastructure_failure_is_not_a_missing_branch(tmp_path: Path) -> None:
    """Exit 128 is what git returns for auth / transport failures. Reading it as
    'no history yet' would skip the compare with a green check."""
    proc = run(tmp_path, 128, stderr="fatal: could not read Username for 'https://github.com'")
    assert proc.returncode == 128, (
        f"an infrastructure failure must not read as a missing branch: {proc.stdout!r}"
    )
    assert proc.stdout == "", "no exists= line may reach $GITHUB_OUTPUT on a failure"
    assert "not 'missing ref'" in proc.stderr
    assert "could not read Username" in proc.stderr, "git's own diagnostic must be surfaced"


def test_the_workflow_runs_this_script_and_git_tracks_it() -> None:
    """The fixture is only evidence for the workflow if the workflow runs the
    script (not a re-inlined copy), and only for a clone if git tracks it."""
    workflow = WORKFLOW.read_text(encoding="utf-8")
    assert "bash tools/bench/history_branch.sh origin refs/heads/gh-pages" in workflow
    assert "git ls-remote" not in workflow, "the exit-code logic must not be duplicated inline"
    tracked = subprocess.run(
        ["git", "ls-files", "--error-unmatch", "tools/bench/history_branch.sh"],
        capture_output=True,
        text=True,
        check=False,
        cwd=REPO,
    )
    assert tracked.returncode == 0, "tools/bench/history_branch.sh is not tracked by git"
    assert os.access(SCRIPT, os.X_OK)
