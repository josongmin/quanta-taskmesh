"""Frozen source custody must precede any executable reproducibility claim."""

from __future__ import annotations

import io
import subprocess
import sys
import tarfile
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_build  # noqa: E402
import host_perf  # noqa: E402

from tools.process_supervisor import SupervisedProcess  # noqa: E402


@pytest.mark.parametrize("index_hint", ["--assume-unchanged", "--skip-worktree"])
def test_frozen_build_rejects_index_hidden_source_before_build(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, index_hint: str
) -> None:
    source = tmp_path / "source"
    source.mkdir()
    lock = source / "Cargo.lock"
    lock.write_text("committed lock\n")
    for args in (
        ["init", "-q"],
        ["add", "Cargo.lock"],
        ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
         "commit", "-qm", "fixture"],
        ["update-index", index_hint, "Cargo.lock"],
    ):
        subprocess.run(["git", *args], cwd=source, check=True, capture_output=True)
    lock.write_text("uncommitted lock\n")
    assert subprocess.check_output(["git", "status", "--porcelain=v1"], cwd=source) == b""
    monkeypatch.setattr(host_perf, "REPO", source)
    witness = tmp_path / "witness"
    with pytest.raises(host_perf.ReceiptError, match="clean committed source"):
        host_build.acquire(witness, [], "host_load_probe")
    assert not witness.exists()


def archive(name: str = "Cargo.lock", kind: bytes = tarfile.REGTYPE) -> bytes:
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode="w") as tar:
        member = tarfile.TarInfo(name)
        member.type, member.size, member.mode = kind, 4 if kind == tarfile.REGTYPE else 0, 0o644
        tar.addfile(member, io.BytesIO(b"lock") if member.size else None)
    return stream.getvalue()


@pytest.mark.parametrize(
    "name,kind",
    [
        ("../Cargo.lock", tarfile.REGTYPE),
        ("/Cargo.lock", tarfile.REGTYPE),
        ("Cargo.lock", tarfile.SYMTYPE),
        ("not-lock", tarfile.REGTYPE),
    ],
)
def test_archive_rejects_escape_links_and_missing_lock(name: str, kind: bytes) -> None:
    with pytest.raises(host_perf.ReceiptError):
        host_build.archive_members(archive(name, kind))


def test_source_inventory_rejects_edit_extra_and_symlink(tmp_path: Path) -> None:
    members = host_build.archive_members(archive())
    path = tmp_path / "Cargo.lock"
    path.write_bytes(b"lock")
    host_build.check_source(tmp_path, members)
    path.write_bytes(b"edit")
    with pytest.raises(host_perf.ReceiptError, match="source changed"):
        host_build.check_source(tmp_path, members)
    path.write_bytes(b"lock")
    (tmp_path / "extra").write_bytes(b"x")
    with pytest.raises(host_perf.ReceiptError, match="inventory changed"):
        host_build.check_source(tmp_path, members)
    (tmp_path / "extra").unlink()
    path.unlink()
    path.symlink_to("/dev/null")
    with pytest.raises(host_perf.ReceiptError, match="source changed"):
        host_build.check_source(tmp_path, members)


def setup_build(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    data = archive()

    def command(args: list[str], **_kwargs: object) -> bytes:
        if args[0] == "git":
            if args[1] == "status":
                return b""
            if args[1] == "archive":
                return data
            return b"a" * 40 if args[-1] == "HEAD" else b"b" * 40
        return (args[0] + " version").encode()

    monkeypatch.setattr(host_build, "command_output", command)
    monkeypatch.setattr(host_perf, "source_identity", lambda: {
        "source_dirty": False,
        "source_head": "a" * 40,
        "source_tree": "b" * 40,
        "source_content_sha256": "c" * 64,
    })
    monkeypatch.setattr(host_perf, "build_environment", lambda: {})

    def build(root: Path, _features: list, _example: str) -> tuple:
        binary = root / "fake-target"
        binary.write_bytes(b"binary")
        binary.chmod(0o755)
        return binary, ["default"], b"stdout", b"stderr"

    monkeypatch.setattr(host_build, "cold_build", build)
    root = tmp_path / "witness"
    host_build.acquire(root, [], "host_load_probe")
    return root


def test_rebuild_is_reexecuted_and_binary_change_rejects(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = setup_build(tmp_path, monkeypatch)
    calls = []

    def rebuild(directory: Path, _features: list, _example: str) -> tuple:
        calls.append(directory)
        binary = root / "fake-target"
        binary.write_bytes(b"other binary")
        return binary, ["default"], b"", b""

    monkeypatch.setattr(host_build, "cold_build", rebuild)
    host_build.verify(root)
    assert not calls
    with pytest.raises(host_perf.ReceiptError, match="cold rebuild differs"):
        host_build.verify(root, rebuild=True)
    assert len(calls) == 1


def test_runner_binds_clean_source_features_and_executable(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = setup_build(tmp_path, monkeypatch)
    identity = {
        "source_dirty": False,
        "source_head": "a" * 40,
        "source_tree": "b" * 40,
        "lock_sha256": host_perf.sha256(b"lock"),
    }
    binary, _, _ = host_build.runner(root, identity, [], "host_load_probe")
    assert binary.read_bytes() == b"binary"
    identity["source_dirty"] = True
    with pytest.raises(host_perf.ReceiptError, match="do not match frozen build"):
        host_build.runner(root, identity, [], "host_load_probe")
    (root / "runner-1").write_bytes(b"changed")
    with pytest.raises(host_perf.ReceiptError, match="executable changed"):
        host_build.verify(root)


@pytest.mark.parametrize(
    "profile", [None, {}, {"opt_level": "3", "debug_assertions": 0, "test": False}]
)
def test_cargo_profile_cannot_be_inferred_or_forged(profile: object) -> None:
    data = host_perf.canonical(
        {
            "reason": "compiler-artifact",
            "target": {"name": "host_load_probe", "kind": ["example"]},
            "profile": profile,
        }
    )
    with pytest.raises(host_perf.ReceiptError, match="build profile|profile is malformed"):
        host_build.cargo_profile(data, "host_load_probe")


def test_cargo_profile_is_read_from_the_actual_named_artifact() -> None:
    profile = {"opt_level": "3", "debug_assertions": False, "test": False, "overflow_checks": False}
    data = host_perf.canonical(
        {
            "reason": "compiler-artifact",
            "target": {"name": "host_load_probe", "kind": ["example"]},
            "profile": profile,
        }
    )
    assert host_build.cargo_profile(data, "host_load_probe") == profile
    with pytest.raises(host_perf.ReceiptError, match="no unique build profile"):
        host_build.cargo_profile(data + b"\n" + data, "host_load_probe")


def test_cargo_config_custody_includes_ancestors_and_global_home(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    home = tmp_path / "cargo-home"
    home.mkdir()
    monkeypatch.setenv("CARGO_HOME", str(home))
    source = tmp_path / "witness/source"
    source.mkdir(parents=True)
    ancestor = tmp_path / ".cargo"
    ancestor.mkdir()
    (ancestor / "config.toml").write_text('[build]\nrustflags = ["-C", "opt-level=0"]\n')
    (home / "config").write_text("[build]\njobs = 2\n")
    configs = host_build.cargo_config_sha256(source)
    assert str(ancestor / "config.toml") in configs
    assert str(home / "config") in configs
    proof = {"build_environment": {}, "cargo_config_sha256": configs}
    with pytest.raises(host_perf.ReceiptError, match="custom compiler flags"):
        host_build.require_standard_codegen(source.parent, proof)
    (ancestor / "config.toml").write_text("[build]\njobs = 3\n")
    with pytest.raises(host_perf.ReceiptError, match="configuration changed"):
        host_build.require_standard_codegen(source.parent, proof)
    proof["cargo_config_sha256"] = host_build.cargo_config_sha256(source)
    host_build.require_standard_codegen(source.parent, proof)


@pytest.mark.parametrize(
    "key",
    [
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTC",
        "RUSTC_WORKSPACE_WRAPPER",
        "CARGO_BUILD_RUSTFLAGS",
        "CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS",
    ],
)
def test_profile_metadata_cannot_admit_effective_codegen_overrides(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, key: str
) -> None:
    monkeypatch.setattr(host_build, "cargo_config_sha256", lambda _source: {})
    with pytest.raises(host_perf.ReceiptError, match="custom compiler flags"):
        host_build.require_standard_codegen(
            tmp_path, {"build_environment": {key: "override"}, "cargo_config_sha256": {}}
        )


@pytest.mark.parametrize(
    "config",
    [
        '[target.aarch64-apple-darwin]\nrustflags = ["-Copt-level=0"]',
        '[env]\nRUSTFLAGS = "-Copt-level=0"',
        '[build]\nrustc-workspace-wrapper = "wrapper"',
    ],
)
def test_config_codegen_injection_is_not_admitted(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, config: str
) -> None:
    path = tmp_path / "config.toml"
    path.write_text(config)
    configs = {str(path): host_perf.sha256(path.read_bytes())}
    monkeypatch.setattr(host_build, "cargo_config_sha256", lambda _source: configs)
    with pytest.raises(host_perf.ReceiptError, match="does not support"):
        host_build.require_standard_codegen(
            tmp_path, {"build_environment": {}, "cargo_config_sha256": configs}
        )


@pytest.mark.parametrize("timed_out,signum", [(True, None), (False, 15)])
def test_cold_build_failure_keeps_execution_and_logs(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, timed_out: bool, signum: object
) -> None:
    import json

    monkeypatch.setattr(
        host_build,
        "run_process",
        lambda *a, **k: SupervisedProcess(0, "partial", "failed", timed_out, signum),
    )
    with pytest.raises(host_perf.ReceiptError, match="frozen build failed"):
        host_build.cold_build(tmp_path, [], "host_load_probe")
    retained = list(tmp_path.glob("failed-build-*"))
    assert len(retained) == 1
    assert (retained[0] / "stdout").read_text() == "partial"
    execution = json.loads((retained[0] / "execution.json").read_bytes())
    assert execution["timed_out"] is timed_out
    assert execution["interrupted_by_signal"] == signum
    assert not (tmp_path / "build-witness.json").exists()


@pytest.mark.parametrize(
    "change", ["none", "missing", "duplicate", "unoptimized", "assertions", "overflow", "bool_opt"]
)
def test_library_and_actual_oracle_profiles_are_bound_to_measured_semantics(
    tmp_path: Path, change: str
) -> None:
    profile = {"opt_level": "3", "debug_assertions": False, "overflow_checks": False, "test": False}
    names = ["taskmesh", "taskmesh_contract", "taskmesh_engine", "taskmesh_bench"]

    def rows(targets: list[str]) -> bytes:
        events = [
            {
                "reason": "compiler-artifact",
                "target": {
                    "name": name,
                    "kind": ["test"] if name.startswith("hardening_") else ["lib"],
                },
                "profile": {**profile, "test": name.startswith("hardening_")},
            }
            for name in targets
        ]
        if change == "missing":
            events = [e for e in events if e["target"]["name"] != "taskmesh_engine"]
        elif change == "duplicate":
            events.append(events[0])
        elif change != "none":
            field, value = {
                "unoptimized": ("opt_level", "0"),
                "assertions": ("debug_assertions", True),
                "overflow": ("overflow_checks", True),
                "bool_opt": ("opt_level", True),
            }[change]
            events[0]["profile"][field] = value
        # Cargo emits reason first; canonical JSON emits profile first. Both
        # layouts must be understood by the oracle parser.
        import json

        return (
            b"\n".join(
                (json.dumps(e).encode() if i % 2 else host_perf.canonical(e))
                for i, e in enumerate(events)
            )
            + b"\n"
        )

    log = rows(names)
    proof = {"logs_sha256": {f"build-{i}.stdout": host_perf.sha256(log) for i in range(2)}}
    for index in range(2):
        (tmp_path / f"build-{index}.stdout").write_bytes(log)
    oracle = rows(
        names[:3]
        + [
            "hardening_mixed_overload",
            "hardening_deadline_custody",
            "hardening_drain",
            "hardening_executor_protocol",
        ]
    )
    if change == "none":
        host_build.require_matching_library_profiles(tmp_path, proof, profile)
        host_build.require_oracle_profiles(oracle + b"test result: ok.\n", profile)
    else:
        with pytest.raises(host_perf.ReceiptError, match="profile"):
            host_build.require_matching_library_profiles(tmp_path, proof, profile)
        with pytest.raises(host_perf.ReceiptError, match="profile"):
            host_build.require_oracle_profiles(oracle, profile)
