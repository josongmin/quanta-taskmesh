"""Run a child process group with bounded timeout and capture cleanup.

Descendants that create a new session leave the owned group and cannot be
signaled through it. Their inherited capture pipes still have a hard drain
boundary, so an escaped descendant cannot hold a gate open indefinitely.
"""

from __future__ import annotations

import math
import os
import signal
import subprocess
import threading
import time
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

TERMINATION_GRACE_SECONDS = 5.0
POLL_INTERVAL_SECONDS = 0.2
ORPHANED_GROUP_DIAGNOSTIC = (
    "\nSUPERVISOR: leader exited with live process group; remaining members killed\n"
)
INACCESSIBLE_GROUP_DIAGNOSTIC = (
    "\nSUPERVISOR: leader exited with a process group that could not be signaled\n"
)
OPEN_CAPTURE_DIAGNOSTIC = (
    "\nSUPERVISOR: capture pipes remained open after the process-group deadline\n"
)


def _kill_remaining_group(pgid: int) -> str:
    """Close an owned group after its leader has been reaped.

    A grandchild can close the captured pipes, outlive a successful leader,
    and otherwise turn an incomplete command into a false PASS. Killing the
    group also handles descendants that were not connected to those pipes.
    """
    try:
        os.killpg(pgid, signal.SIGKILL)
    except ProcessLookupError:
        return "absent"
    except PermissionError:
        return "inaccessible"
    return "killed"


def _partial_text(value: str | bytes | None) -> str:
    if value is None:
        return ""
    return value.decode(errors="replace") if isinstance(value, bytes) else value


def _close_capture_pipes(process: subprocess.Popen[str]) -> None:
    for pipe in (process.stdout, process.stderr):
        if pipe is not None and not pipe.closed:
            pipe.close()


def _finish_leader_after_pipe_abort(process: subprocess.Popen[str]) -> None:
    _close_capture_pipes(process)
    try:
        process.wait(timeout=POLL_INTERVAL_SECONDS)
    except subprocess.TimeoutExpired:
        try:
            process.kill()
        except (ProcessLookupError, PermissionError):
            pass
        try:
            process.wait(timeout=POLL_INTERVAL_SECONDS)
        except subprocess.TimeoutExpired:
            pass


def _drain_capture(process: subprocess.Popen[str], stop: threading.Event) -> tuple[str, str, bool]:
    """Drain in a reader thread until completion or the owner's hard stop."""
    while True:
        try:
            stdout, stderr = process.communicate(timeout=POLL_INTERVAL_SECONDS)
            return stdout or "", stderr or "", False
        except subprocess.TimeoutExpired as exc:
            if not stop.is_set():
                continue
            stdout, stderr = _partial_text(exc.stdout), _partial_text(exc.stderr)
            _finish_leader_after_pipe_abort(process)
            return stdout, stderr, True


@dataclass(frozen=True)
class SupervisedProcess:
    returncode: int | None
    stdout: str
    stderr: str
    timed_out: bool
    interrupted_by_signal: int | None
    aborted_early: bool = False


@dataclass(frozen=True)
class SupervisedBinaryProcess:
    returncode: int | None
    stdout: bytes
    stderr: bytes
    timed_out: bool
    interrupted_by_signal: int | None
    aborted_early: bool = False


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
    if any(
        type(command.timeout_seconds) not in (int, float)
        or not math.isfinite(command.timeout_seconds)
        or command.timeout_seconds <= 0
        for command in commands
    ):
        raise ValueError("timeout_seconds must be finite and positive")
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
    orphaned_groups = [False] * len(commands)
    capture_stops = [threading.Event() for _ in commands]
    interrupted_by_signal: int | None = None

    def signal_group(index: int, signum: int) -> None:
        process = processes[index]
        if process is None:
            return
        try:
            os.killpg(process.pid, signum)
        except (ProcessLookupError, PermissionError):
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
    failed = False
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
            futures[readers.submit(_drain_capture, process, capture_stops[index])] = index
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
                        capture_stops[index].set()
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
                stdout, stderr, capture_aborted = future.result()
                process = processes[index]
                assert process is not None
                if capture_aborted:
                    stderr += OPEN_CAPTURE_DIAGNOSTIC
                    if interrupted[index] is None:
                        timed_out[index] = True
                cleanup = _kill_remaining_group(process.pid)
                if cleanup != "absent":
                    stderr += (
                        ORPHANED_GROUP_DIAGNOSTIC
                        if cleanup == "killed"
                        else INACCESSIBLE_GROUP_DIAGNOSTIC
                    )
                    orphaned_groups[index] = (
                        process.returncode == 0
                        and not timed_out[index]
                        and interrupted[index] is None
                    )
                outputs[index] = stdout, stderr
                ended[index] = time.monotonic()
    except BaseException:
        failed = True
        for stop in capture_stops:
            stop.set()
        for index in range(len(commands)):
            signal_group(index, signal.SIGTERM)
        for index in range(len(commands)):
            signal_group(index, signal.SIGKILL)
        raise
    finally:
        try:
            readers.shutdown(wait=True)
            if failed:
                # A failed reader did not necessarily call communicate() or
                # wait() on its leader. Reap it after all readers have stopped
                # so no thread still owns the same capture streams.
                for process in processes:
                    if process is not None:
                        _finish_leader_after_pipe_abort(process)
                        _kill_remaining_group(process.pid)
        finally:
            for signum, handler in previous_handlers.items():
                signal.signal(signum, handler)

    results = []
    for index, process in enumerate(processes):
        stdout, stderr = outputs[index]
        results.append(
            SupervisedBatchResult(
                process=SupervisedProcess(
                    returncode=(
                        None if orphaned_groups[index] or process is None else process.returncode
                    ),
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
    termination_grace_seconds: float | None = None,
    abort_when: Callable[[], bool] | None = None,
    on_started: Callable[[int], None] | None = None,
    input_text: str | None = None,
    binary_output: bool = False,
) -> SupervisedProcess | SupervisedBinaryProcess:
    """Capture complete output and reap the owned process group on every exit path."""
    if (
        type(timeout_seconds) not in (int, float)
        or not math.isfinite(timeout_seconds)
        or timeout_seconds <= 0
    ):
        raise ValueError("timeout_seconds must be finite and positive")
    if termination_grace_seconds is None:
        termination_grace_seconds = TERMINATION_GRACE_SECONDS
    if (
        type(termination_grace_seconds) not in (int, float)
        or not math.isfinite(termination_grace_seconds)
        or termination_grace_seconds <= 0
    ):
        raise ValueError("termination_grace_seconds must be finite and positive")
    if threading.current_thread() is not threading.main_thread():
        raise RuntimeError("supervised processes must be launched from the main thread")

    process: subprocess.Popen[str] | None = None
    input_pipe = None
    input_bytes = b""
    input_offset = 0
    timed_out = False
    aborted_early = False
    interrupted_by_signal: int | None = None
    termination_deadline: float | None = None
    hard_stop = False

    def signal_group(signum: int) -> None:
        if process is None:
            return
        try:
            os.killpg(process.pid, signum)
        except (ProcessLookupError, PermissionError):
            pass

    def cancel(signum: int, _frame: object) -> None:
        nonlocal interrupted_by_signal, termination_deadline
        if interrupted_by_signal is not None:
            signal_group(signal.SIGKILL)
            return
        interrupted_by_signal = signum
        signal_group(signal.SIGTERM)
        termination_deadline = time.monotonic() + termination_grace_seconds

    previous_handlers: dict[int, object] = {}
    stdout = stderr = b"" if binary_output else ""
    orphaned_group = False
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
            stdin=subprocess.PIPE if input_text is not None else None,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=not binary_output,
            start_new_session=True,
        )
        timeout_deadline = time.monotonic() + timeout_seconds
        if on_started is not None:
            on_started(process.pid)
        if process.stdin is not None:
            # communicate() cannot resume backpressured input with input=None:
            # CPython no longer registers stdin after the first timed poll.
            # Own a nonblocking writer separately from captured pipe draining.
            input_pipe = process.stdin
            input_bytes = input_text.encode("utf-8" if binary_output else input_pipe.encoding)
            os.set_blocking(input_pipe.fileno(), False)
            process.stdin = None
        if interrupted_by_signal is not None:
            signal_group(signal.SIGTERM)
            if termination_deadline is None:
                termination_deadline = time.monotonic() + termination_grace_seconds
        while True:
            now = time.monotonic()
            if (
                (timed_out or aborted_early)
                and termination_deadline is not None
                and now >= termination_deadline
            ):
                signal_group(signal.SIGKILL)
                termination_deadline = None
                hard_stop = True
            elif interrupted_by_signal is not None and termination_deadline is not None:
                if now >= termination_deadline:
                    signal_group(signal.SIGKILL)
                    termination_deadline = None
                    hard_stop = True
            elif not timed_out and interrupted_by_signal is None and now >= timeout_deadline:
                timed_out = True
                signal_group(signal.SIGTERM)
                termination_deadline = now + termination_grace_seconds
            elif (
                not timed_out
                and not aborted_early
                and interrupted_by_signal is None
                and abort_when is not None
                and abort_when()
            ):
                aborted_early = True
                signal_group(signal.SIGTERM)
                termination_deadline = now + termination_grace_seconds

            if input_pipe is not None:
                if timed_out or aborted_early or interrupted_by_signal is not None:
                    input_pipe.close()
                    input_pipe = None
                else:
                    while input_offset < len(input_bytes):
                        try:
                            count = os.write(
                                input_pipe.fileno(),
                                input_bytes[input_offset : input_offset + 65536],
                            )
                        except BlockingIOError:
                            break
                        except BrokenPipeError:
                            input_offset = len(input_bytes)
                            break
                        input_offset += count
                    if input_offset == len(input_bytes):
                        input_pipe.close()
                        input_pipe = None

            deadlines = (
                [timeout_deadline]
                if not timed_out and not aborted_early and interrupted_by_signal is None
                else []
            )
            if termination_deadline is not None:
                deadlines.append(termination_deadline)
            until_deadline = (
                max(0.001, min(deadlines) - time.monotonic())
                if deadlines
                else POLL_INTERVAL_SECONDS
            )
            wait_seconds = min(
                0.001 if input_pipe is not None else POLL_INTERVAL_SECONDS, until_deadline
            )
            try:
                stdout, stderr = process.communicate(timeout=wait_seconds)
                break
            except subprocess.TimeoutExpired as exc:
                if hard_stop:
                    if binary_output:
                        stdout, stderr = exc.stdout or b"", exc.stderr or b""
                        stderr += OPEN_CAPTURE_DIAGNOSTIC.encode()
                    else:
                        stdout, stderr = _partial_text(exc.stdout), _partial_text(exc.stderr)
                        stderr += OPEN_CAPTURE_DIAGNOSTIC
                    _finish_leader_after_pipe_abort(process)
                    break
                continue
        cleanup = _kill_remaining_group(process.pid)
        if cleanup != "absent":
            diagnostic = (
                ORPHANED_GROUP_DIAGNOSTIC if cleanup == "killed" else INACCESSIBLE_GROUP_DIAGNOSTIC
            )
            stderr += diagnostic.encode() if binary_output else diagnostic
            orphaned_group = (
                process.returncode == 0
                and not timed_out
                and not aborted_early
                and interrupted_by_signal is None
            )
    except BaseException:
        if process is not None:
            signal_group(signal.SIGTERM)
            # Capture itself may have failed while a descendant outside the
            # owned process group still holds these pipes. Never retry an
            # unbounded communicate on the exceptional path.
            _close_capture_pipes(process)
            try:
                process.wait(timeout=termination_grace_seconds)
            except subprocess.TimeoutExpired:
                signal_group(signal.SIGKILL)
                _finish_leader_after_pipe_abort(process)
            finally:
                # A child can close our pipes and outlive the leader even on
                # an exceptional path where the leader has already exited.
                _kill_remaining_group(process.pid)
        raise
    finally:
        if input_pipe is not None:
            input_pipe.close()
        for signum, handler in previous_handlers.items():
            signal.signal(signum, handler)

    assert process is not None
    result_type = SupervisedBinaryProcess if binary_output else SupervisedProcess
    return result_type(
        returncode=None if orphaned_group else process.returncode,
        stdout=stdout,
        stderr=stderr,
        timed_out=timed_out,
        interrupted_by_signal=interrupted_by_signal,
        aborted_early=aborted_early,
    )
