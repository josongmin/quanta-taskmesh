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
