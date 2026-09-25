"""The Sep-16 plan's bookkeeping is validated by the gate, not by hand.

`docs/archive/2026-09-25/sep-16-hardening/tickets/validate_plan.py` checks the manifest,
the ticket structure, links, acceptance-id inventory, and — in the implemented
state — that every unchecked box in a ticket is a registered exception
(`EXCEPTIONS.md`) rather than an unfinished item. Nothing ran it before this
test; a validator nobody runs is documentation.
"""

from __future__ import annotations

import importlib.util
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
VALIDATOR = REPO / "docs" / "archive" / "2026-09-25" / "sep-16-hardening" / "tickets" / "validate_plan.py"

_spec = importlib.util.spec_from_file_location("validate_plan", VALIDATOR)
assert _spec and _spec.loader
vp = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(vp)


def test_the_committed_plan_validates() -> None:
    proc = subprocess.run(
        [sys.executable, str(VALIDATOR)], capture_output=True, text=True, check=False, cwd=REPO
    )
    assert proc.returncode == 0, proc.stdout + proc.stderr
    assert "unchecked items all registered in EXCEPTIONS.md" in proc.stdout


def test_an_unchecked_item_that_is_not_a_registered_exception_is_reported(tmp_path: Path) -> None:
    """Three ways the register and the tickets can disagree, each named."""
    (tmp_path / "EXCEPTIONS.md").write_text(
        "# 예외 대장\n\n| 항목 | 분류 |\n|---|---|\n| H16-001-A02 x | BLOCKED |\n",
        encoding="utf-8",
    )
    ticket = tmp_path / "H16-001-x.md"

    # 1. an unchecked box with no link to the register
    ticket.write_text("- [x] `H16-001-A01` done\n- [ ] `H16-001-A02` blocked\n", encoding="utf-8")
    problems = vp.exception_problems(tmp_path, ["H16-001-x.md"])
    assert any("without a link to EXCEPTIONS.md" in p for p in problems), (
        f"an unchecked item must be a registered exception: {problems}"
    )

    # 2. linked, but the acceptance id is not in the register
    ticket.write_text(
        "- [ ] `H16-001-A03` blocked\n  → [EXCEPTIONS.md](EXCEPTIONS.md)\n", encoding="utf-8"
    )
    problems = vp.exception_problems(tmp_path, ["H16-001-x.md"])
    assert any("H16-001-A03: unchecked but not in EXCEPTIONS.md" in p for p in problems), problems

    # 3. the register lists a row for something no ticket leaves unchecked (stale)
    ticket.write_text("- [x] `H16-001-A02` done after all\n", encoding="utf-8")
    problems = vp.exception_problems(tmp_path, ["H16-001-x.md"])
    assert any("lists 1 rows but the tickets have 0 unchecked" in p for p in problems), problems

    # consistent: one unchecked, linked, registered
    ticket.write_text(
        "- [ ] `H16-001-A02` blocked\n  → [EXCEPTIONS.md](EXCEPTIONS.md)\n", encoding="utf-8"
    )
    assert vp.exception_problems(tmp_path, ["H16-001-x.md"]) == []
