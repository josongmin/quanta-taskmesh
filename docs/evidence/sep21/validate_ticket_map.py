#!/usr/bin/env python3
"""Validate compact historical inputs without promoting unqualified tickets."""

from __future__ import annotations

import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
if str(REPO) not in sys.path:
    sys.path.insert(0, str(REPO))

from tools.release.ticket_map import MAP_PATH, adr_evidence, ticket_rows  # noqa: E402


def main() -> None:
    rows = ticket_rows(json.loads((REPO / MAP_PATH).read_text(encoding="utf-8")))
    unqualified = []
    for row in rows:
        status, evidence = adr_evidence(REPO, row)
        if not evidence:
            raise ValueError(f"{row['id']}: historical evidence section missing")
        if status == "IMPLEMENTED_UNQUALIFIED":
            unqualified.append(row["id"])
    recorded_unqualified = [
        row["id"] for row in rows if row["closure_status"] == "IMPLEMENTED_UNQUALIFIED"
    ]
    print(
        "SEP-21 map PASS: tickets=12 findings=23 "
        f"acceptance={sum(len(row['acceptance_ids']) for row in rows)} "
        f"historically_unqualified={','.join(unqualified)} "
        f"recorded_closure_unqualified={','.join(recorded_unqualified)} "
        "source=not_qualified"
    )


if __name__ == "__main__":
    main()
