#!/usr/bin/env bash
#
# Deterministic allocation gate (ADR 9000 / P2 allocation track, P8).
#
# Thin front door over tools/bench/allocation_gate.py, which owns the parsing
# and the threshold (tools/bench/perf-gate.json). Kept as a shell entry point so
# `just bench-gate` and CI invoke the same command.
set -euo pipefail
cd "$(dirname "$0")/.."

# A threshold that is set — even to an empty string — is an explicit override
# and goes to the parser as-is. Falling back to the default on an empty value
# would turn a misconfigured override into a silently different gate.
if [ -n "${MAX_ALLOCS_PER_OP+set}" ]; then
  exec python3 tools/bench/allocation_gate.py --max-allocs-per-op="${MAX_ALLOCS_PER_OP}" "$@"
fi
exec python3 tools/bench/allocation_gate.py "$@"
