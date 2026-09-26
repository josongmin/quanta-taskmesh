"""Frozen source custody must precede any executable reproducibility claim."""

from __future__ import annotations

import io
import sys
import tarfile
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import host_build  # noqa: E402
import host_perf  # noqa: E402


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
