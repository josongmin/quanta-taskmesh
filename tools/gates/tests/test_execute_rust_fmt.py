"""A nested Cargo workspace must remain inside the formatting gate."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import execute_rust_fmt  # noqa: E402


def test_format_check_rejects_a_nested_workspace_with_bad_format(tmp_path: Path) -> None:
    subprocess.run(["git", "init", "-q"], cwd=tmp_path, check=True)
    (tmp_path / "Cargo.toml").write_text(
        '[package]\nname = "root"\nversion = "0.1.0"\nedition = "2021"\n\n'
        '[workspace]\nmembers = []\n',
        encoding="utf-8",
    )
    (tmp_path / "src").mkdir()
    (tmp_path / "src" / "main.rs").write_text("fn main() {}\n", encoding="utf-8")
    nested = tmp_path / "nested"
    (nested / "src").mkdir(parents=True)
    (nested / "Cargo.toml").write_text(
        '[package]\nname = "nested"\nversion = "0.1.0"\nedition = "2021"\n\n[workspace]\n',
        encoding="utf-8",
    )
    (nested / "src" / "main.rs").write_text("fn main(){println!(\"x\");}\n", encoding="utf-8")
    subprocess.run(["git", "add", "."], cwd=tmp_path, check=True)

    assert execute_rust_fmt.cargo_manifests(tmp_path) == [
        "Cargo.toml", "nested/Cargo.toml",
    ]
    assert execute_rust_fmt.main(tmp_path, check=True) == 1
    assert execute_rust_fmt.main(tmp_path, check=False) == 0
    assert execute_rust_fmt.main(tmp_path, check=True) == 0
