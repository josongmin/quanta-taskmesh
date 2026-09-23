"""Run a child process group with bounded timeout and cancellation cleanup."""

from __future__ import annotations

import os
import signal
import subprocess
import threading
import time
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
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


@dataclass(frozen=True)
class SupervisedCommand:
    argv: list[str]
    cwd: Path
    env: dict[str, str]
    timeout_seconds: float


@dataclass(frozen=True)
class SupervisedBatchResult:
    process: SupervisedProcess
    duration_s: float


def run_process_batch(commands: list[SupervisedCommand]) -> list[SupervisedBatchResult]:
    """Run a bounded batch with one main-thread signal owner for every child group.

    Reader threads only drain pipes. They never launch processes or install signal
    handlers, so interruption and timeout cleanup stay owned by this call.
    Results retain command order, not completion order.
    """
    if threading.current_thread() is not threading.main_thread():
        raise RuntimeError("supervised processes must be launched from the main thread")
    if any(command.timeout_seconds <= 0 for command in commands):
        raise ValueError("timeout_seconds must be positive")
    if not commands:
        return []

    processes: list[subprocess.Popen[str] | None] = [None] * len(commands)
    started = [0.0] * len(commands)
    ended = [0.0] * len(commands)
    deadlines = [0.0] * len(commands)
    termination_deadlines: list[float | None] = [None] * len(commands)
    timed_out = [False] * len(commands)
    interrupted = [None] * len(commands)
    outputs: list[tuple[str, str]] = [("", "")] * len(commands)
    interrupted_by_signal: int | None = None

    def signal_group(index: int, signum: int) -> None:
        process = processes[index]
        if process is None:
            return
        try:
            os.killpg(process.pid, signum)
        except ProcessLookupError:
            pass

    def cancel(signum: int, _frame: object) -> None:
        nonlocal interrupted_by_signal
        if interrupted_by_signal is not None:
            for index in range(len(commands)):
                signal_group(index, signal.SIGKILL)
            return
        interrupted_by_signal = signum
        now = time.monotonic()
        for index, process in enumerate(processes):
            # The group can still own descendants and captured pipes after its
            # original leader exits, so do not use process.poll() as custody.
            if process is not None and ended[index] == 0.0:
                interrupted[index] = signum
                signal_group(index, signal.SIGTERM)
                termination_deadlines[index] = now + TERMINATION_GRACE_SECONDS

    previous_handlers: dict[int, object] = {}
    readers = ThreadPoolExecutor(max_workers=len(commands))
    try:
        # Install the one signal owner before any child is launched.
        for signum in (signal.SIGINT, signal.SIGTERM):
            previous_handlers[signum] = signal.getsignal(signum)
            signal.signal(signum, cancel)
        futures = {}
        for index, command in enumerate(commands):
            if interrupted_by_signal is not None:
                break
            process = subprocess.Popen(
                command.argv,
                cwd=command.cwd,
                env=command.env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                start_new_session=True,
            )
            processes[index] = process
            started[index] = time.monotonic()
            deadlines[index] = started[index] + command.timeout_seconds
            futures[readers.submit(process.communicate)] = index
            if interrupted_by_signal is not None:
                interrupted[index] = interrupted_by_signal
                signal_group(index, signal.SIGTERM)
                termination_deadlines[index] = time.monotonic() + TERMINATION_GRACE_SECONDS

        pending = set(futures)
        while pending:
            now = time.monotonic()
            for future in pending:
                index = futures[future]
                if future.done():
                    continue
                if termination_deadlines[index] is not None:
                    if now >= termination_deadlines[index]:
                        signal_group(index, signal.SIGKILL)
                        termination_deadlines[index] = None
                elif interrupted_by_signal is None and now >= deadlines[index]:
                    timed_out[index] = True
                    signal_group(index, signal.SIGTERM)
                    termination_deadlines[index] = now + TERMINATION_GRACE_SECONDS
            completed, pending = wait(
                pending, timeout=POLL_INTERVAL_SECONDS, return_when=FIRST_COMPLETED
            )
            for future in completed:
                index = futures[future]
                outputs[index] = future.result()
                ended[index] = time.monotonic()
    except BaseException:
        for index in range(len(commands)):
            signal_group(index, signal.SIGTERM)
        for index in range(len(commands)):
            signal_group(index, signal.SIGKILL)
        raise
    finally:
        readers.shutdown(wait=True)
        for signum, handler in previous_handlers.items():
            signal.signal(signum, handler)

    results = []
    for index, process in enumerate(processes):
        stdout, stderr = outputs[index]
        results.append(
            SupervisedBatchResult(
                process=SupervisedProcess(
                    returncode=process.returncode if process is not None else None,
                    stdout=stdout,
                    stderr=stderr,
                    timed_out=timed_out[index],
                    interrupted_by_signal=interrupted[index]
                    if process is not None
                    else interrupted_by_signal,
                ),
                duration_s=ended[index] - started[index] if process is not None else 0.0,
            )
        )
    return results


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
