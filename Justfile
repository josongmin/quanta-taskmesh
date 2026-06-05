# taskmesh — local build, test, and lint front door.

set shell := ["bash", "-c"]

root := justfile_directory()

clippy_allows := `grep -v '^#' config/clippy-allows.txt | grep -v '^$' | tr '\n' ' '`
clippy_restrict := `grep -v '^#' config/clippy-restrict.txt | grep -v '^$' | tr '\n' ' '`
clippy_strict := "-D warnings -D clippy::pedantic -D clippy::nursery"

default:
    @just --list

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

check:
    cargo check --workspace --all-targets

test:
    cargo test --workspace

# Pass 1: all shipped targets — pedantic + nursery, test-friendly allows.
# Pass 2: production (--lib --bins) — restriction lints from config/clippy-restrict.txt.
# Pass 3: the publish=false bench harness (statistical float code) gets baseline
#         lint only — pedantic float/cast/flop lints are noise for stats code.
clippy:
    CLIPPY_CONF_DIR={{root}}/config cargo clippy --workspace --exclude taskmesh-bench --all-targets -- {{clippy_strict}} {{clippy_allows}}
    CLIPPY_CONF_DIR={{root}}/config cargo clippy --workspace --exclude taskmesh-bench --lib --bins -- {{clippy_strict}} {{clippy_allows}} {{clippy_restrict}}
    cargo clippy -p taskmesh-bench --all-targets -- -D warnings

deny:
    cargo deny check --config config/deny.toml

semgrep:
    semgrep --config tools/semgrep/rules --error crates

test-architecture:
    python3 tools/arch/check_crate_boundaries.py

py-lint:
    uv run ruff check tools

py-test:
    uv run pytest tools -q

lint-rust: fmt-check clippy

lint-arch: test-architecture

lint-semgrep: semgrep

lint-deps: deny

lint-py: py-lint py-test

lint-rules: semgrep test-architecture

# Run the criterion wall-clock benches (informational; not a gate).
bench:
    cargo bench -p taskmesh-bench --benches

# Deterministic allocation gate (ADR 9000 / P2). Runs anywhere — no valgrind.
bench-gate:
    bash tools/bench-gate.sh

# Instruction-count gate (ADR 9000 / P2). Needs Linux + valgrind + the matching
# iai-callgrind-runner (`cargo install iai-callgrind-runner --version 0.14.2`).
bench-iai:
    bash tools/bench-iai.sh

# Exhaustive concurrency model-check of the governance design (ADR 9000 / P6).
loom:
    RUSTFLAGS="--cfg loom" cargo test -p taskmesh-engine --test loom_governance --release

# Fast local front door: fmt, clippy, unit/integration tests, supply-chain,
# semgrep, architecture, python, and the deterministic allocation gate. This is
# the *fast* proof surface — it intentionally does NOT run the heavier rails CI
# enforces (bench-iai instruction-count, loom + shuttle concurrency model-checks).
# Run `just proof` to exercise those locally before relying on a green `gate`.
gate: fmt-check clippy test deny semgrep test-architecture py-lint py-test bench-gate
    @echo "gate: fast local checks passed (CI additionally runs: bench-iai, loom, shuttle — see 'just proof')"

# Exhaustive concurrency model-check with shuttle's randomized scheduler (ADR
# 9000 / P6). Heavier than loom; matches the CI `shuttle` rail.
shuttle:
    RUSTFLAGS="--cfg shuttle" cargo test -p taskmesh-engine --test shuttle_governance --release

# Full proof surface: everything `gate` runs PLUS the heavy rails CI enforces, so
# a green `proof` locally matches CI's required checks (no local/CI proof gap).
proof: gate bench-iai loom shuttle
    @echo "proof: full proof surface passed (matches CI required rails)"
