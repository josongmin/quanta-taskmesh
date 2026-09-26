"""Complete same-host populations and retained executable custody, with real replay."""

from __future__ import annotations

import json
import shutil
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_perf  # noqa: E402
import host_run  # noqa: E402
import host_stability  # noqa: E402


@pytest.fixture(scope="module")
def binary():
    return host_run.build_runner([], "host_stability_probe")


@pytest.fixture
def setup(tmp_path, monkeypatch, binary):
    scenario = json.loads(
        (host_perf.REPO / "tools/bench/scenarios/h2-burst-recovery-smoke.json").read_text()
    )
    manifest = {
        "schema_version": 1,
        "scenario": scenario,
        "cycles": 3,
        "min_duration_ms": 0,
        "max_total_records": 100,
    }
    path = tmp_path / "input.json"
    path.write_bytes(host_perf.canonical(manifest))
    identity = host_perf.local_identity(scenario, [])
    monkeypatch.setattr(host_perf, "local_identity", lambda *_: identity)
    monkeypatch.setattr(host_stability, "source_content_sha256", lambda: "a" * 64)
    monkeypatch.setattr(host_run, "build_runner", lambda *_: binary)
    return path, tmp_path / "bundle", identity


@pytest.fixture
def bundle(setup):
    path, directory, _ = setup
    receipt = host_stability.acquire(path, directory)
    assert receipt["status"] == "complete"
    return directory


def rewrite(directory, name, value):
    (directory / name).write_bytes(host_perf.canonical(value))
    receipt = json.loads((directory / "receipt.json").read_bytes())
    receipt["artifacts"][name] = host_perf.sha256((directory / name).read_bytes())
    (directory / "receipt.json").write_bytes(host_perf.canonical(receipt))


def test_real_smoke_and_private_typed_replay(bundle):
    receipt = host_stability.verify_receipt(bundle, require_current_source=True)
    assert receipt["performance_status"] == "UNQUALIFIED"
    assert len([p for p in (bundle / "cycles").iterdir()]) == 3


def test_run_rejects_existing_bundle_without_mutation(bundle, setup, monkeypatch):
    path, _, _ = setup
    before = {
        str(item.relative_to(bundle)): host_perf.sha256(item.read_bytes())
        for item in bundle.rglob("*")
        if item.is_file()
    }
    with pytest.raises(host_perf.ReceiptError, match="new output directory"):
        host_stability.acquire(path, bundle)
    monkeypatch.setattr(sys, "argv", ["host_stability.py", "run", str(path), str(bundle)])
    assert host_stability.main() == 1
    after = {
        str(item.relative_to(bundle)): host_perf.sha256(item.read_bytes())
        for item in bundle.rglob("*")
        if item.is_file()
    }
    assert after == before
    assert host_stability.verify_receipt(bundle)["status"] == "complete"


def test_owned_build_failure_retains_rejection(setup, monkeypatch):
    path, directory, _ = setup

    def failed_build(*_args):
        raise host_perf.ReceiptError("build unavailable")

    monkeypatch.setattr(host_run, "build_runner", failed_build)
    with pytest.raises(host_perf.ReceiptError, match="build unavailable"):
        host_stability.acquire(path, directory)
    rejection = json.loads((directory / "rejection.json").read_bytes())
    assert rejection["status"] == "rejected"
    assert rejection["phase"] == "stability acquisition"
    assert "build unavailable" in rejection["reason"]
    assert not (directory / "receipt.json").exists()


def test_rejection_write_failure_preserves_primary_error(setup, monkeypatch, capsys):
    path, directory, _ = setup

    def failed_build(*_args):
        raise host_perf.ReceiptError("primary build failure")

    original_write = host_run.write_new

    def failed_retention(destination, data):
        if destination.name == "rejection.json":
            raise OSError("retention failed")
        original_write(destination, data)

    monkeypatch.setattr(host_run, "build_runner", failed_build)
    monkeypatch.setattr(host_run, "write_new", failed_retention)
    with pytest.raises(host_perf.ReceiptError, match="primary build failure"):
        host_stability.acquire(path, directory)
    assert "retention failed" in capsys.readouterr().err


@pytest.mark.parametrize(
    "mutation", ["missing", "extra", "reorder", "canary", "counter", "admission", "raw", "drain"]
)
def test_population_and_typed_corruptions_reject_even_with_new_hashes(bundle, mutation):
    cycle_path = "cycles/cycle-000001.json"
    if mutation == "missing":
        (bundle / cycle_path).unlink()
    elif mutation == "extra":
        shutil.copy(bundle / cycle_path, bundle / "cycles/cycle-999999.json")
    else:
        cycle = json.loads((bundle / cycle_path).read_bytes())
        if mutation == "reorder":
            cycle["index"] = 0
        elif mutation == "canary":
            cycle["canaries"][0]["body_ran"] = False
        elif mutation == "counter":
            cycle["before"] = json.loads((bundle / "cycles/cycle-000000.json").read_bytes())[
                "before"
            ]
        elif mutation == "admission":
            cycle["admission_open"] = False
        elif mutation == "raw":
            cycle["window"]["records"].pop()
        else:
            cycle["window"]["drain_ok"] = True
        rewrite(bundle, cycle_path, cycle)
    with pytest.raises(host_perf.ReceiptError):
        host_stability.verify_receipt(bundle)


@pytest.mark.parametrize("name", ["runner", "manifest.json", "resources.json", "summary.json"])
def test_artifact_tamper_rejects_before_execution(bundle, name):
    with (bundle / name).open("ab") as stream:
        stream.write(b"tampered")
    with pytest.raises(host_perf.ReceiptError):
        host_stability.verify_receipt(bundle)


def test_resource_unavailable_does_not_qualify_leak_or_performance(bundle):
    resources = json.loads((bundle / "resources.json").read_bytes())
    resources.update(status="unavailable", reason="process resource sample cap reached")
    rewrite(bundle, "resources.json", resources)
    assert host_stability.verify_receipt(bundle)["performance_status"] == "UNQUALIFIED"


def test_wrong_pid_rejects_with_valid_digests(bundle):
    resources = json.loads((bundle / "resources.json").read_bytes())
    resources["pid"] += 1
    rewrite(bundle, "resources.json", resources)
    with pytest.raises(host_perf.ReceiptError, match="PID"):
        host_stability.verify_receipt(bundle)


def test_timeout_retains_partial_and_cannot_pass(setup):
    path, directory, _ = setup
    with pytest.raises(host_perf.ReceiptError, match="execution failed"):
        host_stability.acquire(path, directory, timeout_seconds=0.02)
    receipt = json.loads((directory / "receipt.json").read_bytes())
    assert receipt["status"] == "invalid"
    assert (directory / "stderr.txt").is_file()
    assert (directory / "resources.json").is_file()
    with pytest.raises((OSError, host_perf.ReceiptError)):
        host_stability.verify_receipt(directory)


def test_source_drift_retains_invalid_receipt(setup, monkeypatch):
    path, directory, _ = setup
    calls = 0

    def digest():
        nonlocal calls
        calls += 1
        return ("a" if calls <= 3 else "b") * 64

    monkeypatch.setattr(host_stability, "source_content_sha256", digest)
    with pytest.raises(host_perf.ReceiptError, match="identity changed"):
        host_stability.acquire(path, directory)
    assert json.loads((directory / "receipt.json").read_bytes())["status"] == "invalid"


@pytest.mark.parametrize("budget", [0, -1, float("nan"), float("inf")])
def test_invalid_timeout_rejects_before_build(setup, monkeypatch, budget):
    path, directory, _ = setup
    monkeypatch.setattr(host_run, "build_runner", lambda *_: pytest.fail("must not build"))
    with pytest.raises(host_perf.ReceiptError):
        host_stability.acquire(path, directory, timeout_seconds=budget)
    assert not directory.exists()


def test_sealed_runner_execution_survives_original_replacement(bundle, monkeypatch):
    original = host_stability.run_acquisition

    def run(command, **kwargs):
        assert Path(command[0]).parent != bundle
        (bundle / "runner").write_bytes(b"replaced after sealing")
        return original(command, **kwargs)

    monkeypatch.setattr(host_stability, "run_acquisition", run)
    assert host_stability.verify_receipt(bundle)["status"] == "complete"


def test_replay_never_executes_self_hashed_bundle_runner(bundle, tmp_path):
    marker = tmp_path / "untrusted-runner-executed"
    runner = bundle / "runner"
    runner.write_bytes(f"#!/bin/sh\ntouch '{marker}'\n".encode())
    receipt = json.loads((bundle / "receipt.json").read_bytes())
    receipt["artifacts"]["runner"] = host_perf.sha256(runner.read_bytes())
    (bundle / "receipt.json").write_bytes(host_perf.canonical(receipt))

    # Historical replay checks structure; the self-declared hash is not an
    # authority for executing or authenticating this retained executable.
    assert host_stability.verify_receipt(bundle)["status"] == "complete"
    assert not marker.exists()
    with pytest.raises(host_perf.ReceiptError, match="retained runner differs"):
        host_stability.verify_receipt(bundle, require_current_source=True)


def test_nonregular_cycle_rejects_without_read_wait(bundle):
    cycle = bundle / "cycles/cycle-000001.json"
    cycle.unlink()
    cycle.mkdir()
    with pytest.raises(OSError, match="regular"):
        host_stability.verify_receipt(bundle)


@pytest.mark.parametrize(
    "name,base",
    [
        ("h2-stability-smoke", "h2-burst-recovery-smoke"),
        ("h5-stability-smoke", "h5-custody-smoke"),
        ("h2-stability-stress", "h2-burst-recovery-smoke"),
    ],
)
def test_frozen_manifest_reuses_existing_workload(name, base):
    root = host_perf.REPO / "tools/bench/scenarios"
    manifest = host_stability.manifest_object((root / f"{name}.json").read_bytes())
    assert manifest["scenario"] == json.loads((root / f"{base}.json").read_bytes())


def test_interrupt_uses_same_failure_retention_path(setup, monkeypatch):
    path, directory, _ = setup
    actual_sample = host_stability.sample_subprocess

    def interrupted(*args, **kwargs):
        code, pid, stderr, resources = actual_sample(*args, **kwargs)
        assert code == 0
        return -1, pid, stderr + "\nbenchmark interrupted by signal 2\n", resources

    monkeypatch.setattr(host_stability, "sample_subprocess", interrupted)
    with pytest.raises(host_perf.ReceiptError, match="interrupted"):
        host_stability.acquire(path, directory)
    assert len(list((directory / "cycles").iterdir())) == 3
    assert json.loads((directory / "receipt.json").read_bytes())["status"] == "invalid"
    with pytest.raises(host_perf.ReceiptError):
        host_stability.verify_receipt(directory)


@pytest.mark.parametrize("version", [True, 1.0, "1", 0, 2])
def test_receipt_version_requires_exact_integer(bundle, version):
    receipt = json.loads((bundle / "receipt.json").read_bytes())
    receipt["schema_version"] = version
    (bundle / "receipt.json").write_bytes(host_perf.canonical(receipt))
    with pytest.raises(host_perf.ReceiptError, match="invalid stability receipt"):
        host_stability.verify_receipt(bundle)
