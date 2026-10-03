"""Exercise the CircleCI source binder against real local Git merge refs."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "checkout_verified_source.sh"


def git(repo: Path, *args: str) -> str:
    result = subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True, text=True)
    return result.stdout.strip()


def test_pull_request_binds_github_merge_and_rejects_wrong_head(tmp_path: Path) -> None:
    origin = tmp_path / "origin.git"
    repo = tmp_path / "checkout"
    subprocess.run(["git", "init", "--bare", str(origin)], check=True, capture_output=True)
    subprocess.run(["git", "clone", str(origin), str(repo)], check=True, capture_output=True)
    git(repo, "config", "user.email", "test@example.com")
    git(repo, "config", "user.name", "Test")
    git(repo, "branch", "-M", "main")
    (repo / "source.txt").write_text("base\n", encoding="utf-8")
    git(repo, "add", "source.txt")
    git(repo, "commit", "-m", "base")
    git(repo, "push", "origin", "main")

    git(repo, "switch", "-c", "change")
    (repo / "source.txt").write_text("head\n", encoding="utf-8")
    git(repo, "commit", "-am", "head")
    head = git(repo, "rev-parse", "HEAD")
    git(repo, "switch", "main")
    git(repo, "merge", "--no-ff", "-m", "merge change", "change")
    merge = git(repo, "rev-parse", "HEAD")
    git(repo, "push", "origin", "HEAD:refs/pull/1/merge")
    git(repo, "switch", "--detach", head)

    env = {
        **os.environ,
        "CIRCLE_SHA1": head,
        "CIRCLE_PULL_REQUEST": "https://github.com/josongmin/quanta-taskmesh/pull/1",
        "TASKMESH_CI_EVENT": "pull_request",
    }
    result = subprocess.run(["bash", str(SCRIPT)], cwd=repo, env=env, capture_output=True)
    assert result.returncode == 0, result.stderr.decode()
    assert git(repo, "rev-parse", "HEAD") == merge
    assert (repo / "target/ci/source.env").read_text(encoding="utf-8") == (
        f"export TASKMESH_EXPECTED_SHA={merge}\n"
    )

    base = git(repo, "rev-parse", f"{merge}^1")
    git(repo, "switch", "--detach", base)
    env["CIRCLE_SHA1"] = base
    result = subprocess.run(["bash", str(SCRIPT)], cwd=repo, env=env, capture_output=True)
    assert result.returncode != 0
    assert b"does not contain the triggered head" in result.stderr
