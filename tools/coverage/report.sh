#!/usr/bin/env bash
#
# Line/region coverage REPORT for the library crates (cargo-llvm-cov).
#
# This is a report, not a threshold. The repository makes no coverage-gate
# promise (see tools/semgrep/rules/test-quality.yml header): a percentage says
# which lines a test *executed*, not which behaviours a test *constrains* —
# that is the mutation gate's job. The numbers are recorded in the receipt so a
# reviewer can see uncovered production lines and decide whether they are
# unreachable-by-design or a gap; they never turn a run red on their own.
#
# Needs cargo-llvm-cov. Without it this gate reports NOT_RUN (exit 2), not PASS.
#
# Outcome (one machine-readable line on stdout):
#   taskmesh-coverage status=REPORTED lines=<pct> regions=<pct> functions=<pct> report=<path>
#   taskmesh-coverage status=NOT_RUN reason=<why>        (exit 2)
set -euo pipefail
cd "$(dirname "$0")/../.."

if ! cargo llvm-cov --version >/dev/null 2>&1; then
  echo "coverage: cargo-llvm-cov not installed (cargo install cargo-llvm-cov --locked)" >&2
  echo "taskmesh-coverage status=NOT_RUN reason=no-cargo-llvm-cov"
  exit 2
fi

out_dir="target/coverage"
mkdir -p "${out_dir}"
# Start from this build only. `cargo llvm-cov` reports every instrumented
# object left in its target dir, and a test executable built from an older
# source — here once a doc-examples binary from before that crate was
# excluded — carries its own copy of the library with the *old* line mapping,
# never executed. Merged in, it reported comment lines as uncovered and pulled
# the totals down by 20 points. Cleaning the workspace's artifacts and
# profiles (dependencies stay built) makes the report a function of the source.
cargo llvm-cov clean --workspace
# The bench harness measures and the doc-examples crate only type-checks the
# README's fragments (never executed); neither is the library, and counting
# them would move the numbers without a single library line changing.
cargo llvm-cov --workspace --exclude taskmesh-bench --exclude taskmesh-doc-examples --all-targets \
  --json --summary-only --output-path "${out_dir}/summary.json" >/dev/null
cargo llvm-cov report --lcov --output-path "${out_dir}/lcov.info" >/dev/null

python3 - "${out_dir}/summary.json" <<'PY'
import json, sys
path = sys.argv[1]
totals = json.load(open(path))["data"][0]["totals"]
def pct(key):
    value = totals[key]["percent"]
    return f"{value:.2f}"
print(
    f"taskmesh-coverage status=REPORTED lines={pct('lines')} regions={pct('regions')} "
    f"functions={pct('functions')} report={path}"
)
PY
