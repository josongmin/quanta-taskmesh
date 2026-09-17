"""PM regressions for H16-020 (TM16-035, TM16-039, and read-only lint).

The first two cases run the real CLI against the fixtures retained by the
September audit, exactly as the audit did, so the fix is measured against the
reproduction rather than against a re-description of it.
"""

from __future__ import annotations

import hashlib
import os
import subprocess
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import pm  # noqa: E402

REPO = Path(__file__).resolve().parents[3]
PM_CLI = REPO / "tools" / "pm" / "pm.py"
EVIDENCE = REPO / "docs" / "bugbash" / "sep-16-general" / "tickets" / "evidence"


def run_cli(fixture: Path, *argv: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            sys.executable,
            str(PM_CLI),
            "--targets",
            str(fixture / "targets.yaml"),
            "--pm-dir",
            str(fixture),
            "--repo-root",
            str(fixture),
            *argv,
        ],
        capture_output=True,
        text=True,
        check=False,
        env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
    )


def tree_digest(root: Path) -> dict[str, tuple[str, int, bool]]:
    """(content sha256, mode, is_symlink) for every file under root."""
    out = {}
    for path in sorted(root.rglob("*")):
        if path.is_dir() and not path.is_symlink():
            continue
        rel = str(path.relative_to(root))
        if path.is_symlink():
            out[rel] = (hashlib.sha256(os.readlink(path).encode()).hexdigest(), 0, True)
        else:
            out[rel] = (hashlib.sha256(path.read_bytes()).hexdigest(), path.stat().st_mode, False)
    return out


# ---- TM16-035: template identity is the path, not the basename ---------------


def test_nested_template_is_rendered_not_its_root_namesake() -> None:
    """The audit fixture: `templates/nested/demo.j2` names RIGHT, `templates/demo.j2`
    names WRONG, and the manifest asks for the nested one."""
    proc = run_cli(EVIDENCE / "pm-nested-template", "preview", "--target", "nested")
    assert proc.returncode == 0, proc.stderr
    assert "RIGHT TEMPLATE" in proc.stdout
    assert "WRONG TEMPLATE" not in proc.stdout


def test_lint_rejects_the_output_that_the_wrong_template_produced() -> None:
    """`WRONG.md` on disk is what the old code rendered. It must now be drift."""
    proc = run_cli(EVIDENCE / "pm-nested-template", "lint")
    assert proc.returncode == 1, proc.stdout + proc.stderr
    assert "nested (drift)" in proc.stderr


def test_nested_only_template_works(tmp_path: Path) -> None:
    pm_dir = tmp_path / "pm"
    (pm_dir / "sources").mkdir(parents=True)
    (pm_dir / "templates" / "deep" / "er").mkdir(parents=True)
    (pm_dir / "sources" / "a.md").write_text("body", encoding="utf-8")
    (pm_dir / "templates" / "deep" / "er" / "only.j2").write_text(
        "DEEP {{ body }}", encoding="utf-8"
    )
    (pm_dir / "targets.yaml").write_text(
        "targets:\n  t:\n    output: OUT.md\n    template: templates/deep/er/only.j2\n"
        "    sections: [sources/a.md]\n",
        encoding="utf-8",
    )
    targets = pm.load_targets(pm_dir / "targets.yaml")
    plan = pm.plan_target(targets["t"], pm_dir, tmp_path)
    assert plan.template_path == (pm_dir / "templates" / "deep" / "er" / "only.j2").resolve()
    assert plan.rendered.startswith("DEEP body")


def test_template_include_still_resolves_from_the_template_root(tmp_path: Path) -> None:
    pm_dir = tmp_path / "pm"
    (pm_dir / "sources").mkdir(parents=True)
    (pm_dir / "templates" / "sub").mkdir(parents=True)
    (pm_dir / "sources" / "a.md").write_text("body", encoding="utf-8")
    (pm_dir / "templates" / "_banner.j2").write_text("BANNER\n", encoding="utf-8")
    (pm_dir / "templates" / "sub" / "t.j2").write_text(
        '{% include "_banner.j2" %}{{ body }}', encoding="utf-8"
    )
    (pm_dir / "targets.yaml").write_text(
        "targets:\n  t:\n    output: OUT.md\n    template: templates/sub/t.j2\n"
        "    sections: [sources/a.md]\n",
        encoding="utf-8",
    )
    targets = pm.load_targets(pm_dir / "targets.yaml")
    assert pm.plan_target(targets["t"], pm_dir, tmp_path).rendered == "BANNER\nbody\n"


# ---- TM16-039: duplicate keys are refused before the dict collapses ----------


def test_duplicate_target_key_fails_lint_and_names_the_key() -> None:
    proc = run_cli(EVIDENCE / "pm-duplicate-target", "lint")
    assert proc.returncode == 2, proc.stdout + proc.stderr
    assert "duplicate mapping key 'demo'" in proc.stderr
    assert "line 6" in proc.stderr and "line 2" in proc.stderr


def test_duplicate_target_key_blocks_sync_before_any_write() -> None:
    fixture = EVIDENCE / "pm-duplicate-target"
    before = tree_digest(fixture)
    proc = run_cli(fixture, "sync")
    assert proc.returncode == 2
    assert not (fixture / "MISSING.md").exists(), (
        "sync must not create the first declaration's output either"
    )
    assert tree_digest(fixture) == before, "a refused sync writes nothing"


def test_duplicate_keys_with_identical_values_are_still_errors(tmp_path: Path) -> None:
    text = (
        "targets:\n"
        "  t:\n    output: a\n    template: templates/x.j2\n    sections: [s]\n"
        "  t:\n    output: a\n    template: templates/x.j2\n    sections: [s]\n"
    )
    with pytest.raises(pm.PmError, match="duplicate mapping key 't'"):
        pm.strict_yaml_load(text)


def test_duplicate_keys_inside_a_target_are_errors(tmp_path: Path) -> None:
    text = (
        "targets:\n  t:\n    output: a\n    output: b\n"
        "    template: templates/x.j2\n    sections: [s]\n"
    )
    with pytest.raises(pm.PmError, match="duplicate mapping key 'output'"):
        pm.strict_yaml_load(text)


# ---- manifest validation -----------------------------------------------------


def _manifest(output: str = "x", template: str = "templates/x.j2", extra: str = "") -> str:
    return (
        "targets:\n  t:\n"
        f"    output: {output}\n    template: {template}\n    sections: [s]\n{extra}"
    )


@pytest.mark.parametrize(
    "manifest,needle",
    [
        (_manifest(output="/etc/x"), "must be relative"),
        (_manifest(output="../x"), "must not contain '..'"),
        (_manifest(template="other/x.j2"), "must live under templates/"),
        (_manifest(template="templates/../x.j2"), "must not contain '..'"),
        (_manifest(extra="    extra: 1\n"), "unknown key"),
        (
            "targets:\n"
            "  a:\n    output: ./x\n    template: templates/x.j2\n    sections: [s]\n"
            "  b:\n    output: x\n    template: templates/x.j2\n    sections: [s]\n",
            "both write 'x'",
        ),
        ("targets: {}\nbogus: 1\n", "unknown top-level keys"),
    ],
)
def test_invalid_manifests_are_rejected(tmp_path: Path, manifest: str, needle: str) -> None:
    path = tmp_path / "targets.yaml"
    path.write_text(manifest, encoding="utf-8")
    with pytest.raises(pm.PmError, match=needle):
        pm.load_targets(path)


def test_a_template_symlink_escaping_the_template_root_is_refused(tmp_path: Path) -> None:
    pm_dir = tmp_path / "pm"
    (pm_dir / "sources").mkdir(parents=True)
    (pm_dir / "templates").mkdir()
    (pm_dir / "sources" / "a.md").write_text("body", encoding="utf-8")
    outside = tmp_path / "outside.j2"
    outside.write_text("ESCAPED {{ body }}", encoding="utf-8")
    (pm_dir / "templates" / "link.j2").symlink_to(outside)
    (pm_dir / "targets.yaml").write_text(
        "targets:\n  t:\n    output: OUT.md\n    template: templates/link.j2\n"
        "    sections: [sources/a.md]\n",
        encoding="utf-8",
    )
    targets = pm.load_targets(pm_dir / "targets.yaml")
    with pytest.raises(pm.PmError, match="resolves outside"):
        pm.plan_target(targets["t"], pm_dir, tmp_path)


# ---- lint is read-only; sync is safe ----------------------------------------


def _fixture(tmp_path: Path) -> tuple[Path, Path]:
    pm_dir = tmp_path / "pm"
    repo = tmp_path / "repo"
    (pm_dir / "sources").mkdir(parents=True)
    (pm_dir / "templates").mkdir()
    repo.mkdir()
    (pm_dir / "sources" / "a.md").write_text("alpha", encoding="utf-8")
    (pm_dir / "templates" / "t.j2").write_text("GEN {{ body }}", encoding="utf-8")
    (pm_dir / "targets.yaml").write_text(
        "targets:\n  t:\n    output: OUT.md\n    template: templates/t.j2\n"
        "    sections: [sources/a.md]\n",
        encoding="utf-8",
    )
    return pm_dir, repo


def test_lint_never_modifies_the_tree(tmp_path: Path) -> None:
    pm_dir, repo = _fixture(tmp_path)
    # Drifted, missing, and ok states — none may write.
    (repo / "OUT.md").write_text("hand edited\n", encoding="utf-8")
    os.chmod(repo / "OUT.md", 0o640)
    for _ in range(2):
        before = tree_digest(tmp_path)
        proc = run_cli_dirs(pm_dir, repo, "lint")
        assert proc.returncode == 1
        assert tree_digest(tmp_path) == before, "lint must not change bytes, modes, or links"
    (repo / "OUT.md").unlink()
    before = tree_digest(tmp_path)
    assert run_cli_dirs(pm_dir, repo, "lint").returncode == 1
    assert run_cli_dirs(pm_dir, repo, "status").returncode == 0
    assert run_cli_dirs(pm_dir, repo, "preview", "--target", "t").returncode == 0
    assert tree_digest(tmp_path) == before


def run_cli_dirs(pm_dir: Path, repo: Path, *argv: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            sys.executable,
            str(PM_CLI),
            "--targets",
            str(pm_dir / "targets.yaml"),
            "--pm-dir",
            str(pm_dir),
            "--repo-root",
            str(repo),
            *argv,
        ],
        capture_output=True,
        text=True,
        check=False,
        env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
    )


def test_sync_refuses_to_overwrite_a_hand_edited_target(tmp_path: Path) -> None:
    pm_dir, repo = _fixture(tmp_path)
    (repo / "OUT.md").write_text("the user's own words\n", encoding="utf-8")
    proc = run_cli_dirs(pm_dir, repo, "sync")
    assert proc.returncode == 1
    assert "refused" in proc.stderr
    assert (repo / "OUT.md").read_text(encoding="utf-8") == "the user's own words\n"
    # With explicit consent it is replaced, atomically.
    proc = run_cli_dirs(pm_dir, repo, "sync", "--force")
    assert proc.returncode == 0, proc.stderr
    assert (repo / "OUT.md").read_text(encoding="utf-8") == "GEN alpha\n"
    assert not list(repo.glob("*.pm-tmp")), "no temp files left behind"


def test_sync_creates_missing_targets_and_leaves_matching_ones_alone(tmp_path: Path) -> None:
    pm_dir, repo = _fixture(tmp_path)
    proc = run_cli_dirs(pm_dir, repo, "sync")
    assert proc.returncode == 0, proc.stderr
    assert "created" in proc.stdout
    proc = run_cli_dirs(pm_dir, repo, "sync")
    assert proc.returncode == 0
    assert "unchanged" in proc.stdout


def test_a_broken_target_aborts_sync_before_any_file_is_written(tmp_path: Path) -> None:
    """Planning is all-or-nothing: the good target is not written if the bad one fails."""
    pm_dir, repo = _fixture(tmp_path)
    (pm_dir / "targets.yaml").write_text(
        "targets:\n"
        "  good:\n    output: GOOD.md\n    template: templates/t.j2\n"
        "    sections: [sources/a.md]\n"
        "  bad:\n    output: BAD.md\n    template: templates/missing.j2\n"
        "    sections: [sources/a.md]\n",
        encoding="utf-8",
    )
    before = tree_digest(repo)
    proc = run_cli_dirs(pm_dir, repo, "sync")
    assert proc.returncode == 2
    assert "missing template" in proc.stderr
    assert tree_digest(repo) == before
    assert not (repo / "GOOD.md").exists()


def test_the_real_manifest_lints_clean_and_is_explicit_about_owning_nothing() -> None:
    """See tools/pm/README.md: AGENTS.md is hand-owned; PM currently owns no target."""
    proc = subprocess.run(
        [sys.executable, str(PM_CLI), "lint"],
        capture_output=True,
        text=True,
        check=False,
        cwd=REPO,
    )
    assert proc.returncode == 0, proc.stdout + proc.stderr
    assert "0 target(s)" in proc.stdout
    manifest = (REPO / "tools" / "pm" / "targets.yaml").read_text(encoding="utf-8")
    assert "AGENTS.md" not in manifest.split("targets:")[-1], "AGENTS.md must not be a PM target"
