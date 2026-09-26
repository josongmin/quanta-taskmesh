#!/usr/bin/env bash
#
# Deterministic allocation gate (ADR 9000 / P2 allocation track, P8).
#
# Thin front door over tools/bench/allocation_gate.py, which owns the parsing
# and the threshold (tools/bench/perf-gate.json). Kept as a shell entry point so
# `just bench-gate` and CI invoke the same command.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ -n "${MAX_ALLOCS_PER_OP+set}" ]; then
  echo "bench-gate: MAX_ALLOCS_PER_OP cannot override the reviewed threshold" >&2
  exit 2
fi
exec uv run python tools/bench/allocation_gate.py "$@"
