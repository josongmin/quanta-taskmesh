#!/usr/bin/env python3
"""Lint every Python source that can enter this repository's Git source set."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


def python_sources(root: Path) -> list[str]:
    listed = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=root,
        capture_output=True,
        check=True,
    )
    paths = sorted(
        os.fsdecode(path)
        for path in listed.stdout.split(b"\0")
        if path and path.endswith(b".py")
    )
    if not paths:
        raise ValueError("py-lint: no Python sources discovered")
    return paths


def main(root: Path = REPO) -> int:
    return subprocess.run(
        ["ruff", "check", "--no-respect-gitignore", "--", *python_sources(root)],
        cwd=root,
        check=False,
    ).returncode


if __name__ == "__main__":
    raise SystemExit(main())
