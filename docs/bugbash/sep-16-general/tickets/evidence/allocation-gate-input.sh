#!/usr/bin/env bash
# Observation test: calls the actual gate with controlled cargo stdout/status.
# Exit 0 proves CURRENT false-green behavior, not corrected fail-closed behavior.
set -euo pipefail
repo_root="$(cd "$(dirname "$0")/../../../../.." && pwd)"

cargo() {
  case "${TASKMESH_AUDIT_ALLOC_CASE}" in
    valid_below) printf 'allocations/op = 3.000\n' ;;
    valid_above) printf 'allocations/op = 5.000\n' ;;
    malformed) printf 'allocations/op = ...\n' ;;
    partial_number) printf 'allocations/op = 3.0.0\n' ;;
    missing) printf 'no allocation measurement\n' ;;
    failed_probe) return 17 ;;
    *) return 99 ;;
  esac
}
export -f cargo

for scenario in valid_below valid_above malformed partial_number missing failed_probe; do
  status=0
  output="$(TASKMESH_AUDIT_ALLOC_CASE="$scenario" MAX_ALLOCS_PER_OP=4 \
    bash "$repo_root/tools/bench-gate.sh" 2>&1)" || status=$?
  printf '%s: exit=%s\n%s\n' "$scenario" "$status" "$output"
  case "$scenario" in
    valid_below|malformed|partial_number) expected=0 ;;
    valid_above|missing) expected=1 ;;
    failed_probe) expected=17 ;;
  esac
  if [ "$status" -ne "$expected" ]; then
    printf 'Unexpected observation: expected exit %s, got %s\n' "$expected" "$status" >&2
    exit 1
  fi
done
