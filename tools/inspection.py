"""Owned metadata execution and regular-file artifact reads.

Binary stdout/stderr stay byte-exact; execution safety budgets are not SLOs.
File reads reject nonregular descriptors without waiting on FIFO open.
"""

from __future__ import annotations

import locale
import os
import stat
from pathlib import Path

from tools.process_supervisor import SupervisedBinaryProcess, run_process

METADATA_TIMEOUT_SECONDS = 30
METADATA_GRACE_SECONDS = 0.5


class InspectionExecutionError(OSError):
    """Incomplete metadata execution must never become a usable identity."""


def inspect_process(
    command: list[str],
    *,
    cwd: Path,
    env: dict[str, str] | None = None,
    timeout_seconds: float | None = None,
) -> SupervisedBinaryProcess:
    result = run_process(
        command,
        cwd=cwd,
        env=os.environ.copy() if env is None else env,
        timeout_seconds=METADATA_TIMEOUT_SECONDS if timeout_seconds is None else timeout_seconds,
        termination_grace_seconds=METADATA_GRACE_SECONDS,
        binary_output=True,
    )
    assert isinstance(result, SupervisedBinaryProcess)
    if (
        result.interrupted_by_signal is not None
        or result.timed_out
        or result.aborted_early
        or result.returncode is None
    ):
        reason = (
            f"interrupted by signal {result.interrupted_by_signal}"
            if result.interrupted_by_signal is not None
            else "timeout"
            if result.timed_out
            else "unsettled execution"
        )
        raise InspectionExecutionError(
            f"metadata {command[0]} {reason}: {result.stderr.decode(errors='replace')[-1000:]}"
        )
    return result


def metadata_output(
    command: list[str],
    *,
    cwd: Path,
    env: dict[str, str] | None = None,
    timeout_seconds: float | None = None,
) -> bytes:
    result = inspect_process(command, cwd=cwd, env=env, timeout_seconds=timeout_seconds)
    if result.returncode != 0:
        detail = result.stderr.decode(errors="replace")[-1000:]
        raise OSError(f"metadata {' '.join(command[:2])} exit {result.returncode}: {detail}")
    return result.stdout


def read_regular_file(path: Path) -> tuple[bytes, int]:
    flags = os.O_RDONLY | os.O_NONBLOCK | getattr(os, "O_CLOEXEC", 0)
    descriptor = os.open(path, flags)
    try:
        mode = os.fstat(descriptor).st_mode
        if not stat.S_ISREG(mode):
            raise OSError(f"regular file required: {path}")
        with os.fdopen(descriptor, "rb") as stream:
            descriptor = -1
            return stream.read(), mode & 0o777
    finally:
        if descriptor >= 0:
            os.close(descriptor)


def read_regular_bytes(path: Path) -> bytes:
    return read_regular_file(path)[0]


def copy_regular_file(source: Path, destination: Path) -> None:
    data, mode = read_regular_file(source)
    with destination.open("xb") as stream:
        stream.write(data)
        os.fchmod(stream.fileno(), mode)


def read_regular_text(path: Path, encoding: str | None = None, errors: str | None = None) -> str:
    text = read_regular_bytes(path).decode(
        encoding or locale.getpreferredencoding(False), errors or "strict"
    )
    return text.replace("\r\n", "\n").replace("\r", "\n")
