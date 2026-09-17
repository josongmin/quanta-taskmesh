#!/usr/bin/env python3
"""Validate a `--output-format bencher` capture before it becomes a data point.

`cargo bench | tee file` can leave a partial file behind when the bench fails
mid-run (or when it never ran and only warnings were printed). A trend store
that accepts that file records a bogus point and, worse, may report a
"regression" or an "improvement" against it later. This checks that every
expected benchmark produced at least one well-formed bencher line.

    test <name> ... bench:  <int> ns/iter (+/- <int>)
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

BENCHER_LINE = re.compile(
    r"^test (?P<name>\S+) \.\.\. bench:\s+(?P<ns>[0-9,]+) ns/iter \(\+/- (?P<pm>[0-9,]+)\)\s*$"
)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("output", type=Path)
    parser.add_argument(
        "--expect",
        action="append",
        default=[],
        help="a bench group name that must appear at least once",
    )
    args = parser.parse_args(argv)

    try:
        lines = args.output.read_text(encoding="utf-8").splitlines()
    except OSError as exc:
        print(f"FAIL: cannot read {args.output}: {exc}", file=sys.stderr)
        return 1

    names: list[str] = []
    for line in lines:
        match = BENCHER_LINE.match(line)
        if match:
            names.append(match.group("name"))
    if not names:
        print(f"FAIL: {args.output} contains no bencher result lines", file=sys.stderr)
        return 1
    missing = [group for group in args.expect if not any(group in name for name in names)]
    if missing:
        print(
            f"FAIL: no result lines for expected benches {missing} (found {len(names)} lines)",
            file=sys.stderr,
        )
        return 1
    print(f"ok: {len(names)} bencher result lines covering {args.expect or 'all'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
