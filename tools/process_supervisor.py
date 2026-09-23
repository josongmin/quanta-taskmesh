"""Run a child process group with bounded timeout and cancellation cleanup."""

from __future__ import annotations

import os
import signal
import subprocess
import threading
import time
from dataclasses import dataclass
from pathlib import Path

TERMINATION_GRACE_SECONDS = 5.0
POLL_INTERVAL_SECONDS = 0.2


@dataclass(frozen=True)
class SupervisedProcess:
    returncode: int | None
    stdout: str
    stderr: str
    timed_out: bool
    interrupted_by_signal: int | None


def run_process(
    argv: list[str],
    *,
    cwd: Path,
    env: dict[str, str],
    timeout_seconds: float,
) -> SupervisedProcess:
    """Capture complete output and reap the owned process group on every exit path."""
    if timeout_seconds <= 0:
        raise ValueError("timeout_seconds must be positive")
    if threading.current_thread() is not threading.main_thread():
        raise RuntimeError("supervised processes must be launched from the main thread")

    process: subprocess.Popen[str] | None = None
    timed_out = False
    interrupted_by_signal: int | None = None
    termination_deadline: float | None = None

    def signal_group(signum: int) -> None:
        if process is None:
            return
        try:
            os.killpg(process.pid, signum)
        except ProcessLookupError:
            pass

    def cancel(signum: int, _frame: object) -> None:
        nonlocal interrupted_by_signal, termination_deadline
        if interrupted_by_signal is not None:
            signal_group(signal.SIGKILL)
            return
        interrupted_by_signal = signum
        signal_group(signal.SIGTERM)
        termination_deadline = time.monotonic() + TERMINATION_GRACE_SECONDS

    previous_handlers: dict[int, object] = {}
    stdout = stderr = ""
    try:
        # Install the handler before Popen so cancellation cannot land in the
        # spawn-to-handler gap and strand an unsupervised child.
        for signum in (signal.SIGINT, signal.SIGTERM):
            previous_handlers[signum] = signal.getsignal(signum)
            signal.signal(signum, cancel)
        process = subprocess.Popen(
            argv,
            cwd=cwd,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            start_new_session=True,
        )
        timeout_deadline = time.monotonic() + timeout_seconds
        if interrupted_by_signal is not None:
            signal_group(signal.SIGTERM)
            if termination_deadline is None:
                termination_deadline = time.monotonic() + TERMINATION_GRACE_SECONDS
        while True:
            now = time.monotonic()
            if timed_out and termination_deadline is not None and now >= termination_deadline:
                signal_group(signal.SIGKILL)
                termination_deadline = None
            elif interrupted_by_signal is not None and termination_deadline is not None:
                if now >= termination_deadline:
                    signal_group(signal.SIGKILL)
                    termination_deadline = None
            elif not timed_out and interrupted_by_signal is None and now >= timeout_deadline:
                timed_out = True
                signal_group(signal.SIGTERM)
                termination_deadline = now + TERMINATION_GRACE_SECONDS

            deadlines = (
                [timeout_deadline] if not timed_out and interrupted_by_signal is None else []
            )
            if termination_deadline is not None:
                deadlines.append(termination_deadline)
            until_deadline = (
                max(0.001, min(deadlines) - time.monotonic())
                if deadlines
                else POLL_INTERVAL_SECONDS
            )
            wait_seconds = min(POLL_INTERVAL_SECONDS, until_deadline)
            try:
                stdout, stderr = process.communicate(timeout=wait_seconds)
                break
            except subprocess.TimeoutExpired:
                continue
    except BaseException:
        if process is not None:
            signal_group(signal.SIGTERM)
            try:
                process.communicate(timeout=TERMINATION_GRACE_SECONDS)
            except subprocess.TimeoutExpired:
                signal_group(signal.SIGKILL)
                process.communicate()
            except BaseException:
                signal_group(signal.SIGKILL)
                process.wait()
        raise
    finally:
        for signum, handler in previous_handlers.items():
            signal.signal(signum, handler)

    assert process is not None
    return SupervisedProcess(
        returncode=process.returncode,
        stdout=stdout or "",
        stderr=stderr or "",
        timed_out=timed_out,
        interrupted_by_signal=interrupted_by_signal,
    )
