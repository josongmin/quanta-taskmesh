"""The lint gate must cover Python sources outside its tooling directory."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import execute_py_lint  # noqa: E402


def test_python_lint_rejects_a_tracked_validation_script_outside_tools(tmp_path: Path) -> None:
    subprocess.run(["git", "init", "-q"], cwd=tmp_path, check=True)
    script = tmp_path / "docs" / "validate.py"
    script.parent.mkdir()
    script.write_text("import os\n", encoding="utf-8")
    subprocess.run(["git", "add", "docs/validate.py"], cwd=tmp_path, check=True)

    assert execute_py_lint.python_sources(tmp_path) == ["docs/validate.py"]
    assert execute_py_lint.main(tmp_path) == 1
