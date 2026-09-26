# ruff: noqa: UP045 -- Python 3.9 annotations.
"""Owned, bounded subprocesses for diagnostic benchmark acquisition.

Inner builds/probes terminate before the enclosing control arm or study owner.
Deadlines are execution safety limits, not performance budgets.
"""

from __future__ import annotations

import math
import sys
from pathlib import Path
from typing import Callable, Optional

REPO = Path(__file__).resolve().parents[2]
if str(REPO) not in sys.path:
    sys.path.insert(0, str(REPO))
from acquisition_process import run_acquisition  # noqa: E402

from tools.process_supervisor import SupervisedProcess  # noqa: E402

BUILD_TIMEOUT_SECONDS = 1800
PROBE_TIMEOUT_SECONDS = 1800
CONTROL_TIMEOUT_SECONDS = 3600
INNER_GRACE_SECONDS = 1.0
CONTROL_GRACE_SECONDS = 3.0


def run_bench(
    command: list[str],
    *,
    cwd: Path,
    timeout_seconds: float,
    termination_grace_seconds: float = INNER_GRACE_SECONDS,
    on_started: Optional[Callable[[int], None]] = None,
    input_text: Optional[str] = None,
) -> SupervisedProcess:
    if (
        type(timeout_seconds) not in (int, float)
        or not math.isfinite(timeout_seconds)
        or timeout_seconds <= 0
    ):
        raise ValueError("benchmark timeout_seconds must be finite and positive")
    try:
        started = (lambda process: on_started(process.pid)) if on_started is not None else None
        result = run_acquisition(
            command,
            cwd=cwd,
            timeout_seconds=timeout_seconds,
            termination_grace_seconds=termination_grace_seconds,
            on_started=started,
            stdin_data=input_text.encode("utf-8") if input_text is not None else None,
        )
    except UnicodeError as error:
        raise OSError("benchmark capture is not valid text") from error
    reason = None
    if result.interrupted_by_signal is not None:
        reason = f"benchmark interrupted by signal {result.interrupted_by_signal}"
    elif result.timed_out:
        reason = f"benchmark timeout after {timeout_seconds}s"
    elif result.aborted_early:
        reason = "benchmark aborted before completion"
    elif result.returncode is None:
        reason = "benchmark leader exited without settled process group"
    if reason is None:
        return result
    return SupervisedProcess(
        None,
        result.stdout,
        result.stderr + "\n" + reason + "\n",
        result.timed_out,
        result.interrupted_by_signal,
        result.aborted_early,
    )


def run_control(command: list[str], *, cwd: Path) -> SupervisedProcess:
    try:
        return run_bench(
            command,
            cwd=cwd,
            timeout_seconds=CONTROL_TIMEOUT_SECONDS,
            termination_grace_seconds=CONTROL_GRACE_SECONDS,
        )
    except OSError as error:
        return SupervisedProcess(
            None, "", f"benchmark launch/capture failed: {error}\n", False, None
        )
