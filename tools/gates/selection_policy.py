"""Explicit execution consent for costly mutation gates."""
from __future__ import annotations

from collections.abc import Iterable


def profile_gate_ids(required: dict) -> dict[str, list[str]]:
    ids = required["required"]
    nightly = required["nightly_required"]
    return {
        "release": ids,
        "ci": [gate_id for gate_id in ids if gate_id not in nightly],
        "nightly": nightly,
    }


def explicit_gate_ids(required: dict) -> set[str]:
    ids = required.get("explicit_selection_required")
    required_ids = required.get("required")
    if (
        not isinstance(ids, list)
        or not ids
        or any(not isinstance(gate_id, str) for gate_id in ids)
        or len(set(ids)) != len(ids)
        or not isinstance(required_ids, list)
        or any(not isinstance(gate_id, str) for gate_id in required_ids)
        or not set(ids) <= set(required_ids)
    ):
        raise ValueError("explicit_selection_required must name unique required gates")
    return set(ids)


def require_explicit_selection(
    selected: Iterable[str],
    required: dict,
    *,
    named: Iterable[str] = (),
    include_mutation: bool = False,
) -> None:
    protected = explicit_gate_ids(required)
    blocked = sorted((set(selected) & protected) - set(named))
    if blocked and not include_mutation:
        raise ValueError(
            f"mutation gates require explicit selection: {', '.join(blocked)}; "
            "use --id <gate> or --include-mutation"
        )
