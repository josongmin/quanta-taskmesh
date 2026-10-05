"""Supervised execution shared by release evidence producers."""

from __future__ import annotations

import os
from pathlib import Path

from tools.process_supervisor import SupervisedBinaryProcess, run_process

MAX_CAPTURE_BYTES = 64 * 1024 * 1024


def run_release_command(
    argv: list[str], *, cwd: Path, timeout_seconds: float
) -> SupervisedBinaryProcess:
    """Own the process group and retain byte-exact output within a finite budget."""
    result = run_process(
        argv,
        cwd=cwd,
        env=os.environ.copy(),
        timeout_seconds=timeout_seconds,
        binary_output=True,
        max_capture_bytes=MAX_CAPTURE_BYTES,
    )
    assert isinstance(result, SupervisedBinaryProcess)
    return result


def settled_exit_code(process: SupervisedBinaryProcess) -> int | None:
    """An exit code alone cannot attest completed execution."""
    if process.timed_out or process.interrupted_by_signal is not None or process.aborted_early:
        return None
    return process.returncode
