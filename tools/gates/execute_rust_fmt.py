#!/usr/bin/env python3
"""Run rustfmt through every Git-visible Cargo manifest, including nested workspaces."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


def cargo_manifests(root: Path) -> list[str]:
    listed = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=root,
        capture_output=True,
        check=True,
    )
    manifests = sorted(
        os.fsdecode(path)
        for path in listed.stdout.split(b"\0")
        if path and path.split(b"/")[-1] == b"Cargo.toml"
    )
    if not manifests or "Cargo.toml" not in manifests:
        raise ValueError("fmt: root Cargo.toml is missing from the source set")
    return manifests


def main(root: Path = REPO, *, check: bool = True) -> int:
    failed = False
    for manifest in cargo_manifests(root):
        command = ["cargo", "fmt", "--manifest-path", manifest, "--all"]
        if check:
            command.append("--check")
        print(f"fmt: {manifest}", flush=True)
        failed |= subprocess.run(command, cwd=root, check=False).returncode != 0
    return int(failed)


if __name__ == "__main__":
    if sys.argv[1:] not in ([], ["--check"]):
        raise SystemExit("usage: execute_rust_fmt.py [--check]")
    raise SystemExit(main(check=bool(sys.argv[1:])))
