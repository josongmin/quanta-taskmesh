# ruff: noqa: UP045 -- pyproject supports Python 3.9, which lacks PEP 604 unions.
"""Bounded, low-frequency samples of the measured child process.

Sampling runs in the controller, outside Taskmesh's request path. These are
observations, not exact RSS or thread high-water marks.
"""

from __future__ import annotations

import threading
import time
from pathlib import Path
from typing import Any, Optional

import psutil
from acquisition_process import execution_record, run_acquisition


class ProcessResourceSampler:
    def __init__(self, pid: int, cadence_ms: int = 250, max_samples: int = 20_000) -> None:
        if pid <= 0 or cadence_ms <= 0 or max_samples <= 0:
            raise ValueError("pid, cadence and sample cap must be positive")
        self.pid = pid
        self.cadence_ms = cadence_ms
        self.max_samples = max_samples
        self.samples: list[dict[str, int]] = []
        self.reason: Optional[str] = None
        self.create_time_ns: Optional[int] = None
        self.started_ns = time.monotonic_ns()
        self.started_epoch_ns = time.time_ns()
        self.boot_time_ns = round(psutil.boot_time() * 1_000_000_000)
        self.ended_ns: Optional[int] = None
        self.ended_epoch_ns: Optional[int] = None
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._collect, daemon=True)

    def start(self) -> None:
        self._thread.start()

    def stop(self) -> dict[str, Any]:
        self._stop.set()
        self._thread.join(timeout=5)
        self.ended_ns = time.monotonic_ns()
        self.ended_epoch_ns = time.time_ns()
        if self._thread.is_alive():
            self.reason = "sampler thread did not stop"
        if not self.samples and self.reason is None:
            self.reason = "process ended before first sample"
        return {
            "schema_version": 2,
            "status": "complete" if self.reason is None else "unavailable",
            "reason": self.reason,
            "pid": self.pid,
            "process_create_time_ns": self.create_time_ns,
            "boot_time_ns": self.boot_time_ns,
            "cadence_ms": self.cadence_ms,
            "sampling_started_monotonic_ns": self.started_ns,
            "sampling_ended_monotonic_ns": self.ended_ns,
            "sampling_started_epoch_ns": self.started_epoch_ns,
            "sampling_ended_epoch_ns": self.ended_epoch_ns,
            "samples": self.samples,
        }

    def _collect(self) -> None:
        try:
            process = psutil.Process(self.pid)
            self.create_time_ns = round(process.create_time() * 1_000_000_000)
            while True:
                if len(self.samples) >= self.max_samples:
                    self.reason = "sample cap reached"
                    return
                with process.oneshot():
                    cpu = process.cpu_times()
                    memory = process.memory_info()
                    threads = process.num_threads()
                self.samples.append(
                    {
                        "monotonic_ns": time.monotonic_ns(),
                        "cpu_user_ns": round(cpu.user * 1_000_000_000),
                        "cpu_system_ns": round(cpu.system * 1_000_000_000),
                        "rss_bytes": memory.rss,
                        "threads": threads,
                    }
                )
                if self._stop.wait(self.cadence_ms / 1000):
                    return
        except (psutil.Error, OSError) as error:
            if not self.samples:
                self.reason = f"process unavailable: {type(error).__name__}"
            elif not isinstance(error, psutil.NoSuchProcess):
                self.reason = f"sampling failed: {type(error).__name__}"


def sample_subprocess(
    command: list[str],
    cwd: Path,
    cadence_ms: int = 250,
    *,
    sample_resources: bool = True,
    timeout_seconds: float = 1800,
) -> tuple[int, int, str, dict[str, Any]]:
    """Run an owned bounded child and sample that exact launched PID."""
    started_ns = time.monotonic_ns()
    started_epoch_ns = time.time_ns()
    boot_time_ns = round(psutil.boot_time() * 1_000_000_000)
    sampler = None
    pid = None

    def started(child) -> None:
        nonlocal sampler, pid
        pid = child.pid
        if sample_resources:
            sampler = ProcessResourceSampler(pid, cadence_ms=cadence_ms)
            sampler.start()

    try:
        result = run_acquisition(
            command, cwd=cwd, timeout_seconds=timeout_seconds, on_started=started
        )
    finally:
        if sampler is not None:
            resources = sampler.stop()
    assert pid is not None
    if sampler is None:
        resources = {
            "schema_version": 2,
            "status": "unavailable",
            "reason": "resource sampling intentionally disabled for paired control",
            "pid": pid,
            "process_create_time_ns": None,
            "boot_time_ns": boot_time_ns,
            "cadence_ms": cadence_ms,
            "sampling_started_monotonic_ns": started_ns,
            "sampling_ended_monotonic_ns": time.monotonic_ns(),
            "sampling_started_epoch_ns": started_epoch_ns,
            "sampling_ended_epoch_ns": time.time_ns(),
            "samples": [],
        }
    resources["schema_version"] = 3
    resources["execution"] = execution_record(result, timeout_seconds)
    incomplete = (
        result.returncode != 0
        or result.timed_out
        or result.interrupted_by_signal is not None
        or result.aborted_early
    )
    code = result.returncode if result.returncode is not None else 1
    if incomplete and code == 0:
        code = 1
    stderr = result.stderr
    if incomplete and result.stdout:
        stderr += "\nACQUISITION: partial stdout\n" + result.stdout
    return code, pid, stderr, resources
