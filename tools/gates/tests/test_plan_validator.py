"""The compact implementation ADR must retain every ticket's content row."""

from __future__ import annotations

import importlib.util
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]
VALIDATOR = REPO / "docs/plans/bugbash-sep-25-general/tickets/validate_plan.py"
SPEC = importlib.util.spec_from_file_location("validate_plan", VALIDATOR)
assert SPEC is not None and SPEC.loader is not None
VALIDATE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATE)


def test_compact_adr_has_one_complete_row_per_ticket() -> None:
    rows = VALIDATE.implementation_rows(VALIDATE.ADR.read_text())
    assert rows == {f"BG25-{number:03}" for number in range(1, 13)}


@pytest.mark.parametrize(
    "text",
    [
        "| BG25-001 | | 0004 |",
        "| BG25-001 | Boundary | |",
        "| BG25-001 | Boundary | Authority |\n| BG25-001 | Other | Authority |",
    ],
)
def test_empty_or_duplicate_implementation_rows_reject(text: str) -> None:
    with pytest.raises(ValueError, match="implementation ADR row"):
        VALIDATE.implementation_rows(text)
