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
    # Keep the rayon-enabled documentation fixture from feature-unifying the
    # default taskmesh build. Validate each surface explicitly.
    cargo check --locked --workspace --exclude taskmesh-doc-examples --all-targets
    cargo check --locked -p taskmesh --features rayon --all-targets
    cargo check --locked -p taskmesh-doc-examples --all-targets

test:
    # nextest keeps Cargo's --workspace --lib --tests selection but schedules
    # independent test binaries concurrently. Four tests keep a 16-core Mac
    # busy without unbounded fan-out: integration cases create Tokio/OS workers,
    # so an unbounded process fan-out just trades elapsed time for contention.
    # A clean bootstrap without cargo-nextest retains the identical Cargo
    # selection rather than silently skipping the test gate.
    set -euo pipefail; \
    cargo_version="$(cargo --version)"; cargo_version="${cargo_version#cargo }"; cargo_version="${cargo_version%% *}"; \
    if nextest_identity="$(cargo nextest --version 2>/dev/null)"; then \
        runner=nextest; runner_version="${nextest_identity#cargo-nextest }"; runner_version="${runner_version%% *}"; \
        CARGO_BUILD_JOBS="${TASKMESH_BUILD_JOBS:-4}" cargo nextest run --locked --workspace --exclude taskmesh-doc-examples --lib --tests --test-threads "${TASKMESH_TEST_JOBS:-4}"; \
    else \
        runner=cargo; runner_version="${cargo_version}"; \
        CARGO_BUILD_JOBS="${TASKMESH_BUILD_JOBS:-4}" cargo test --locked --workspace --exclude taskmesh-doc-examples --lib --tests -- --test-threads "${TASKMESH_TEST_JOBS:-4}"; \
    fi; \
    CARGO_BUILD_JOBS="${TASKMESH_BUILD_JOBS:-4}" cargo test --locked -p taskmesh-doc-examples --lib --tests -- --test-threads "${TASKMESH_TEST_JOBS:-4}"; \
    printf 'taskmesh-test status=PASS runner=%s runner_version=%s fixture_runner=cargo cargo_version=%s\n' "$runner" "$runner_version" "$cargo_version"

# Laptop feedback excludes benchmark harness tests and the generated README/doc
# fixture. Their correctness and buildability remain required by full `test`,
# `doctest`, and `bench-smoke`, but not after an ordinary runtime/tooling edit.
test-core:
    if cargo nextest --version >/dev/null 2>&1; then \
        CARGO_BUILD_JOBS="${TASKMESH_BUILD_JOBS:-4}" cargo nextest run --locked --workspace --exclude taskmesh-bench --exclude taskmesh-doc-examples --lib --tests --test-threads "${TASKMESH_TEST_JOBS:-4}"; \
    else \
        CARGO_BUILD_JOBS="${TASKMESH_BUILD_JOBS:-4}" cargo test --locked --workspace --exclude taskmesh-bench --exclude taskmesh-doc-examples --lib --tests -- --test-threads "${TASKMESH_TEST_JOBS:-4}"; \
    fi

# Pass 1: test/example targets — pedantic + nursery, test-friendly allows.
# Pass 2: production (--lib --bins) — restriction lints from config/clippy-restrict.txt.
# Pass 3: the publish=false bench harness (statistical float code) gets baseline
#         lint only — pedantic float/cast/flop lints are noise for stats code.
clippy:
    CLIPPY_CONF_DIR={{root}}/config cargo clippy --locked --workspace --exclude taskmesh-bench --exclude taskmesh-doc-examples --tests --examples --benches -- {{clippy_strict}} {{clippy_allows}}
    CLIPPY_CONF_DIR={{root}}/config cargo clippy --locked --workspace --exclude taskmesh-bench --exclude taskmesh-doc-examples --lib --bins -- {{clippy_strict}} {{clippy_allows}} {{clippy_restrict}}
    CLIPPY_CONF_DIR={{root}}/config cargo clippy --locked -p taskmesh --features rayon --lib --bins -- {{clippy_strict}} {{clippy_allows}} {{clippy_restrict}}
    cargo clippy --locked -p taskmesh-doc-examples --all-targets -- -D warnings
    cargo clippy --locked -p taskmesh-bench --all-targets -- -D warnings

clippy-core:
    # Inner-loop Clippy owns production code. `test-core` compiles and runs the
    # tests; full `clippy` remains the authority for test/example/bench lints.
    CARGO_BUILD_JOBS="${TASKMESH_BUILD_JOBS:-4}" CLIPPY_CONF_DIR={{root}}/config cargo clippy --locked --workspace --exclude taskmesh-bench --exclude taskmesh-doc-examples --lib --bins -- {{clippy_strict}} {{clippy_allows}} {{clippy_restrict}}

deny:
    cargo deny check --config config/deny.toml

semgrep:
    uv run python tools/semgrep/check.py

test-architecture:
    uv run python tools/arch/check_crate_boundaries.py
    uv run python docs/plans/bugbash-sep-25-general/tickets/validate_plan.py
    uv run python docs/plans/bugbash-sep-25-general/tickets/validate_scenario_evidence.py

py-lint:
    uv run ruff check tools

py-test:
    uv run pytest tools -q

prompt-check:
    uv run python tools/pm/check.py

# Targeted Python tooling checks. Keep lint-only and test-only edits separate;
# `dev-python-fast` is the convenience path when both changed.
dev-python-lint *files:
    uv run ruff check {{files}}

dev-python-tests *tests:
    uv run pytest {{tests}} -q -m "not qualification"

dev-python-fast files tests:
    just dev-python-lint {{files}}
    just dev-python-tests {{tests}}

# Rust edit loop: scope compilation, lint, and behavior tests to the changed
# package. Use `dev` for cross-package or shared-boundary changes.
dev-rust-fast package *consumers:
    cargo fmt --package {{package}} --check
    CARGO_BUILD_JOBS="${TASKMESH_BUILD_JOBS:-4}" CLIPPY_CONF_DIR={{root}}/config cargo clippy --locked -p {{package}} --lib --bins -- {{clippy_strict}} {{clippy_allows}} {{clippy_restrict}}
    packages=({{package}} {{consumers}}); args=(); for package in "${packages[@]}"; do args+=(-p "$package"); done; CARGO_BUILD_JOBS="${TASKMESH_BUILD_JOBS:-4}" cargo test --locked "${args[@]}" --lib --tests -- --test-threads "${TASKMESH_TEST_JOBS:-4}"

# Test-only Rust edits do not need a production Clippy pass. Include downstream
# consumer package names when a public contract change needs compatibility proof.
dev-rust-tests package *consumers:
    cargo fmt --package {{package}} --check
    packages=({{package}} {{consumers}}); args=(); for package in "${packages[@]}"; do args+=(-p "$package"); done; CARGO_BUILD_JOBS="${TASKMESH_BUILD_JOBS:-4}" cargo test --locked "${args[@]}" --lib --tests -- --test-threads "${TASKMESH_TEST_JOBS:-4}"

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

# Package/file-scoped mutation diagnostics for the edit loop. Never a full
# qualification receipt; use `mutants-generated` for the complete denominator.
mutants-focused *ARGS:
    uv run python tools/verification/run_focused_mutants.py {{ARGS}}

lint-rust: fmt-check clippy

lint-arch: test-architecture

lint-semgrep: semgrep

lint-deps: deny

lint-py: py-lint py-test

lint-rules: semgrep test-architecture prompt-check

# Feature-matrix drift: the `rayon` feature auto-wires the default CPU executor
# on a cfg-gated path that default-feature builds never compile.
test-rayon:
    cargo test --locked -p taskmesh --features rayon --lib
    cargo test --locked -p taskmesh --features rayon --test hardening_executor_authority rayon_cpu_domain_is_separate_and_observable -- --exact
    cargo test --locked -p taskmesh --features rayon --test e2e_scenarios rayon_cpu_soak_results_correct_and_drains -- --exact

doctest:
    cargo test --locked --workspace --exclude taskmesh-doc-examples --doc
    cargo test --locked -p taskmesh --features rayon --doc

# rustdoc must stay link-clean (private-intra-doc-link drift detector).
rustdoc:
    RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --workspace --exclude taskmesh-doc-examples
    RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps -p taskmesh --features rayon

# Benches must keep compiling and smoke-running (no timing is recorded).
bench-smoke:
    cargo bench --locked -p taskmesh-bench -- --test

# Consumer MSRV (H16-019): compile a minimal consumer OUTSIDE this workspace on
# the declared `rust-version`, with the default and the `rayon` public surface.
# The workspace's own dev/bench dependency graph does not resolve on that
# toolchain (and does not have to); this proves the *library* contract instead.
consumer-msrv:
    python3 tools/consumer-msrv/check.py

# Run the criterion wall-clock benches (informational; not a gate).
bench:
    cargo bench --locked -p taskmesh-bench --benches

# Deterministic allocation gate (ADR 9000 / P2). Runs anywhere — no valgrind.
bench-gate:
    bash tools/bench-gate.sh

# Instruction-count gate (ADR 9000 / P2). Needs Linux + valgrind + the matching
# iai-callgrind-runner (`cargo install iai-callgrind-runner --version 0.14.2`).
bench-iai:
    bash tools/bench-iai.sh

# Focused checker commands are kept for debugging. The registered `modelcheck`
# gate runs both checkers plus failure replay, so these are not standalone proof
# inventory gates and must not be added to `release` or hosted CI.
loom:
    RUSTFLAGS="--cfg loom" cargo test --locked -p taskmesh-engine --features loom --test loom_governance --release

# Registered qualification gate: fmt, full clippy/tests, supply-chain, static
# policy, Python, allocation, inventory, and fuzz-harness compilation. Use
# `just dev` for the smaller laptop feedback subset and
# `just verify-macos-ci` for the complete CI profile.
gate: fmt-check clippy test deny semgrep test-architecture py-lint py-test bench-gate gates-inventory fuzz-check
    @echo "gate: registered functional/static gate passed; run 'just verify-macos-ci' for CI qualification"

# The libFuzzer targets (fuzz/) type-check and lint on stable, so they cannot
# rot between nightly fuzzing campaigns (`just fuzz`).
fuzz-check:
    bash tools/fuzz/check.sh

# Purpose-scoped Rust product loop. Python tooling checks are separate in
# `dev-python-fast`; unrelated Python lint/tests are not repeated for Rust edits.
# Heavy authorities remain in the `ci`, `nightly`, and `release` profiles.
dev: fmt-check clippy-core test-core semgrep test-architecture gates-inventory
    @echo "dev: core macOS feedback passed; run 'just verify-macos-ci' for receipt-backed CI qualification"

# Local proof matrix: feature-matrix drift, doctests, link-clean rustdoc, bench
# compilation, and the consumer-MSRV build. Chained by `ci`.
matrix: test-rayon doctest rustdoc bench-smoke consumer-msrv
    @echo "matrix: feature-matrix checks passed"

# Exhaustive concurrency model-check with shuttle's randomized scheduler (ADR
# 9000 / P6). Heavier than loom; run directly only while debugging that checker.
shuttle:
    RUSTFLAGS="--cfg shuttle" cargo test --locked -p taskmesh-engine --features shuttle --test shuttle_governance --release

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
# This is not in `release`: a current-source qualification receipt and
# explicit API/wire/behavior adjudication are separate release inputs.
semver-release:
    python3 tools/release/semver.py --out target/release/semver

# Final release collector runs CI and nightly gates; use only for explicit release qualification.
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

# CI profile: functional, static, and contract matrix without long campaigns.
ci: gate matrix
    @echo "ci: functional, static, and matrix surface passed"

# Explicit high-cost proof profile. No schedule is configured.
nightly: mutants-critical modelcheck tsan fuzz coverage-report bench-iai mutants-generated
    @echo "nightly: high-cost proof surface passed"

# Release gate profile includes CI and nightly; required.json owns the union.
release: ci nightly
    @echo "release: ci and nightly required gate surfaces passed"

# CI macOS receipt without high-cost nightly campaigns. Release qualification
# remains `release` or the nightly-inclusive required collector.
# Independent low-cost static checks use the inventory's bounded four-worker
# group; Cargo builds and resource-heavy proof producers remain serialized.
# Dirty source is rejected before any gate starts.
verify-macos-ci:
    CARGO_BUILD_JOBS="${TASKMESH_BUILD_JOBS:-4}" uv run python tools/gates/run.py --profile ci --require-clean-source --allow-platform-skips --receipt target/verification/macos-gates.json

verify-macos-nightly:
    CARGO_BUILD_JOBS="${TASKMESH_BUILD_JOBS:-4}" uv run python tools/gates/run.py --profile nightly --require-clean-source --allow-platform-skips --receipt target/verification/macos-nightly-gates.json

# Install the tracked fail-closed push admission hook for this clone. The hook
# accepts a branch update only when the saved local receipt matches the exact
# clean commit being pushed. Git's explicit --no-verify remains the emergency
# override and must be disclosed when used.
install-hooks:
    git config --local core.hooksPath .githooks
    @echo "hooks: installed .githooks (pre-push requires current local receipt)"

validate-local-receipt:
    uv run python tools/gates/run.py --validate-receipt target/verification/macos-gates.json
