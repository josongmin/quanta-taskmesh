"""Bounded acquisition execution and cooperative descendant custody.

Nested acquisition helpers inherit a private owner registry. Every helper
registers its launched process group and parent call, so cleanup reaches owned
nested sessions even after their collectors exit or close capture pipes.
Deliberately stripping the owner environment, spawning before registration or
using an uncooperative launcher to escape the group is outside this contract.
"""

from __future__ import annotations

import json
import math
import os
import signal
import stat
import subprocess
import sys
import tempfile
import uuid
from dataclasses import replace
from pathlib import Path
from typing import Callable

import psutil

REPO = Path(__file__).resolve().parents[2]
if str(REPO) not in sys.path:
    sys.path.insert(0, str(REPO))
from tools.process_supervisor import SupervisedProcess, run_process  # noqa: E402

OWNER_DIRECTORY = "TASKMESH_ACQUISITION_OWNER_DIRECTORY"
OWNER_CALL = "TASKMESH_ACQUISITION_OWNER_CALL"
MAX_CAPTURE_BYTES = 8 * 1024 * 1024
MAX_OWNER_RECORD_BYTES = 4096
TERMINATION_GRACE_SECONDS = 0.5


def deadline(value: float) -> float:
    if type(value) not in (int, float) or not math.isfinite(value) or value <= 0:
        raise ValueError("timeout_seconds must be positive and finite")
    return float(value)


def execution_record(result: SupervisedProcess, timeout_seconds: float) -> dict:
    return {
        "timeout_seconds": deadline(timeout_seconds),
        "returncode": result.returncode,
        "timed_out": result.timed_out,
        "interrupted_by_signal": result.interrupted_by_signal,
        "aborted_early": result.aborted_early,
    }


def _registry(directory: Path) -> None:
    metadata = directory.stat()
    if (
        directory.is_symlink()
        or not stat.S_ISDIR(metadata.st_mode)
        or metadata.st_uid != os.getuid()
        or metadata.st_mode & 0o077
    ):
        raise ValueError("acquisition owner registry must be a private owned directory")


def _call_identity(value: object) -> bool:
    return (
        isinstance(value, str)
        and len(value) == 32
        and all(character in "0123456789abcdef" for character in value)
    )


def _owner_record(path: Path) -> dict:
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as stream:
        metadata = os.fstat(stream.fileno())
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.getuid()
            or metadata.st_size > MAX_OWNER_RECORD_BYTES
        ):
            raise ValueError("owner record must be a bounded regular owned file")
        payload = stream.read(MAX_OWNER_RECORD_BYTES + 1)
        if len(payload) > MAX_OWNER_RECORD_BYTES:
            raise ValueError("owner record byte limit exceeded")
    record = json.loads(payload)
    if not isinstance(record, dict) or set(record) != {"call", "parent", "pid", "created"}:
        raise ValueError("invalid owner record")
    if not _call_identity(record["call"]) or record["call"] != path.stem:
        raise ValueError("owner call identity differs")
    if record["parent"] is not None and not _call_identity(record["parent"]):
        raise ValueError("invalid owner parent identity")
    if type(record["pid"]) is not int or record["pid"] <= 0:
        raise ValueError("invalid owner process identity")
    if (
        type(record["created"]) not in (int, float)
        or not math.isfinite(record["created"])
        or record["created"] <= 0
    ):
        raise ValueError("invalid owner process creation time")
    return record


def _cleanup(directory: Path, call: str, *, root_owner: bool = False) -> tuple[bool, list[str]]:
    records = {}
    diagnostics = []
    for path in directory.glob("*.json"):
        try:
            records[path.stem] = _owner_record(path)
        except (OSError, ValueError) as error:
            diagnostics.append(f"owner registry unavailable: {error}")
    owned = {call}
    while True:
        children = {key for key, record in records.items() if record["parent"] in owned}
        if children.issubset(owned):
            break
        owned.update(children)
    live = False
    for key in sorted(owned):
        record = records.get(key)
        if record is None:
            continue
        try:
            process = psutil.Process(record["pid"])
            if (
                process.create_time() != record["created"]
                or process.status() == psutil.STATUS_ZOMBIE
            ):
                continue
            pgid = os.getpgid(process.pid)
            if process.pid == os.getpid() or pgid == os.getpgrp() or pgid != process.pid:
                diagnostics.append("owner record does not identify an isolated child group")
                continue
            live = True
            try:
                os.killpg(pgid, signal.SIGKILL)
            except ProcessLookupError:
                continue
            diagnostics.append(f"owned live group killed: pid={process.pid} call={key}")
        except psutil.NoSuchProcess:
            continue
        except (psutil.Error, OSError, ValueError) as error:
            diagnostics.append(f"owner cleanup failed: {error}")
    # A registered leader may already have exited while group members survive.
    # Inherited owner environment identifies cooperative descendants independently
    # of their session or current parent, and avoids signaling a reused PID/group.
    signaled = set()
    for process in psutil.process_iter(["pid", "create_time"]):
        try:
            environment = process.environ()
            if (
                environment.get(OWNER_DIRECTORY) != str(directory)
                or (not root_owner and environment.get(OWNER_CALL) not in owned)
                or process.status() == psutil.STATUS_ZOMBIE
            ):
                continue
            pgid = os.getpgid(process.pid)
            if pgid == os.getpgrp():
                diagnostics.append("owned descendant joined the controller process group")
                continue
            if pgid in signaled:
                continue
            live = True
            os.killpg(pgid, signal.SIGKILL)
            signaled.add(pgid)
            diagnostics.append(f"owned descendant group killed: pgid={pgid}")
        except (psutil.NoSuchProcess, ProcessLookupError):
            continue
        except (psutil.AccessDenied, PermissionError):
            # Uninspectable unrelated processes do not belong to this private
            # ownership contract. Registered groups are checked separately above.
            continue
    return live or bool(diagnostics), diagnostics


def run_acquisition(
    command: list[str],
    *,
    cwd: Path,
    timeout_seconds: float = 1800,
    env: dict[str, str] | None = None,
    on_started: Callable[[subprocess.Popen[str]], None] | None = None,
    stdin_data: bytes | None = None,
) -> SupervisedProcess:
    timeout_seconds = deadline(timeout_seconds)
    environment = dict(os.environ if env is None else env)
    inherited = os.environ.get(OWNER_DIRECTORY)
    supplied = environment.get(OWNER_DIRECTORY)
    if inherited and supplied and inherited != supplied:
        raise ValueError("nested acquisition cannot replace its owner registry")
    owner = inherited or supplied
    temporary = (
        tempfile.TemporaryDirectory(prefix="taskmesh-acquisition-") if owner is None else None
    )
    directory = Path(temporary.name if temporary is not None else owner).resolve()
    _registry(directory)
    parent = os.environ.get(OWNER_CALL) or environment.get(OWNER_CALL)
    call = uuid.uuid4().hex
    environment[OWNER_DIRECTORY] = str(directory)
    environment[OWNER_CALL] = call

    def started(process: subprocess.Popen[str]) -> None:
        created = psutil.Process(process.pid).create_time()
        record = {"call": call, "parent": parent, "pid": process.pid, "created": created}
        target = directory / (call + ".json")
        staged = directory / (call + ".tmp")
        staged.write_text(json.dumps(record))
        staged.replace(target)
        if on_started is not None:
            on_started(process)

    result = None
    try:
        result = run_process(
            command,
            cwd=cwd,
            env=environment,
            timeout_seconds=timeout_seconds,
            termination_grace_seconds=TERMINATION_GRACE_SECONDS,
            max_capture_bytes=MAX_CAPTURE_BYTES,
            on_started=started,
            stdin_data=stdin_data,
        )
    finally:
        incomplete, diagnostics = _cleanup(directory, call, root_owner=temporary is not None)
        if temporary is not None:
            temporary.cleanup()
    assert result is not None
    if incomplete:
        result = replace(
            result,
            returncode=None if result.returncode == 0 else result.returncode,
            aborted_early=True,
            stderr=result.stderr + "\nACQUISITION: " + "; ".join(diagnostics) + "\n",
        )
    return result
