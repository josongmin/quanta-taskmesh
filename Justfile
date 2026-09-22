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

# Gate inventory (H16-018): the Justfile, the manual fallback workflows, and the
# required set must agree. The validator also rejects automatic hosted triggers,
# so a push, pull request, or schedule cannot silently start spending minutes.
gates-inventory:
    uv run python tools/gates/validate_inventory.py

# Mutation gate (H16-014): reintroduce each fixed defect one at a time and
# require the named regression to reject it for the named reason. A survivor is
# a gap in the proof. See tools/verification/mutations.json.
# Under `uv run`: the inventory's `runner: pytest` entries are proved by the
# Python tooling's own tests, which need the managed environment.
mutants-critical *ARGS:
    uv run python tools/verification/run_mutations.py {{ARGS}}

# Generated cargo-mutants campaign. This denominator is intentionally separate
# from the curated single-edit inventory above; a subset or survivor is FAIL,
# never evidence for the curated gate.
mutants-generated *ARGS:
    uv run python tools/verification/run_generated_mutants.py --jobs 2 {{ARGS}}

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
# (`just matrix`) or the heavier local rails (bench-iai instruction-count, loom
# + shuttle/model replay checks, both mutation campaigns, consumer MSRV). Run
# `just verify-local` for the complete required set.
gate: fmt-check clippy test deny semgrep test-architecture py-lint py-test pm-lint bench-gate gates-inventory fuzz-check
    @echo "gate: fast local checks passed; run 'just verify-local' for the complete required set"

# The libFuzzer targets (fuzz/) type-check and lint on stable, so they cannot
# rot between nightly fuzzing campaigns (`just fuzz`).
fuzz-check:
    bash tools/fuzz/check.sh

# Local proof matrix: feature-matrix drift, doctests, link-clean rustdoc, bench
# compilation, and the consumer-MSRV build. Chained by `proof`.
matrix: test-rayon doctest rustdoc bench-smoke consumer-msrv
    @echo "matrix: feature-matrix checks passed"

# Exhaustive concurrency model-check with shuttle's randomized scheduler (ADR
# 9000 / P6). Heavier than loom; matches the CI `shuttle` rail.
shuttle:
    RUSTFLAGS="--cfg shuttle" cargo test -p taskmesh-engine --features shuttle --test shuttle_governance --release

# ThreadSanitizer over the production engine and host concurrency tests
# (nightly + rust-src; NOT_RUN/exit 2 without them — never PASS). Complements
# loom/shuttle: they explore interleavings of a modelled memory system, TSan
# watches the real one.
tsan:
    bash tools/tsan/run.sh

# Coverage-guided fuzzing of the production engine, the host builder and the
# wire formats (nightly + cargo-fuzz; NOT_RUN/exit 2 without them — never
# PASS). FUZZ_SECONDS bounds each target (default 30).
fuzz:
    bash tools/fuzz/run.sh

# V03 owns the bounded Loom + Shuttle + clean-process replay producer. The
# receipt consumes its EvidenceEnvelopeV1; it does not parse the raw logs.
modelcheck:
    python3 tools/modelcheck/run.py all

# Coverage REPORT (cargo-llvm-cov). Numbers for the receipt; never a threshold —
# this repository makes no coverage-gate promise. NOT_RUN/exit 2 without the tool.
coverage-report:
    bash tools/coverage/report.sh

# Release-only compatibility audit against the immutable 0.2.0 release commit.
# This is not in ordinary `proof`: a current-source qualification receipt and
# explicit API/wire/behavior adjudication are separate release inputs.
semver-release:
    python3 tools/release/semver.py --out target/release/semver

qualify-local:
    uv run python tools/qualification/receipt.py collect --local-qualified --out target/qualification/local-receipt.json

validate-local-qualification:
    uv run python tools/qualification/receipt.py validate target/qualification/local-receipt.json

release-finding-proof:
    python3 tools/release/finding_proof.py --out target/release/finding-proof

release-receipt: validate-local-qualification semver-release release-finding-proof
    python3 tools/release/receipt.py collect --local-qualified --ordinary target/qualification/local-receipt.json --semver target/release/semver/semver-manifest.json --adjudication target/release/input/adjudication.json --out target/release/release-receipt.json

release-local: release-receipt
    @echo "release-local: exact-source local release receipt completed"

# Full local proof surface: every gate in tools/gates/required.json — `gate`, the
# feature `matrix`, and the heavy rails. tools/gates/validate_inventory.py
# verifies that this chain expands to exactly the required set.
proof: gate matrix mutants-critical mutants-generated loom shuttle modelcheck tsan fuzz coverage-report bench-iai
    @echo "proof: full local required proof surface passed"

# macOS verification is receipt-backed: every required gate applicable to this
# host must pass. Linux-only gates remain recorded as SKIPPED_PLATFORM and are
# never promoted into the cross-platform `QUALIFIED` verdict.
verify-macos:
    uv run python tools/gates/run.py --required --allow-platform-skips --receipt target/verification/macos-gates.json

# Canonical macOS verification entry point. Keep `proof` for the exact
# cross-platform required-set expansion checked by gates-inventory.
verify-local: verify-macos
    @echo "verify-local: macOS receipt written to target/verification/macos-gates.json"

# Install the tracked fail-closed push admission hook for this clone. The hook
# accepts a branch update only when the saved local receipt matches the exact
# clean commit being pushed. Git's explicit --no-verify remains the emergency
# override and must be disclosed when used.
install-hooks:
    git config --local core.hooksPath .githooks
    @echo "hooks: installed .githooks (pre-push requires current local receipt)"

validate-local-receipt:
    uv run python tools/gates/run.py --validate-receipt target/verification/macos-gates.json
