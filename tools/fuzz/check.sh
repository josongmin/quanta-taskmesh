#!/usr/bin/env bash
#
# Type-check every fuzz target on the stable toolchain (part of `just gate`).
#
# The fuzzing run itself needs nightly + cargo-fuzz (`just fuzz`); this keeps
# the targets compiling against the current engine/host API in between, so a
# renamed outcome or a changed signature is caught by the fast gate rather
# than by the next nightly campaign. `--all-targets` is what makes every
# `[[bin]]` target part of the check; `-D warnings` because a warning in a
# harness is usually a dropped `#[must_use]` outcome.
set -euo pipefail
cd "$(dirname "$0")/../../fuzz"
cargo check --all-targets --locked
RUSTFLAGS="-D warnings" cargo clippy --all-targets --locked -- -D warnings -D clippy::pedantic \
  -A clippy::cast_possible_truncation -A clippy::cast_precision_loss -A clippy::too_many_lines \
  -A clippy::module_name_repetitions -A clippy::missing_panics_doc
