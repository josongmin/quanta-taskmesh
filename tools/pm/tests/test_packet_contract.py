"""Reject drift in on-demand packets without executing their campaign commands."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]
VALIDATOR = REPO / "docs/bugbash/sep-21/tickets/validate_plan.py"
SPEC = importlib.util.spec_from_file_location("sep21_packet_contract", VALIDATOR)
assert SPEC is not None and SPEC.loader is not None
PACKET = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKET)


def test_current_packet_structure() -> None:
    result = subprocess.run(
        [sys.executable, str(VALIDATOR), "--structure-only"],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert "source=not_checked" in result.stdout


def test_acceptance_range_matches_ticket_inventory() -> None:
    PACKET.validate_prompt_acceptance(
        Path("contract.md"),
        "C01-A01~A02, `SEP21-C01-A03`",
        {"SEP21-C01-A01", "SEP21-C01-A02", "SEP21-C01-A03"},
    )


@pytest.mark.parametrize(
    "body",
    [
        "C01-A01",
        "C01-A01~A03",
        "C01-A01~A02, E01-A01",
        "C01-A02~A01",
        "C01-A00~A02",
    ],
)
def test_missing_extra_or_invalid_acceptance_rejects(body: str) -> None:
    with pytest.raises(SystemExit, match="acceptance"):
        PACKET.validate_prompt_acceptance(
            Path("contract.md"),
            body,
            {"SEP21-C01-A01", "SEP21-C01-A02"},
        )


def test_removed_recipe_rejects_even_with_environment_prefix() -> None:
    body = "```sh\nFUZZ_SECONDS=60 just fuzz\n```\n"
    PACKET.validate_prompt_commands(Path("concurrency.md"), body, "fuzz:\n    echo fuzz\n")
    with pytest.raises(SystemExit, match="unknown Justfile recipe: fuzz"):
        PACKET.validate_prompt_commands(
            Path("concurrency.md"), body, "fuzz-check:\n    echo check\n"
        )


@pytest.mark.parametrize("case", ["valid", "stale_inventory", "typo", "duplicate_producer"])
def test_signal_inventory_and_producers_agree(case: str) -> None:
    readme = "## dependency signal 이름\n\n- `V01_SCHEMA_READY`\n"
    prompts = {Path("proof.md"): "signal: V01_SCHEMA_READY\n"}
    if case == "stale_inventory":
        readme += "- `PROOF_LANES_CLOSED`\n"
    elif case == "typo":
        prompts[Path("consumer.md")] = "Wait for V01_SCHEME_READY.\n"
    elif case == "duplicate_producer":
        prompts[Path("other.md")] = "signal: V01_SCHEMA_READY\n"
    if case == "valid":
        PACKET.validate_prompt_signals(readme, prompts)
    else:
        with pytest.raises(SystemExit, match="signal"):
            PACKET.validate_prompt_signals(readme, prompts)
