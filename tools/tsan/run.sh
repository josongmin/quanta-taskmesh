#!/usr/bin/env bash
#
# ThreadSanitizer over the concurrency tests of the PRODUCTION engine and host.
#
# loom and shuttle explore interleavings of the engine on a modelled memory
# system; TSan watches the real one: the compiled code, parking_lot, Tokio,
# OS threads. It reports data races the model checkers cannot see (a missing
# fence in an adapter, an atomic with the wrong ordering) and nothing about
# lock discipline the models already prove. The two are complementary, and
# neither is a substitute for the other.
#
# Needs the nightly toolchain with `rust-src` (`-Zbuild-std` instruments std
# too; a race between instrumented code and an uninstrumented std is invisible).
# Without it this gate reports NOT_RUN (exit 2) — never PASS.
#
# Outcomes (one machine-readable line on stdout):
#   taskmesh-tsan status=CLEAN target=<triple>
#   taskmesh-tsan status=NOT_RUN reason=<why>          (exit 2)
# A TSan report makes the test binary fail, so the gate fails with the report
# on stderr; `halt_on_error=1` stops at the first one instead of drowning it.
set -euo pipefail
cd "$(dirname "$0")/../.."

not_run() {
  echo "tsan: $1" >&2
  echo "taskmesh-tsan status=NOT_RUN reason=$2"
  exit 2
}

if ! rustup run nightly rustc -V >/dev/null 2>&1; then
  not_run "nightly toolchain not installed (rustup toolchain install nightly --component rust-src)" no-nightly
fi
if ! rustup component list --toolchain nightly 2>/dev/null | grep -q '^rust-src.*(installed)'; then
  not_run "nightly rust-src component not installed (rustup component add rust-src --toolchain nightly)" no-rust-src
fi
target="$(rustc -vV | sed -n 's/^host: //p')"
case "${target}" in
  x86_64-unknown-linux-gnu | aarch64-unknown-linux-gnu | aarch64-apple-darwin | x86_64-apple-darwin) ;;
  *) not_run "ThreadSanitizer is not supported on ${target}" "unsupported-target-${target}" ;;
esac

export RUSTFLAGS="-Zsanitizer=thread"
export RUSTDOCFLAGS="-Zsanitizer=thread"
export TSAN_OPTIONS="halt_on_error=1"

# Engine: the OS-thread stress and fuzz drivers, effect retirement (wakers fired
# outside the lock), and the ticket lifecycle races.
cargo +nightly test -Zbuild-std --target "${target}" -p taskmesh-engine \
  --test concurrency_stress \
  --test concurrency_fuzz \
  --test hardening_effect_retirement \
  --test hardening_lifecycle

# Host: Tokio workers, the blocking pool, dedicated stack threads, cancellation
# and deadline races, and the mixed soak.
cargo +nightly test -Zbuild-std --target "${target}" -p taskmesh \
  --test e2e_chaos \
  --test hardening_executor_protocol \
  --test hardening_deadline_custody \
  --test runtime_cancel_timeout \
  --test host_inferno

echo "taskmesh-tsan status=CLEAN target=${target}"
