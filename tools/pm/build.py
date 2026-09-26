#!/usr/bin/env python3
"""Explicitly synchronize managed router blocks after validating every candidate."""

from __future__ import annotations

import sys

import check


def main() -> int:
    try:
        changed = check.build()
    except check.PolicyError as exc:
        print(f"prompt-build: FAIL: {exc}", file=sys.stderr)
        return 1
    print(f"prompt-build: PASS: updated={len(changed)}")
    for path in changed:
        print(f"- {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
