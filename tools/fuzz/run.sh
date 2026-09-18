#!/usr/bin/env bash
#
# Coverage-guided fuzzing of the production engine, the host builder and the
# wire formats (`fuzz/fuzz_targets/*.rs`, ADR 0003 D16).
#
# proptest (`differential_model.rs`) and the model checkers generate inputs
# blind; libFuzzer steers them by the branches they reach. Each target asserts
# the published invariants after every step, and the engine's own
# `debug_assert` consistency checks stay armed (see `fuzz/Cargo.toml`), so a
# finding is a panic with the failing input saved under `fuzz/artifacts/`.
#
# Needs the nightly toolchain and `cargo-fuzz` (`cargo install cargo-fuzz`).
# Without them this gate reports NOT_RUN (exit 2) — never PASS. The targets
# themselves type-check on stable in the fast gate (`just fuzz-check`), so they
# cannot rot between fuzzing runs.
#
# Outcomes (one machine-readable line on stdout):
#   taskmesh-fuzz status=CLEAN targets=<n> seconds_per_target=<s> runs=<total>
#   taskmesh-fuzz status=NOT_RUN reason=<why>                       (exit 2)
# A crash makes `cargo fuzz run` exit non-zero, so the gate fails with the
# reproducer path on stderr.
#
# FUZZ_SECONDS bounds each target's wall time (default 30); a longer local
# campaign is `FUZZ_SECONDS=600 just fuzz`.
set -euo pipefail
cd "$(dirname "$0")/../.."

not_run() {
  echo "fuzz: $1" >&2
  echo "taskmesh-fuzz status=NOT_RUN reason=$2"
  exit 2
}

if ! rustup run nightly rustc -V >/dev/null 2>&1; then
  not_run "nightly toolchain not installed (rustup toolchain install nightly)" no-nightly
fi
if ! cargo +nightly fuzz --version >/dev/null 2>&1; then
  not_run "cargo-fuzz not installed (cargo install cargo-fuzz)" no-cargo-fuzz
fi

seconds="${FUZZ_SECONDS:-30}"
case "${seconds}" in
  ''|*[!0-9]*) echo "fuzz: FUZZ_SECONDS must be a whole number of seconds, got '${seconds}'" >&2; exit 1 ;;
esac

cd fuzz
# One target name per line; macOS ships bash 3.2, so no `mapfile`.
targets="$(cargo +nightly fuzz list)"
if [ -z "${targets}" ]; then
  echo "fuzz: no fuzz targets found — the harness is broken, not the code" >&2
  exit 1
fi
target_count="$(printf '%s\n' "${targets}" | wc -l | tr -d ' ')"

total_runs=0
for target in ${targets}; do
  echo "fuzz: ${target} for ${seconds}s" >&2
  log="$(mktemp)"
  # `-max_total_time` bounds the campaign; a crash exits non-zero and
  # `set -e` ends the gate with libFuzzer's report and reproducer path.
  cargo +nightly fuzz run "${target}" -- -max_total_time="${seconds}" 2>&1 | tee "${log}" >&2
  runs="$(sed -n 's/^Done \([0-9]*\) runs in .*/\1/p' "${log}" | tail -n 1)"
  rm -f "${log}"
  if [ -z "${runs}" ]; then
    echo "fuzz: ${target} finished without libFuzzer's run summary — refusing to report CLEAN" >&2
    exit 1
  fi
  total_runs=$((total_runs + runs))
done

echo "taskmesh-fuzz status=CLEAN targets=${target_count} seconds_per_target=${seconds} runs=${total_runs}"
