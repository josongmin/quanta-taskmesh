"""Process sampler checks its own artifact; this is not a host performance run."""

from __future__ import annotations

import json
import subprocess
import sys
import time
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_perf  # noqa: E402
from process_resource import ProcessResourceSampler, sample_subprocess  # noqa: E402


def test_sampler_keeps_bounded_cpu_rss_and_thread_rows() -> None:
    child = subprocess.Popen(
        [sys.executable, "-c", "import sys; sys.stdin.readline()"],
        stdin=subprocess.PIPE,
    )
    sampler = ProcessResourceSampler(child.pid, cadence_ms=25, max_samples=100)
    try:
        sampler.start()
        deadline = time.monotonic() + 5
        while not sampler.samples and time.monotonic() < deadline:
            time.sleep(0.01)
        assert sampler.samples, sampler.reason
        assert child.stdin is not None
        child.stdin.write(b"\n")
        child.stdin.flush()
        child.wait(timeout=5)
    finally:
        if child.poll() is None:
            child.kill()
            child.wait(timeout=5)
    artifact = sampler.stop()
    assert artifact["status"] == "complete"
    assert artifact["schema_version"] == 2
    assert artifact["pid"] == child.pid
    assert artifact["boot_time_ns"] <= artifact["process_create_time_ns"]
    assert artifact["sampling_started_epoch_ns"] < artifact["sampling_ended_epoch_ns"]
    assert 1 <= len(artifact["samples"]) <= 100
    artifact["schema_version"] = 3
    artifact["execution"] = {
        "timeout_seconds": 1800.0,
        "returncode": 0,
        "timed_out": False,
        "interrupted_by_signal": None,
        "aborted_early": False,
    }
    raw = json.dumps(artifact).encode()
    assert host_perf.validate_resource_artifact(raw, child.pid) == artifact
    artifact["samples"][0]["threads"] = 0
    with pytest.raises(host_perf.ReceiptError, match="thread count"):
        host_perf.validate_resource_artifact(json.dumps(artifact).encode(), child.pid)
    artifact["samples"][0]["threads"] = 1
    artifact["sampling_ended_epoch_ns"] += 2_000_000_000
    with pytest.raises(host_perf.ReceiptError, match="cross-clock window"):
        host_perf.validate_resource_artifact(json.dumps(artifact).encode(), child.pid)


def test_sampler_off_control_retains_typed_unavailable_artifact(tmp_path: Path) -> None:
    exit_code, pid, stderr, artifact = sample_subprocess(
        [sys.executable, "-c", "pass"], tmp_path, sample_resources=False
    )
    assert exit_code == 0, stderr
    assert artifact["status"] == "unavailable"
    assert artifact["reason"] == "resource sampling intentionally disabled for paired control"
    assert artifact["samples"] == []
    assert host_perf.validate_resource_artifact(json.dumps(artifact).encode(), pid) == artifact


def test_sample_subprocess_timeout_retains_pid_partial_output_and_failure(tmp_path: Path) -> None:
    started = time.monotonic()
    code, pid, stderr, resources = sample_subprocess(
        [sys.executable, "-c", "import time; print('partial stdout', flush=True); time.sleep(60)"],
        tmp_path,
        cadence_ms=10,
        timeout_seconds=0.2,
    )
    assert time.monotonic() - started < 3
    assert code != 0
    assert resources["schema_version"] == 3
    assert resources["pid"] == pid
    assert resources["execution"]["timed_out"] is True
    assert resources["sampling_started_epoch_ns"] <= resources["sampling_ended_epoch_ns"]
    assert "partial stdout" in stderr
