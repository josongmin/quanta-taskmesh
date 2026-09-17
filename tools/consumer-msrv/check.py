#!/usr/bin/env python3
"""Consumer MSRV check (H16-019 / TM16-021).

Compiles `tools/consumer-msrv` — a standalone package outside the root
workspace — on the toolchain the library *declares* as its minimum, with the
default feature set and again with `rayon`.

Two things are deliberately kept apart:

- the **consumer MSRV** (`rust-version` in the root `Cargo.toml`), which is a
  promise to downstream users about the library crates; and
- the **developer toolchain floor**, which is whatever the root workspace's
  dev/bench/proof dependency graph needs and is allowed to be newer.

A missing toolchain is reported as NOT_RUN with exit code 2. It is never
reported as a pass: "we did not check" and "it works" are different results
and the qualification receipt treats them differently.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
FIXTURE = Path(__file__).resolve().parent


def declared_msrv(manifest: Path) -> str:
    text = manifest.read_text(encoding="utf-8")
    match = re.search(r'^rust-version\s*=\s*"([^"]+)"', text, re.MULTILINE)
    if not match:
        raise SystemExit(f"{manifest} declares no rust-version")
    return match.group(1)


def toolchain_present(version: str) -> bool:
    proc = subprocess.run(
        ["rustup", "toolchain", "list"], capture_output=True, text=True, check=False
    )
    if proc.returncode != 0:
        return False
    return any(
        line.startswith(f"{version}-") or line.startswith(f"{version}.")
        for line in proc.stdout.splitlines()
    ) or any(line.split("-", 1)[0] == version for line in proc.stdout.splitlines())


def resolve_toolchain(version: str) -> str | None:
    """The installed toolchain name that satisfies `version` (exact or x.y.0)."""
    proc = subprocess.run(
        ["rustup", "toolchain", "list"], capture_output=True, text=True, check=False
    )
    if proc.returncode != 0:
        return None
    candidates = [line.split()[0] for line in proc.stdout.splitlines() if line.strip()]
    for candidate in candidates:
        name = candidate.split("-", 1)[0]
        if name == version or name == f"{version}.0":
            return candidate
    return None


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--install",
        action="store_true",
        help="install the declared toolchain via rustup if missing",
    )
    args = parser.parse_args(argv)

    workspace_msrv = declared_msrv(REPO / "Cargo.toml")
    fixture_msrv = declared_msrv(FIXTURE / "Cargo.toml")
    if workspace_msrv != fixture_msrv:
        print(
            f"FAIL: consumer fixture declares rust-version {fixture_msrv} "
            f"but the workspace declares {workspace_msrv}",
            file=sys.stderr,
        )
        return 1

    toolchain = resolve_toolchain(workspace_msrv)
    if toolchain is None and args.install:
        subprocess.run(
            ["rustup", "toolchain", "install", workspace_msrv, "--profile", "minimal"], check=False
        )
        toolchain = resolve_toolchain(workspace_msrv)
    if toolchain is None:
        print(
            f"taskmesh-consumer-msrv status=NOT_RUN msrv={workspace_msrv} reason=toolchain-missing "
            f"(install with: rustup toolchain install {workspace_msrv}, or pass --install)"
        )
        return 2

    results = []
    for label, extra in [("default", []), ("rayon", ["--features", "rayon"])]:
        cmd = [
            "cargo",
            f"+{toolchain}",
            "check",
            "--locked",
            "--manifest-path",
            str(FIXTURE / "Cargo.toml"),
            *extra,
        ]
        lock = FIXTURE / "Cargo.lock"
        if not lock.exists():
            # First run: resolve on the MSRV toolchain itself so the lockfile is
            # one that toolchain can read.
            cmd.remove("--locked")
        proc = subprocess.run(cmd, capture_output=True, text=True, check=False, cwd=FIXTURE)
        status = "PASS" if proc.returncode == 0 else "FAIL"
        results.append((label, status))
        print(f"taskmesh-consumer-msrv surface={label} toolchain={toolchain} status={status}")
        if proc.returncode != 0:
            sys.stderr.write(proc.stderr[-6000:])

    if any(status != "PASS" for _, status in results):
        return 1
    print(f"taskmesh-consumer-msrv status=PASS msrv={workspace_msrv} toolchain={toolchain}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
