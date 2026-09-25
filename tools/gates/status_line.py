"""Single authority for self-reported gate verdict lines and saved PASS rows."""

from __future__ import annotations

import json
from pathlib import Path


def final_status_line(marker: str, stdout: str) -> str | None:
    """Keep the final marker line; earlier per-surface lines are not the verdict."""
    lines = [line for line in stdout.splitlines() if line.startswith(marker + " ")]
    return lines[-1] if lines else None


def status_line_qualifies(gate: dict, stdout: str) -> bool:
    """Require exactly one status token on the gate's final marker line."""
    spec = gate.get("status_line")
    if spec is None:
        return True
    if (
        not isinstance(spec, dict)
        or not isinstance(spec.get("marker"), str)
        or not isinstance(spec.get("require"), str)
        or not spec["require"].startswith("status=")
    ):
        return False
    verdict = final_status_line(spec["marker"], stdout)
    if verdict is None:
        return False
    status_tokens = [token for token in verdict.split() if token.startswith("status=")]
    if status_tokens != [spec["require"]]:
        return False
    if gate.get("id") not in {"test", "py-test"}:
        return True
    fields: dict[str, str] = {}
    for token in verdict.split()[1:]:
        if "=" not in token:
            return False
        key, value = token.split("=", 1)
        if key in fields or not value:
            return False
        fields[key] = value
    if gate.get("id") == "test":
        return (
            set(fields)
            == {
                "status",
                "runner",
                "runner_version",
                "targets",
                "selected",
                "passed",
                "catalog_digest",
                "selection_digest",
                "execution_digest",
            }
            and fields["runner"] == "nextest"
            and fields["runner_version"] == "0.9.104"
            and fields["targets"].isdigit()
            and fields["selected"].isdigit()
            and fields["passed"] == fields["selected"]
            and int(fields["targets"]) > 0
            and int(fields["selected"]) > 0
            and all(
                len(fields[key]) == 64 and all(char in "0123456789abcdef" for char in fields[key])
                for key in ("catalog_digest", "selection_digest", "execution_digest")
            )
        )
    return (
        set(fields)
        == {
            "status",
            "runner",
            "runner_version",
            "modules",
            "selected",
            "passed",
            "excluded",
            "slow",
            "qualification",
            "catalog_digest",
            "selection_digest",
            "execution_digest",
        }
        and fields["runner"] == "pytest"
        and all(
            fields[key].isdigit()
            for key in ("modules", "selected", "passed", "excluded", "slow", "qualification")
        )
        and int(fields["modules"]) > 0
        and int(fields["selected"]) > 0
        and int(fields["passed"]) > 0
        and int(fields["passed"]) + int(fields["excluded"]) == int(fields["selected"])
        and all(
            len(fields[key]) == 64 and all(char in "0123456789abcdef" for char in fields[key])
            for key in ("catalog_digest", "selection_digest", "execution_digest")
        )
    )


def pass_status_line_problems(
    results: list[dict], inventory_path: Path, *, allow_imported_producers: bool = False
) -> list[str]:
    """Reject a saved PASS that contradicts its retained self-report line."""
    try:
        gates = json.loads(inventory_path.read_text(encoding="utf-8"))["gates"]
        if not isinstance(gates, list):
            raise ValueError("gates is not a list")
        by_id = {gate["id"]: gate for gate in gates}
        if len(by_id) != len(gates):
            raise ValueError("gate ids are duplicated")
    except (OSError, json.JSONDecodeError, KeyError, TypeError, ValueError) as exc:
        return [f"status-line inventory cannot be loaded: {exc}"]
    problems = []
    for result in results:
        if result.get("status") != "PASS":
            continue
        gate_id = result["id"]
        gate = by_id.get(gate_id)
        if gate is None:
            problems.append(f"gate {gate_id} has no inventory entry")
            continue
        if (
            allow_imported_producers
            and result.get("imported_producer") is True
            and "producer" in gate
        ):
            # The final collector validates this row against the registered
            # producer envelope instead of a locally captured status line.
            continue
        if "status_line" not in gate:
            continue
        line = result.get("status_line")
        if (
            not isinstance(line, str)
            or "\n" in line
            or "\r" in line
            or not status_line_qualifies(gate, line)
        ):
            problems.append(f"gate {gate_id} PASS lacks an unambiguous required status line")
    return problems
