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
    uv run python tools/arch/check_crate_boundaries.py

py-lint:
    uv run ruff check tools

py-test:
    uv run pytest tools -q

# Prompt-manager drift lint on the REAL manifest (read-only). Fixture-level
# behaviour is covered by py-test; this is the actual repository state.
pm-lint:
    uv run python tools/pm/pm.py lint

# Gate inventory (H16-018): the Justfile, the CI workflows, and the required set
# must agree. Deleting a gate from one place is reported, not silently honored.
# Under `uv run`: the validator parses the workflows with PyYAML.
gates-inventory:
    uv run python tools/gates/validate_inventory.py

# Mutation gate (H16-014): reintroduce each fixed defect one at a time and
# require the named regression to reject it for the named reason. A survivor is
# a gap in the proof. See tools/verification/mutations.json.
# Under `uv run`: the inventory's `runner: pytest` entries are proved by the
# Python tooling's own tests, which need the managed environment.
mutants-critical *ARGS:
    uv run python tools/verification/run_mutations.py {{ARGS}}

lint-rust: fmt-check clippy

lint-arch: test-architecture

lint-semgrep: semgrep

lint-deps: deny

lint-py: py-lint py-test

lint-rules: semgrep test-architecture

# Feature-matrix drift: the `rayon` feature auto-wires the default CPU executor
# on a cfg-gated path that default-feature builds never compile.
test-rayon:
    cargo test -p taskmesh --features rayon
    cargo test -p taskmesh-rayon

doctest:
    cargo test --doc -p taskmesh

# rustdoc must stay link-clean (private-intra-doc-link drift detector).
rustdoc:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace

# Benches must keep compiling and smoke-running (no timing is recorded).
bench-smoke:
    cargo bench -p taskmesh-bench -- --test

# Consumer MSRV (H16-019): compile a minimal consumer OUTSIDE this workspace on
# the declared `rust-version`, with the default and the `rayon` public surface.
# The workspace's own dev/bench dependency graph does not resolve on that
# toolchain (and does not have to); this proves the *library* contract instead.
consumer-msrv:
    python3 tools/consumer-msrv/check.py

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
    RUSTFLAGS="--cfg loom" cargo test -p taskmesh-engine --features loom --test loom_governance --release

# Fast local front door: fmt, clippy, unit/integration tests, supply-chain,
# semgrep, architecture, python, and the deterministic allocation gate. This is
# the *fast* proof surface — it intentionally does NOT run the feature matrix
# (`just matrix`) or the heavier rails CI enforces (bench-iai instruction-count,
# loom + shuttle model-checks, mutation gate, consumer MSRV). Run `just proof`
# to exercise those locally before relying on a green `gate`.
gate: fmt-check clippy test deny semgrep test-architecture py-lint py-test pm-lint bench-gate gates-inventory
    @echo "gate: fast local checks passed (CI additionally runs: matrix [test-rayon doctest rustdoc bench-smoke consumer-msrv], mutants-critical, loom, shuttle, bench-iai — see 'just proof')"

# The CI proof-matrix job: feature-matrix drift, doctests, link-clean rustdoc,
# bench compilation, and the consumer-MSRV build. Chained by `proof`.
matrix: test-rayon doctest rustdoc bench-smoke consumer-msrv
    @echo "matrix: feature-matrix checks passed"

# Exhaustive concurrency model-check with shuttle's randomized scheduler (ADR
# 9000 / P6). Heavier than loom; matches the CI `shuttle` rail.
shuttle:
    RUSTFLAGS="--cfg shuttle" cargo test -p taskmesh-engine --features shuttle --test shuttle_governance --release

# Full proof surface: every gate in tools/gates/required.json — `gate`, the
# feature `matrix`, and the heavy rails — so a green `proof` locally is the
# same set of checks CI requires. tools/gates/validate_inventory.py verifies
# that this chain expands to exactly the required set; a gate added to CI
# without being added here fails `gates-inventory`.
proof: gate matrix mutants-critical loom shuttle bench-iai
    @echo "proof: full proof surface passed (matches CI required rails)"
