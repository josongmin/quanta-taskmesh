#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "bench-iai: requires Linux (iai-callgrind runs on valgrind; current OS: $(uname -s))" >&2
  exit 1
fi

if ! command -v valgrind >/dev/null 2>&1; then
  echo "bench-iai: requires valgrind on PATH" >&2
  exit 1
fi

if ! command -v iai-callgrind-runner >/dev/null 2>&1; then
  echo "bench-iai: requires iai-callgrind-runner v0.14.2 on PATH" >&2
  exit 1
fi

cargo bench -p taskmesh-bench --features iai --bench iai_governance "$@"
