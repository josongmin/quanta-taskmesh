#!/usr/bin/env bash
#
# Deterministic allocation gate (ADR 9000 / P2 allocation track, P8).
#
# Runs the alloc_probe example and fails if the admit→release hot path allocates
# more heap operations per op than the committed baseline. This is a
# machine-independent, deterministic regression gate — it runs anywhere (no
# valgrind), unlike the iai-callgrind instruction-count gate (Linux-only).
#
# The behavioral/e2e gates (harness invariants + tests/hellgate.rs) run via the
# normal `cargo test --workspace`, so they are not duplicated here.
#
# Override the threshold with MAX_ALLOCS_PER_OP (default 5, the current baseline).
set -euo pipefail
cd "$(dirname "$0")/.."

MAX_ALLOCS_PER_OP="${MAX_ALLOCS_PER_OP:-5}"

echo "== allocation gate: admit+release allocs/op must be <= ${MAX_ALLOCS_PER_OP} =="
probe_out="$(cargo run -q -p taskmesh-bench --example alloc_probe --release)"
echo "${probe_out}"

per_op="$(printf '%s\n' "${probe_out}" | sed -n 's/.*allocations\/op = \([0-9.][0-9.]*\).*/\1/p')"
if [ -z "${per_op}" ]; then
  echo "FAIL: could not parse allocations/op from alloc_probe output" >&2
  exit 1
fi

awk -v v="${per_op}" -v m="${MAX_ALLOCS_PER_OP}" 'BEGIN {
  if (v + 0 > m + 0) { printf "FAIL: %s allocs/op exceeds baseline %s\n", v, m; exit 1 }
  printf "ok: %s allocs/op (<= baseline %s)\n", v, m
}'

echo "bench-gate: allocation gate passed"
