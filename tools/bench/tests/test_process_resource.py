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
from process_resource import ProcessResourceSampler  # noqa: E402


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
    assert artifact["pid"] == child.pid
    assert 1 <= len(artifact["samples"]) <= 100
    raw = json.dumps(artifact).encode()
    assert host_perf.validate_resource_artifact(raw, child.pid) == artifact
    artifact["samples"][0]["threads"] = 0
    with pytest.raises(host_perf.ReceiptError, match="thread count"):
        host_perf.validate_resource_artifact(json.dumps(artifact).encode(), child.pid)
