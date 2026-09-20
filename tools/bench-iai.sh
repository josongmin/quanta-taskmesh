#!/usr/bin/env bash
#
# Instruction-count gate (ADR 9000 / P2). Linux + valgrind + iai-callgrind-runner.
#
# Thresholds, the runner version, and the set of inputs that define baseline
# compatibility all come from tools/bench/perf-gate.json. Local runs and CI read
# the same file, so "full proof matches CI" is true by construction rather than
# by remembering to set the same environment variable in two places.
#
# Usage:
#   tools/bench-iai.sh                 run the gate
#   tools/bench-iai.sh fingerprint     print the baseline-compatibility fingerprint and exit
#                                      (CI keys its baseline cache on this)
# No other arguments are accepted: a stray word would otherwise reach
# `cargo bench` as a *filter*, and a run that measured nothing would stamp a
# baseline and report QUALIFIED.
#
# Outcomes (one machine-readable line on stdout):
#
#   taskmesh-iai-gate status=QUALIFIED        fingerprint=<hex>
#       A compatible baseline existed and the run was compared against it. A
#       regression beyond the configured threshold makes the bench itself fail.
#   taskmesh-iai-gate status=BASELINE_CREATED fingerprint=<hex>
#       No compatible baseline existed (fresh cache, or the bench definition,
#       dependencies, toolchain, or runner changed). This run recorded one. It
#       is NOT a regression-qualified pass and must not be reported as one.
#
# An incompatible cached baseline is discarded rather than compared against:
# comparing instruction counts across a redefined measurement is how a real
# regression hides behind a definitional change (or a phantom one appears).
#
# Every early exit prints a `bench-iai:` diagnostic. This script never dies
# silently: a prerequisite that is missing, a runner whose version cannot be
# read, or a benchmark that fails all say so on stderr.
set -euo pipefail
cd "$(dirname "$0")/.."

CONFIG="tools/bench/perf-gate.json"
HELPER="tools/bench/iai_gate.py"

fail() {
  echo "bench-iai: $*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || fail "requires $1 on PATH"
}
need python3
[[ -f "${HELPER}" ]] || fail "helper ${HELPER} is missing"

read_cfg() {
  python3 "${HELPER}" config "$1"
}

BENCH="$(read_cfg bench)"
RUNNER_VERSION="$(read_cfg iai_callgrind_runner)"
REGRESSION="$(read_cfg regression)"
SCHEMA="$(read_cfg measurement_schema)"

case "$#:${1:-}" in
  0:) mode="run" ;;
  1:fingerprint) mode="fingerprint" ;;
  *) fail "unexpected argument(s): '$*' (accepted: none, or 'fingerprint')" ;;
esac

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "bench-iai: requires Linux (iai-callgrind runs on valgrind; current OS: $(uname -s))" >&2
  echo "taskmesh-iai-gate status=UNSUPPORTED_PLATFORM os=$(uname -s)"
  exit 1
fi
need valgrind
need iai-callgrind-runner
need cargo
need rustc

# The runner has no `--version`; the helper knows the two places it does say
# its version and reports which one answered (or why neither did).
if ! installed_runner="$(python3 "${HELPER}" runner-version)"; then
  fail "could not determine the installed iai-callgrind-runner version (see above); config requires ${RUNNER_VERSION}"
fi
if [[ "${installed_runner}" != "${RUNNER_VERSION}" ]]; then
  fail "iai-callgrind-runner ${installed_runner} on PATH; config requires ${RUNNER_VERSION} (cargo install iai-callgrind-runner --version ${RUNNER_VERSION} --locked)"
fi

# Baseline compatibility fingerprint: the measured definition, its dependency
# graph, the toolchain that compiled it, and the runner.
rustc_probe="$(mktemp)"
trap 'rm -f "${rustc_probe}"' EXIT
rustc -Vv >"${rustc_probe}"
[[ -s "${rustc_probe}" ]] || fail "rustc -Vv printed nothing; the fingerprint cannot name the compiler"
# The fingerprint's inputs must all be present: a valgrind that prints nothing
# (or only to stderr) would hash to the same baseline as any other valgrind.
valgrind_version="$(valgrind --version 2>/dev/null || true)"
[[ -n "${valgrind_version}" ]] || fail "valgrind --version printed nothing on stdout; the fingerprint cannot name valgrind"
fingerprint="$(python3 "${HELPER}" fingerprint \
  --runner "${RUNNER_VERSION}" \
  --valgrind "${valgrind_version}" \
  --rustc-file "${rustc_probe}")" || fail "could not compute the baseline fingerprint"

if [[ "${mode}" == "fingerprint" ]]; then
  echo "${fingerprint}"
  exit 0
fi

if [[ -n "${IAI_CALLGRIND_REGRESSION:-}" && "${IAI_CALLGRIND_REGRESSION}" != "${REGRESSION}" ]]; then
  echo "bench-iai: IAI_CALLGRIND_REGRESSION=${IAI_CALLGRIND_REGRESSION} in the environment disagrees with ${CONFIG} (${REGRESSION}); the config wins" >&2
fi
export IAI_CALLGRIND_REGRESSION="${REGRESSION}"

baseline_dir="target/iai"
status="BASELINE_CREATED"
expected_comparison="no"
if [[ -d "${baseline_dir}" ]]; then
  if python3 "${HELPER}" validate-baseline --root "${baseline_dir}" \
    --fingerprint "${fingerprint}" --runner "${RUNNER_VERSION}" \
    --valgrind "${valgrind_version}" --rustc-file "${rustc_probe}"; then
    expected_comparison="yes"
  else
    echo "bench-iai: cached baseline is incomplete, corrupt, or incompatible; discarding it" >&2
    rm -rf "${baseline_dir}"
  fi
fi
mkdir -p "${baseline_dir}"

# Summary files are run outputs, not baseline inputs. Removing them after the
# baseline was verified prevents a stale summary from claiming this run made a
# comparison. Raw *.out/*.log baseline data remains in place for the runner.
find "${baseline_dir}" -type f -name summary.json -delete
rm -f "${baseline_dir}/comparison-manifest.json" "${baseline_dir}/benchmark-output.log"
export IAI_CALLGRIND_SAVE_SUMMARY=yes
export IAI_CALLGRIND_COLOR=never

# Raw output is kept even when cargo fails. It has no baseline authority
# without a verified baseline manifest, but remains available for diagnosis.
if ! cargo bench -p taskmesh-bench --features iai --bench "${BENCH}" 2>&1 \
  | tee "${baseline_dir}/benchmark-output.log"; then
  fail "benchmark ${BENCH} failed; no evidence manifest was accepted"
fi

if ! status="$(python3 "${HELPER}" finalize --root "${baseline_dir}" \
  --fingerprint "${fingerprint}" --runner "${RUNNER_VERSION}" \
  --valgrind "${valgrind_version}" --rustc-file "${rustc_probe}" \
  --expected-comparison "${expected_comparison}")"; then
  fail "runner exit 0 did not produce complete raw/summary comparison evidence"
fi

echo "taskmesh-iai-gate status=${status} fingerprint=${fingerprint} regression=${REGRESSION} schema=${SCHEMA}"
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  {
    echo "status=${status}"
    echo "fingerprint=${fingerprint}"
  } >>"${GITHUB_OUTPUT}"
fi
if [[ "${status}" != "QUALIFIED" ]]; then
  echo "bench-iai: NOT regression-qualified — this run recorded a fresh baseline; a later run against it can qualify" >&2
fi
