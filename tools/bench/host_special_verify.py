#!/usr/bin/env python3
"""Recheck a retained special host diagnostic without a current-source claim."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import host_perf
from host_special_run import verify_receipt


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--require-current-source", action="store_true")
    args = parser.parse_args()
    try:
        receipt = verify_receipt(args.directory, require_current_source=args.require_current_source)
    except (OSError, host_perf.ReceiptError) as error:
        print(f"special receipt rejected: {error}", file=sys.stderr)
        return 1
    print(f"SPECIAL_STRUCTURALLY_VALID performance=UNQUALIFIED mode={receipt['mode']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
