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
#   taskmesh-fuzz status=PASS targets=<n> corpus_sha256=<sha> receipt=<path>
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
if [ "${seconds}" -eq 0 ]; then
  echo "fuzz: FUZZ_SECONDS must be greater than zero" >&2
  exit 1
fi

cd fuzz
# One target name per line; macOS ships bash 3.2, so no `mapfile`.
targets="$(cargo +nightly fuzz list)"
if [ -z "${targets}" ]; then
  echo "fuzz: no fuzz targets found — the harness is broken, not the code" >&2
  exit 1
fi
verify_args=""
for target in ${targets}; do
  verify_args="${verify_args} --actual-target ${target}"
done
# Target names are constrained by Cargo and the producer manifest. Deliberate
# word splitting keeps this compatible with macOS bash 3.2.
# shellcheck disable=SC2086
python3 ../tools/fuzz/evidence.py verify ${verify_args}

output_dir="${TASKMESH_FUZZ_OUTPUT_DIR:-../target/sep21/v03/fuzz/local}"
logs_dir="${output_dir}/raw"
seed_dir="${output_dir}/seed-corpus"
python3 ../tools/fuzz/evidence.py validate-paths \
  --output-dir "${output_dir}" \
  --logs-dir "${logs_dir}" \
  --corpus-dir "${seed_dir}"
mkdir -p "${logs_dir}"
python3 ../tools/fuzz/evidence.py prepare-corpus \
  --output-dir "${output_dir}" \
  --destination "${seed_dir}"
started_at="$(python3 -c 'from datetime import datetime, timezone; print(datetime.now(timezone.utc).isoformat())')"

process_exit=0
for target in ${targets}; do
  : >"${logs_dir}/${target}.log"
done
source_before="${output_dir}/source-before.json"
python3 ../tools/fuzz/evidence.py capture-source \
  --output-dir "${output_dir}" \
  --destination "${source_before}"
for target in ${targets}; do
  echo "fuzz: ${target} for ${seconds}s" >&2
  log="${logs_dir}/${target}.log"
  # `-max_total_time` bounds the campaign; a crash exits non-zero and
  # is retained as a raw artifact before semantic validation.
  set +e
  TASKMESH_FUZZ_WITNESS=1 cargo +nightly fuzz run "${target}" "${seed_dir}/${target}" -- -max_total_time="${seconds}" 2>&1 | tee "${log}" >&2
  target_exit="${PIPESTATUS[0]}"
  set -e
  if [ "${target_exit}" -ne 0 ]; then
    process_exit="${target_exit}"
    break
  fi
done

finished_at="$(python3 -c 'from datetime import datetime, timezone; print(datetime.now(timezone.utc).isoformat())')"
cd ..
python3 tools/fuzz/evidence.py collect \
  --logs-dir "${logs_dir#../}" \
  --output-dir "${output_dir#../}" \
  --source-before "${source_before#../}" \
  --duration-seconds "${seconds}" \
  --started-at "${started_at}" \
  --finished-at "${finished_at}" \
  --process-exit "${process_exit}"
