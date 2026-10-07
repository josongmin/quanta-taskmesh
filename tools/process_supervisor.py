"""Run a child process group with bounded timeout and capture cleanup.

Descendants that leave the owned process group (including a new session or
PGID) cannot be signaled through it. Their inherited capture pipes still have
a hard drain boundary, so they cannot hold a gate open indefinitely.
"""

from __future__ import annotations

import ctypes
import errno
import math
import os
import selectors
import signal
import subprocess
import sys
import tempfile
import threading
import time
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path
from typing import Callable

TERMINATION_GRACE_SECONDS = 5.0
POLL_INTERVAL_SECONDS = 0.2
POST_KILL_REAP_SECONDS = 2 * POLL_INTERVAL_SECONDS
ORPHANED_GROUP_DIAGNOSTIC = (
    "\nSUPERVISOR: leader exited with live process group; remaining members killed\n"
)
INACCESSIBLE_GROUP_DIAGNOSTIC = (
    "\nSUPERVISOR: leader exited with a process group that could not be signaled\n"
)
OPEN_CAPTURE_DIAGNOSTIC = (
    "\nSUPERVISOR: capture pipes remained open after the process-group deadline\n"
)
UNKNOWN_GROUP_DIAGNOSTIC = "\nSUPERVISOR: owned process-group custody could not be proved\n"


def _kill_owned_group(pgid: int) -> tuple[str, str]:
    """Signal the still-owned group while its original leader is unreaped."""
    try:
        os.killpg(pgid, signal.SIGKILL)
    except ProcessLookupError:
        return "absent", ""
    except PermissionError as error:
        # The leader started a new session, so its PID is the owned PGID.
        # Keep the forensic record numeric and leave the fail-closed verdict intact.
        return "inaccessible", (
            f"SUPERVISOR: group forensic leader_pid={pgid} pgid={pgid} errno={error.errno}\n"
        )
    return "killed", ""


class _SigVal(ctypes.Union):
    _fields_ = [("integer", ctypes.c_int), ("pointer", ctypes.c_void_p)]


class _SigInfo(ctypes.Structure):
    _fields_ = [
        ("signo", ctypes.c_int), ("error", ctypes.c_int),
        ("code", ctypes.c_int), ("pid", ctypes.c_int),
        ("uid", ctypes.c_uint), ("status", ctypes.c_int),
        ("address", ctypes.c_void_p), ("value", _SigVal),
        ("band", ctypes.c_long), ("padding", ctypes.c_ulong * 7),
    ]


@lru_cache(maxsize=1)
def _darwin_waitid():
    libc = ctypes.CDLL(None, use_errno=True)
    waitid = libc.waitid
    waitid.argtypes = [ctypes.c_int, ctypes.c_uint, ctypes.POINTER(_SigInfo), ctypes.c_int]
    waitid.restype = ctypes.c_int
    if ctypes.sizeof(_SigInfo) != 104:
        raise RuntimeError("unsupported Darwin siginfo_t ABI")
    return libc, waitid


def _leader_exited(process: subprocess.Popen[str] | subprocess.Popen[bytes]) -> bool:
    """Observe exit without releasing the PID which anchors the owned PGID."""
    if os.name != "posix":
        raise RuntimeError("process-group custody requires a POSIX platform")
    if sys.platform == "darwin":
        _libc, waitid = _darwin_waitid()
        info = _SigInfo()
        if waitid(os.P_PID, process.pid, ctypes.byref(info), os.WEXITED | os.WNOWAIT | os.WNOHANG):
            if ctypes.get_errno() == errno.EINTR:
                return False
            raise OSError(ctypes.get_errno(), "waitid(WNOWAIT) failed")
        if info.pid not in (0, process.pid):
            raise OSError("waitid returned a different process identity")
        return info.pid == process.pid
    if sys.platform == "linux":
        try:
            info = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOWAIT | os.WNOHANG)
        except InterruptedError:
            return False
        return info is not None and info.si_pid == process.pid
    raise RuntimeError("process-group custody is unsupported on this platform")


class _DarwinBsdInfo(ctypes.Structure):
    """MacOSX15.5 SDK proc_bsdinfo, including zombie status and start identity."""

    _fields_ = [
        (name, ctypes.c_uint32)
        for name in (
            "flags", "status", "xstatus", "pid", "ppid", "uid", "gid",
            "ruid", "rgid", "svuid", "svgid", "reserved",
        )
    ] + [
        ("comm", ctypes.c_char * 16),
        ("name", ctypes.c_char * 32),
    ] + [
        (name, ctypes.c_uint32)
        for name in ("nfiles", "pgid", "pjobc", "tdev", "tpgid")
    ] + [
        ("nice", ctypes.c_int32),
        ("start_sec", ctypes.c_uint64),
        ("start_usec", ctypes.c_uint64),
    ]


@lru_cache(maxsize=1)
def _darwin_libproc():
    libproc = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    listing = libproc.proc_listpgrppids
    listing.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_int]
    listing.restype = ctypes.c_int
    pidinfo = libproc.proc_pidinfo
    pidinfo.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_int]
    pidinfo.restype = ctypes.c_int
    if ctypes.sizeof(_DarwinBsdInfo) != 136:
        raise RuntimeError("unsupported Darwin proc_bsdinfo ABI")
    return libproc, listing, pidinfo


def _darwin_group_snapshot(pgid: int) -> dict[int, tuple[int, int, int]]:
    """Return the kernel's group members and their process identities.

    libproc returns a PID count. Its underlying proc_listpids maps errors to
    zero, so empty output cannot establish custody while the leader is held.
    """
    _libproc, listing, pidinfo = _darwin_libproc()
    estimate = listing(pgid, None, 0)
    if estimate <= 0 or estimate > 65536:
        raise OSError("process-group membership estimate unavailable")
    capacity = estimate + 20
    pids = (ctypes.c_int * capacity)()
    count = listing(pgid, pids, ctypes.sizeof(pids))
    if count <= 0 or count >= capacity:
        raise OSError("process-group membership unavailable or truncated")
    members = {}
    for pid in pids[:count]:
        if pid <= 0 or pid in members:
            raise OSError("invalid process-group membership")
        info = _DarwinBsdInfo()
        ctypes.set_errno(0)
        # arg=1 includes zombproc. The default arg=0 reports ESRCH for a
        # listed zombie and would lose its identity at retirement.
        returned = pidinfo(pid, 3, 1, ctypes.byref(info), ctypes.sizeof(info))
        if returned != ctypes.sizeof(info):
            raise OSError(ctypes.get_errno(), "process-group member identity unavailable")
        if info.pid != pid or info.pgid != pgid:
            raise OSError("process-group member identity changed")
        members[pid] = (info.start_sec * 1_000_000 + info.start_usec, info.pgid, info.status)
    return members


def _linux_group_snapshot(pgid: int) -> dict[int, tuple[int, int, str, int]]:
    members = {}
    with os.scandir("/proc") as entries:
        for entry in entries:
            if not entry.name.isdecimal():
                continue
            try:
                stat = (Path(entry.path) / "stat").read_bytes()
            except OSError as error:
                pid = int(entry.name)
                if error.errno not in (errno.ENOENT, errno.ESRCH) or pid == pgid:
                    raise
                # Procfs can retain a directory entry after an unrelated task
                # disappears. Classify that PID through the kernel before
                # omitting it; an unreadable owned member may still have live
                # worker threads and must leave custody unproved.
                try:
                    member_pgid = os.getpgid(pid)
                except ProcessLookupError:
                    continue
                if member_pgid != pgid:
                    continue
                raise
            tail = stat[stat.rfind(b")") + 2 :].split()
            if len(tail) < 20:
                raise OSError("process stat identity unavailable")
            member_pgid = int(tail[2])
            if member_pgid == pgid:
                thread_count = int(tail[17])
                if thread_count < 1:
                    raise OSError("process thread-group liveness unavailable")
                members[int(entry.name)] = (
                    int(tail[19]), member_pgid, tail[0].decode("ascii"), thread_count
                )
    return members


def _group_snapshot(pgid: int) -> dict[int, tuple]:
    if sys.platform == "darwin":
        return _darwin_group_snapshot(pgid)
    if sys.platform == "linux":
        return _linux_group_snapshot(pgid)
    raise RuntimeError("process-group custody is unsupported on this platform")


def _require_supported_custody() -> None:
    if sys.platform == "darwin":
        _darwin_waitid()
        _darwin_libproc()
    elif sys.platform == "linux" and hasattr(os, "waitid") and Path("/proc").is_dir():
        return
    else:
        raise RuntimeError("process-group custody is unsupported on this platform")


def _member_exited(identity: tuple) -> bool:
    state = identity[2]
    if isinstance(state, str):
        if len(identity) != 4 or type(identity[3]) is not int or identity[3] < 1:
            raise OSError("process thread-group liveness unavailable")
        # A Linux thread-group leader can be a zombie while its workers run.
        # The retained zombie is the group's only task after all workers exit.
        return state in ("Z", "X", "x") and identity[3] == 1
    return state == 5


def _stable_group_snapshot(pgid: int) -> dict[int, tuple]:
    first = _group_snapshot(pgid)
    second = _group_snapshot(pgid)
    # Scheduling state changes while an identified member remains in the group.
    # Stability binds membership, birth identity and PGID; liveness belongs to
    # the later observation, with the original exited leader held throughout.
    first_identities = {pid: identity[:2] for pid, identity in first.items()}
    second_identities = {pid: identity[:2] for pid, identity in second.items()}
    if pgid not in first or pgid not in second or first_identities != second_identities:
        raise OSError("process-group membership changed or leader absent")
    if (
        first[pgid][2] not in ("Z", 5)
        or second[pgid][2] not in ("Z", 5)
        or not _member_exited(first[pgid])
        or not _member_exited(second[pgid])
    ):
        raise OSError("held process-group leader is not exited")
    return second


def _has_live_member(pgid: int, members: dict[int, tuple]) -> bool:
    return any(
        pid != pgid and not _member_exited(identity)
        for pid, identity in members.items()
    )


def _await_killed_group_quiescence(pgid: int) -> tuple[bool, str]:
    """Bound cleanup after SIGKILL while the original leader remains waitable."""
    deadline = time.monotonic() + POST_KILL_REAP_SECONDS
    reason = "group still has live members"
    while True:
        try:
            if not _has_live_member(pgid, _stable_group_snapshot(pgid)):
                return True, ""
            reason = "group still has live members"
        except (OSError, ValueError) as error:
            reason = str(error)
        if time.monotonic() >= deadline:
            return False, f"SUPERVISOR: cleanup incomplete leader_pid={pgid}: {reason}\n"
        time.sleep(min(POLL_INTERVAL_SECONDS / 4, max(0, deadline - time.monotonic())))


def _retire_owned_group(pgid: int) -> tuple[str, str]:
    """Observe the original group, excluding descendants that left it."""
    try:
        members = _stable_group_snapshot(pgid)
    except (OSError, ValueError) as error:
        cleanup, forensic = _kill_owned_group(pgid)
        reason = f"SUPERVISOR: group custody unavailable leader_pid={pgid}: {error}\n"
        if cleanup == "killed":
            complete, detail = _await_killed_group_quiescence(pgid)
            if not complete:
                reason += detail
        return "unknown", reason + forensic
    if not _has_live_member(pgid, members):
        return "quiescent", ""
    cleanup, forensic = _kill_owned_group(pgid)
    if cleanup != "killed":
        return "inaccessible", forensic or f"SUPERVISOR: live group vanished pgid={pgid}\n"
    complete, detail = _await_killed_group_quiescence(pgid)
    if not complete:
        return "unknown", detail
    return "killed", ""


def _close_capture_pipes(process: subprocess.Popen[str]) -> None:
    for pipe in (process.stdout, process.stderr):
        if pipe is not None and not pipe.closed:
            pipe.close()


def _decode_text(
    value: bytes, process: subprocess.Popen[str], stream: str, *, partial: bool = False
) -> str:
    pipe = process.stdout if stream == "stdout" else process.stderr
    assert pipe is not None
    decoded = value.decode(pipe.encoding, errors="replace" if partial else pipe.errors or "strict")
    return decoded.replace("\r\n", "\n").replace("\r", "\n")


def _drain_capture(process: subprocess.Popen[str], stop: threading.Event) -> tuple[str, str, bool]:
    """Drain pipes without calling poll/communicate/wait on the held leader."""
    selector = selectors.DefaultSelector()
    captured = {"stdout": bytearray(), "stderr": bytearray()}
    try:
        for name, pipe in (("stdout", process.stdout), ("stderr", process.stderr)):
            assert pipe is not None
            os.set_blocking(pipe.fileno(), False)
            selector.register(pipe, selectors.EVENT_READ, name)
        while selector.get_map():
            if stop.is_set():
                return (
                    _decode_text(bytes(captured["stdout"]), process, "stdout", partial=True),
                    _decode_text(bytes(captured["stderr"]), process, "stderr", partial=True),
                    True,
                )
            for key, _events in selector.select(POLL_INTERVAL_SECONDS):
                chunk = os.read(key.fileobj.fileno(), 65536)
                if chunk:
                    captured[key.data].extend(chunk)
                else:
                    selector.unregister(key.fileobj)
        return (
            _decode_text(bytes(captured["stdout"]), process, "stdout"),
            _decode_text(bytes(captured["stderr"]), process, "stderr"),
            False,
        )
    finally:
        selector.close()
        _close_capture_pipes(process)


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
class StartedProcess:
    """Payload identity without a wait/poll capability over the held leader."""

    pid: int


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
        raise ValueError("timeout_seconds must be positive")
    if not commands:
        return []
    _require_supported_custody()

    processes: list[subprocess.Popen[str] | None] = [None] * len(commands)
    started = [0.0] * len(commands)
    ended = [0.0] * len(commands)
    deadlines = [0.0] * len(commands)
    termination_deadlines: list[float | None] = [None] * len(commands)
    hard_stop_deadlines: list[float | None] = [None] * len(commands)
    timed_out = [False] * len(commands)
    interrupted: list[int | None] = [None] * len(commands)
    outputs: list[tuple[str, str]] = [("", "")] * len(commands)
    orphaned_groups = [False] * len(commands)
    group_retired = [False] * len(commands)
    capture_stops = [threading.Event() for _ in commands]
    interrupted_by_signal: int | None = None

    def signal_group(index: int, signum: int) -> None:
        process = processes[index]
        if process is None or group_retired[index]:
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
            if process is not None and not group_retired[index]:
                interrupted[index] = signum
                signal_group(index, signal.SIGTERM)
                termination_deadlines[index] = now + TERMINATION_GRACE_SECONDS

    previous_handlers: dict[int, object] = {}
    readers = ThreadPoolExecutor(max_workers=len(commands))
    failed = False
    try:
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
            futures[index] = readers.submit(_drain_capture, process, capture_stops[index])
            if interrupted_by_signal is not None:
                interrupted[index] = interrupted_by_signal
                signal_group(index, signal.SIGTERM)
                termination_deadlines[index] = time.monotonic() + TERMINATION_GRACE_SECONDS

        pending = set(futures)
        captures_done = set()
        while pending:
            now = time.monotonic()
            for index in list(pending):
                process = processes[index]
                assert process is not None
                future = futures[index]
                if future.done() and index not in captures_done:
                    stdout, stderr, capture_aborted = future.result()
                    if capture_aborted:
                        stderr += OPEN_CAPTURE_DIAGNOSTIC
                        if interrupted[index] is None:
                            timed_out[index] = True
                    outputs[index] = stdout, stderr
                    captures_done.add(index)
                if termination_deadlines[index] is not None:
                    if now >= termination_deadlines[index]:
                        signal_group(index, signal.SIGKILL)
                        capture_stops[index].set()
                        termination_deadlines[index] = None
                        hard_stop_deadlines[index] = now + POST_KILL_REAP_SECONDS
                elif (
                    interrupted_by_signal is None
                    and not timed_out[index]
                    and now >= deadlines[index]
                ):
                    timed_out[index] = True
                    signal_group(index, signal.SIGTERM)
                    termination_deadlines[index] = now + TERMINATION_GRACE_SECONDS
                leader_done = _leader_exited(process)
                if index in captures_done and leader_done:
                    cleanup, forensic = _retire_owned_group(process.pid)
                    group_retired[index] = True
                    process.wait()
                    if cleanup != "quiescent":
                        stdout, stderr = outputs[index]
                        stderr += (
                            ORPHANED_GROUP_DIAGNOSTIC
                            if cleanup == "killed"
                            else UNKNOWN_GROUP_DIAGNOSTIC + forensic
                            if cleanup == "unknown"
                            else INACCESSIBLE_GROUP_DIAGNOSTIC + forensic
                        )
                        outputs[index] = stdout, stderr
                        orphaned_groups[index] = True
                    ended[index] = time.monotonic()
                    pending.remove(index)
                elif hard_stop_deadlines[index] is not None and now >= hard_stop_deadlines[index]:
                    raise TimeoutError("owned leader did not exit after group SIGKILL")
            if pending:
                unfinished = {futures[index] for index in pending if index not in captures_done}
                if unfinished:
                    wait(unfinished, timeout=POLL_INTERVAL_SECONDS, return_when=FIRST_COMPLETED)
                else:
                    time.sleep(POLL_INTERVAL_SECONDS)
    except BaseException:
        failed = True
        for stop in capture_stops:
            stop.set()
        for index in range(len(commands)):
            signal_group(index, signal.SIGTERM)
            signal_group(index, signal.SIGKILL)
            group_retired[index] = True
        raise
    finally:
        try:
            readers.shutdown(wait=True)
            if failed:
                for process in processes:
                    if process is not None:
                        _close_capture_pipes(process)
                        try:
                            process.wait(timeout=POLL_INTERVAL_SECONDS)
                        except subprocess.TimeoutExpired:
                            pass
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
                    aborted_early=orphaned_groups[index],
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
    on_started: Callable[[StartedProcess], None] | None = None,
    max_capture_bytes: int | None = None,
    stdin_data: bytes | None = None,
    binary_output: bool = False,
) -> SupervisedProcess | SupervisedBinaryProcess:
    """Settle the original group before returning an execution result.

    Exceptional paths send bounded group cleanup while the leader is held and
    propagate the error. They do not claim completed worker custody.
    """
    if (
        type(timeout_seconds) not in (int, float)
        or not math.isfinite(timeout_seconds)
        or timeout_seconds <= 0
    ):
        raise ValueError("timeout_seconds must be positive")
    if termination_grace_seconds is None:
        termination_grace_seconds = TERMINATION_GRACE_SECONDS
    if (
        type(termination_grace_seconds) not in (int, float)
        or not math.isfinite(termination_grace_seconds)
        or termination_grace_seconds <= 0
    ):
        raise ValueError("termination_grace_seconds must be positive")
    if threading.current_thread() is not threading.main_thread():
        raise RuntimeError("supervised processes must be launched from the main thread")
    _require_supported_custody()

    if max_capture_bytes is not None and (
        type(max_capture_bytes) is not int or max_capture_bytes <= 0
    ):
        raise ValueError("max_capture_bytes must be a positive integer")
    if stdin_data is not None and not isinstance(stdin_data, bytes):
        raise ValueError("stdin_data must be bytes")
    input_file = tempfile.TemporaryFile() if stdin_data is not None else None
    if input_file is not None:
        input_file.write(stdin_data)
        input_file.seek(0)
    capture_selector = selectors.DefaultSelector()
    captured = {"stdout": bytearray(), "stderr": bytearray()}
    capture_overflow = False
    process: subprocess.Popen[str] | None = None
    group_retired = False
    timed_out = False
    aborted_early = False
    interrupted_by_signal: int | None = None
    termination_deadline: float | None = None
    hard_stop = False
    hard_stop_deadline: float | None = None
    capture_aborted = False

    def signal_group(signum: int) -> None:
        if process is None or group_retired:
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
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            stdin=input_file,
            text=not binary_output,
            start_new_session=True,
        )
        for name, pipe in (("stdout", process.stdout), ("stderr", process.stderr)):
            assert pipe is not None
            os.set_blocking(pipe.fileno(), False)
            capture_selector.register(pipe, selectors.EVENT_READ, name)
        if on_started is not None:
            on_started(StartedProcess(process.pid))
        timeout_deadline = time.monotonic() + timeout_seconds
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
                hard_stop_deadline = now + POST_KILL_REAP_SECONDS
            elif interrupted_by_signal is not None and termination_deadline is not None:
                if now >= termination_deadline:
                    signal_group(signal.SIGKILL)
                    termination_deadline = None
                    hard_stop = True
                    hard_stop_deadline = now + POST_KILL_REAP_SECONDS
            elif not timed_out and interrupted_by_signal is None and now >= timeout_deadline:
                timed_out = True
                signal_group(signal.SIGTERM)
                termination_deadline = now + termination_grace_seconds
            elif (
                not timed_out
                and not aborted_early
                and interrupted_by_signal is None
                and (capture_overflow or (abort_when is not None and abort_when()))
            ):
                aborted_early = True
                signal_group(signal.SIGTERM)
                termination_deadline = now + termination_grace_seconds

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
            wait_seconds = min(POLL_INTERVAL_SECONDS, until_deadline)
            if capture_selector.get_map():
                for key, _events in capture_selector.select(wait_seconds):
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        capture_selector.unregister(key.fileobj)
                        continue
                    if max_capture_bytes is None:
                        captured[key.data].extend(chunk)
                    else:
                        remaining = max_capture_bytes - sum(
                            len(value) for value in captured.values()
                        )
                        captured[key.data].extend(chunk[: max(0, remaining)])
                        if len(chunk) > remaining:
                            capture_overflow = True
            else:
                time.sleep(wait_seconds)
            leader_done = _leader_exited(process)
            if leader_done and not capture_selector.get_map():
                break
            if hard_stop and capture_selector.get_map():
                capture_aborted = True
                captured["stderr"].extend(OPEN_CAPTURE_DIAGNOSTIC.encode())
                for key in list(capture_selector.get_map().values()):
                    capture_selector.unregister(key.fileobj)
                _close_capture_pipes(process)
            if hard_stop and not leader_done and termination_deadline is None:
                # SIGKILL was sent to the anchored group. Do not wait forever
                # for a kernel-stuck child or an escaped capture writer.
                if hard_stop_deadline is not None and time.monotonic() >= hard_stop_deadline:
                    raise TimeoutError("owned leader did not exit after group SIGKILL")
        stdout = bytes(captured["stdout"])
        stderr = bytes(captured["stderr"])
        if not binary_output:
            if max_capture_bytes is not None:
                stdout = stdout.decode(errors="replace")
                stderr = stderr.decode(errors="replace")
            else:
                stdout = _decode_text(stdout, process, "stdout", partial=capture_aborted)
                stderr = _decode_text(stderr, process, "stderr", partial=capture_aborted)
        if capture_overflow:
            diagnostic = "\nSUPERVISOR: capture byte limit exceeded\n"
            stderr += diagnostic.encode() if binary_output else diagnostic
            aborted_early = True
        cleanup, forensic = _retire_owned_group(process.pid)
        group_retired = True
        process.wait()
        if cleanup != "quiescent":
            diagnostic = (
                ORPHANED_GROUP_DIAGNOSTIC
                if cleanup == "killed"
                else UNKNOWN_GROUP_DIAGNOSTIC + forensic
                if cleanup == "unknown"
                else INACCESSIBLE_GROUP_DIAGNOSTIC + forensic
            )
            stderr += diagnostic.encode() if binary_output else diagnostic
            # An owned descendant required forced cleanup: the command did
            # not complete, even if its leader returned a domain result code.
            orphaned_group = True
            aborted_early = True
    except BaseException:
        if process is not None and not group_retired:
            signal_group(signal.SIGTERM)
            signal_group(signal.SIGKILL)
            group_retired = True
            _close_capture_pipes(process)
            try:
                process.wait(timeout=POLL_INTERVAL_SECONDS)
            except subprocess.TimeoutExpired:
                pass
        raise
    finally:
        if input_file is not None:
            input_file.close()
        capture_selector.close()
        if process is not None:
            _close_capture_pipes(process)
        for signum, handler in previous_handlers.items():
            signal.signal(signum, handler)

    assert process is not None
    result_type = SupervisedBinaryProcess if binary_output else SupervisedProcess
    empty = b"" if binary_output else ""
    return result_type(
        returncode=None if orphaned_group else process.returncode,
        stdout=stdout or empty,
        stderr=stderr or empty,
        timed_out=timed_out,
        interrupted_by_signal=interrupted_by_signal,
        aborted_early=aborted_early,
    )
