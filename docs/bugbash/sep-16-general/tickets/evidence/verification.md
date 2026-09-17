# Verification receipt — 2026-09-16

- Source HEAD: `9ae9547216c70458f54f37368a67661321060886`.
- Initial worktree: clean `main...origin/main` (cached local tracking state; remote/hosted CI not qualified here).
- Audit changes: documentation and a standalone observation harness only; production source unchanged.
- Rust source surfaces: contract, engine, Tokio host, Rayon adapter; additional parallel review covers benchmark/tools/CI/docs.

## Executed in previous final audit

| Command | Result |
| --- | --- |
| `cargo test --workspace --locked` | exit 0 |
| `cargo fmt --all --check` | exit 0 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| `python3 tools/arch/check_crate_boundaries.py` | exit 0, no violations |
| `just clippy` | exit 101, runtime.rs:707 needless_pass_by_value |
| `semgrep --config tools/semgrep/rules --error crates` | exit 1, runtime.rs:114/163 discarded result |
| `cargo deny check --config config/deny.toml` | exit 1, crossbeam-epoch vulnerability + proc-macro-error2 unmaintained |
| Standalone locked/offline observation probe, debug | 14/14 observations reproduced, exit 0 |
| Standalone locked/offline observation probe, release | 14/14 observations reproduced, exit 0 |
| Probe `cargo fmt --check` | exit 0 |
| `cargo +1.83.0 check -p taskmesh-engine --test prop_invariants --locked` | exit 101, getrandom 0.4.2 edition2024 manifest incompatible |
| `uv run python tools/pm/pm.py lint` | exit 1, agents drift + 4 missing targets |
| Architecture check_edges negative metadata diagnostic | unknown/engine-Tokio/engine-bench/nonoptional Rayon all incorrectly return [] |
| `semgrep ... --verbose --error crates` | integration suites excluded in 52 skipped paths |
| Bash default-mode failing pipeline diagnostic | exit 0 for `false` piped into `true`, reproduces failure masking |
| Final ticket-document integrity check | 25 IDs, priority counts 4/19/2, 29 Markdown files, no broken local links/trailing whitespace |

Final worktree check: source HEAD unchanged, no tracked production diff, only `docs/bugbash/` newly untracked. The four delegated audits completed and their agents were closed.

The earlier audit on the same production HEAD also completed workspace/Rayon tests, rustdoc, Python tests, allocation gate, loom and shuttle. These are prior same-source observations; the final audit does not upgrade them to hosted CI, release, runtime integration, or coverage of every defect listed.

## Reproduce observations

From repository root:

```sh
cargo test --offline --locked \
  --manifest-path docs/bugbash/sep-16-general/tickets/evidence/repro/Cargo.toml \
  --target-dir /tmp/taskmesh-sep16-repro-target \
  -- --nocapture
```

Remove `--offline` if the locked packages are not cached. The standalone workspace and its lockfile preserve the runtime dependency versions used by the repository audit (including Tokio 1.52.3).

The probe intentionally asserts **current defective behavior**, so its green result proves reproduction, not remediation. When fixing a ticket, add production regression tests with the corrected assertions; do not use this observation harness as a product qualification gate.

| Probe | Ticket |
| --- | --- |
| substrate_waiters_bypass_reject_policy_and_queue_bound | TM16-001 |
| blocking_runfor_ignores_deadline | TM16-002 |
| invalid_topology_panics_in_fallible_build | TM16-003 |
| default_policy_boots_without_builtin_inventory | TM16-004 |
| completion_only_memory_policy_allows_early_stage_release | TM16-005 |
| saturation_at_u32_max_admits_over_budget | TM16-008 |
| stage_release_does_not_refresh_leak_activity | TM16-009 |
| leak_sweep_keeps_a_claimable_dead_ticket | TM16-010 |
| estimated_reconcile_resurrects_released_reservation | TM16-011 |
| large_wfq_weights_collapse_ratio_into_fifo | TM16-013 |
| abandoned_wfq_work_leaves_phantom_debt | TM16-014 |
| inline_cpu_work_runs_before_relative_timer_starts | TM16-022 |
| blocking_stack_request_does_not_take_large_stack_slot | TM16-023 |
| requested_stack_async_deadline_waits_for_blocking_child_shutdown | TM16-024 |

The caught topology panic and debug dead-ticket invariant panic are expected output in the observation tests. TM16-005 is a reproduced contract ambiguity, not an independently confirmed runtime defect. Timing/concurrency probes characterize the current host execution; deterministic regression hooks remain ticket work. No fix has been applied.

## Additional audit — TM16-026–029

- Reconfirmed source HEAD unchanged; entry/exit tracked diff is empty and only `docs/bugbash/` is untracked.
- Re-read governor admission/abandon/promote/validation, state/ledger, memory activity, fairness selection, host hint/worker/deadline paths, benchmark workload/loadgen/metrics and their real callers/tests.
- New records: 4 × P2. Total inventory: 29 (P1=4, P2=23, P3=2 including one contract decision).
- Prior workspace/CI/tool-gate results above were not rerun or upgraded by this additional audit.

| Command / observation | Result |
| --- | --- |
| Runtime harness `cargo test --offline --locked --manifest-path .../evidence/repro/Cargo.toml` | debug 15/15, release 15/15, exit 0 |
| Benchmark harness `cargo test --offline --locked --manifest-path .../evidence/repro-bench/Cargo.toml` | debug 3/3, release 3/3, exit 0 |
| TM16-026 controlled clock-read/state-commit interleaving | just-created lease reclaimed; late heartbeat overwrites newer activity and is reclaimed |
| TM16-027 invalid event input | NaN/negative/reversed time accepted; inf debug overflow caught; release wrap returns conserved |
| TM16-028 seed 42, rate 1/1000, equal 1ms mean dwell, count 1000 | actual 2.648265/s; configured stationary CTMC rate 500.5/s |
| TM16-029 2 arrivals, service 10000ns, interval 1000ns | completed=2, histogram samples=10, p50=4999ns |
| Both standalone harnesses `cargo fmt --check` | exit 0 |
| Additional ticket integrity check | 29 unique IDs; priority counts 4/23/2; 33 Markdown files; no broken local links/trailing whitespace |
| `git diff --exit-code`, HEAD/status | exit 0; same HEAD; only untracked docs/bugbash/ |

These 18 observation tests assert current behavior, not corrected behavior. The added clock provider uses monotonically advanced atomic time plus barriers; no real NTP jump or OS preemption-frequency measurement was performed. Benchmark debug overflow is intentionally caught in the retained observation; the initial uncaught diagnostic exited 101 and established the failure site.

The new standalone benchmark lockfile was seeded from the repository lock and Cargo-pruned offline. It preserves the source snapshot's measurement dependency versions (rand 0.8.6, rand_distr 0.4.3, hdrhistogram 7.5.4). Root Cargo.lock and production manifests were not edited.

```sh
cargo test --offline --locked \
  --manifest-path docs/bugbash/sep-16-general/tickets/evidence/repro-bench/Cargo.toml \
  --target-dir /tmp/taskmesh-sep16-bench-target -- --nocapture
cargo test --release --offline --locked \
  --manifest-path docs/bugbash/sep-16-general/tickets/evidence/repro-bench/Cargo.toml \
  --target-dir /tmp/taskmesh-sep16-bench-target -- --nocapture
```

No additional production bug was established for queued abandon-without-promote under identical per-class queued costs. DRR idle credit/custom waker panic and unused zero-median tail ratio behavior are separated as hardening/quality candidates in QUALITY-AND-SCOPE.

To repeat the release-mode observation:

```sh
cargo test --release --offline --locked \
  --manifest-path docs/bugbash/sep-16-general/tickets/evidence/repro/Cargo.toml \
  --target-dir /tmp/taskmesh-sep16-repro-target -- --nocapture
```

The final audit directly confirmed runner 0.14.2's regression environment option in its primary `src/runner/args.rs`, and Rust 1.85 requirements in the locked getrandom/proptest/clap manifests. Linux IAI execution and actual 1.81 consumer qualification were not performed.

## Additional audit (2) — TM16-030–033

- Same source HEAD `9ae9547216c70458f54f37368a67661321060886`; tracked source/manifest/lockfile diff remains empty.
- 3 new P2 behavioral records plus 1 P3 documentation/gate-inventory record. Total: 33 records, P1=4/P2=26/P3=3, including the existing TM16-005 contract decision.
- Focus: driven-port object retirement, synchronous admission budget, worker setup/string inputs, serialized contract assumptions, real recipe inventory and policy comments. No production remediation, commit or push performed.

| Executed command / observation | Result |
| --- | --- |
| Runtime harness locked/offline tests, debug | 18/18, exit 0 |
| Runtime harness locked/offline tests, release | 18/18, exit 0 |
| Benchmark harness locked/offline tests, debug and release | 3/3 each, exit 0 |
| queued_waker_drop_reenters_governor_under_lock | child reaches destructor marker and remains blocked; parent kill/wait after 2s |
| requested_stack_class_with_nul_panics_before_spawn_error_mapping | std thread-name validation panic becomes caller Tokio JoinError::is_panic, not RunError |
| acquire_timeout_does_not_cover_synchronous_admission_lock_wait | 5ms acquire budget; controlled 100ms lock hold; job begins after ≥80ms and returns Ok |
| `just --show mutants-critical` | exit 1, no such recipe |
| `just --show cov-gate` | exit 1, no such recipe |
| Both standalone harnesses `cargo fmt --check` | exit 0 |
| Ticket document integrity | 33 unique IDs; priority counts 4/26/3; 37 Markdown files; no broken local links/trailing whitespace |
| Final HEAD / `git diff --exit-code` / status | same HEAD; exit 0; only untracked docs/bugbash/ |

There are 21 retained observations across the two harnesses. Green proves reproduction only. The custom-port scenarios exercise accepted embedding APIs and controlled contention; they do not establish default TokioPermitWaker deadlock or natural contention frequency. The deadlock child was forcibly terminated and waited on; no hung child is intentionally retained. The thread-name panic was executed on the blocking requested-stack path; the shared async setup risk is static evidence only.

Additional P3 policy drift is not counted as a runtime failure or as authorization to introduce mandatory mutation/coverage tooling. Unknown-field wire acceptance, DRR idle credit and fixture duplication remain contract/quality candidates rather than additional confirmed defects. Earlier whole-workspace/strict lint/hosted qualification results were not refreshed by these focused commands.

## Additional audit (3) — TM16-034–036

- Source HEAD remains `9ae9547216c70458f54f37368a67661321060886`; only the existing untracked docs/bugbash/ tree was extended. No production code, root manifests/lockfile, AGENTS, commit or push changed.
- New records: P2=1/P3=2. Inventory total: 36, P1=4/P2=27/P3=5, including TM16-005's existing contract decision.
- Reviewed benchmark callers/bodies/fixture readiness, PM renderer/config/tests, measurement-library primary source, metric reporting and previously recorded ownership/input boundaries.

| Executed check | Result / proof scope |
| --- | --- |
| Runtime observation harness, locked/offline debug and release | 18/18 each, exit 0 |
| Benchmark observation harness, locked/offline debug and release | 5/5 each, exit 0 |
| iai_function_bodies_destroy_input_governor_before_return | source-extracted roundtrip/reject/snapshot bodies begin input destruction before returning; portable ownership proof, not Callgrind |
| contention_batch_count_differs_from_criterion_iterations | source-guarded arithmetic: (iters=1,t=8) runs 8 ops; (9,8) runs 8; Criterion divides elapsed by requested iters |
| Actual PM CLI preview on retained nested-template fixture | exit 0, WRONG TEMPLATE rendered instead of configured RIGHT TEMPLATE |
| Actual PM CLI lint on same fixture | exit 0, 1 target up to date despite wrong template identity |
| Both standalone harnesses `cargo fmt --check` | exit 0 |
| `git diff --exit-code` | exit 0, no tracked source diff |
| Final document/HEAD integrity | 36 unique IDs, priority counts 4/27/5, 42 Markdown files, no broken local links/trailing whitespace, unchanged HEAD |

The two Rust harnesses now contain 23 observations/diagnostics, including a static formula diagnostic and portable ownership test. These are reproduction evidence, not remediation greens. The new build script extracts the actual repository IAI function bodies without measurement attributes; it does not emulate Callgrind. Linux instruction counts, teardown percentage and actual Criterion wall-clock bias were not measured.

Primary source read locally: iai-callgrind 0.14.2 default entry-point docs (`src/lib_bench.rs:206-207`), macros 0.5.1 original signature/body rendering (`src/lib_bench.rs:433-434`), Criterion 0.5.1 `elapsed / iters` analysis (`src/analysis/mod.rs:124-127`). No library-version upgrade was performed. The standalone bench manifest/lockfile only added a direct reference to the already-used taskmesh package; root dependency ownership was preserved.

The retained PM fixture is under `evidence/pm-nested-template/` and uses a pre-created incorrect output to exercise the real lint path. No PM sync was run against actual project instructions. Default flat targets are not proven affected by the nested-template bug.

Fixed-sleep holder readiness, USL no-finite-peak reporting and unsupported/forward-compatible input assumptions remain separated as quality/contract candidates. Previous workspace/strict-lint/hosted qualification was not refreshed by the focused commands above.

## Additional audit (4) — TM16-037–038

- Source HEAD remains `9ae9547216c70458f54f37368a67661321060886`; tracked diff is empty. Existing untracked docs/bugbash/ was extended; no production remediation/commit/push.
- New records: P2=1/P3=1. Total inventory: 38, P1=4/P2=28/P3=6, including the existing TM16-005 contract decision.
- Reviewed queued CPU cancellation versus actual worker start, fallback policy authority, DRR empty/reentry state, and allocation gate parsing/failure propagation. Independent source reads and shell/runtime probes were batched in parallel; no additional delegated agent was used for this pass.
- CPU caller cancellation versus worker-owned lease lifetime and fallback resource-only/full-policy semantics did not yield an additional established defect. Existing contract/quality notes were retained. DRR idle credit was promoted from candidate to confirmed service-bound defect based on a new deterministic public-engine probe, not standard-algorithm preference alone.

| Executed check | Result / proof scope |
| --- | --- |
| New DRR probe alone, locked/offline debug | 1/1, exit 0; fresh prefix A,A,B versus warmed prefix A×10 |
| Runtime harness, locked/offline debug | 19/19, exit 0 |
| Runtime harness, locked/offline release | 19/19, exit 0 |
| Runtime harness `cargo fmt --check` | exit 0 after formatting the newly added closure; initial check exited 1 and prevented chained tests from running |
| `bash -n .../evidence/allocation-gate-input.sh` | exit 0 |
| Actual allocation gate with 6 controlled cargo cases | below=0, above=1, malformed dots=0, partial number=0, missing=1, cargo failure=17; observation harness exit 0 |
| Exploratory duplicate metric lines | current macOS awk exited 2; rejected as a false-green finding, not retained as an expected passing scenario |
| `git diff --exit-code`, HEAD/status | exit 0; same HEAD; only untracked docs/bugbash/ |

Commands retained for this pass:

```sh
cargo test --offline --locked \
  --manifest-path docs/bugbash/sep-16-general/tickets/evidence/repro/Cargo.toml \
  --target-dir /tmp/taskmesh-sep16-audit.ZsmveG/target
cargo test --release --offline --locked \
  --manifest-path docs/bugbash/sep-16-general/tickets/evidence/repro/Cargo.toml \
  --target-dir /tmp/taskmesh-sep16-audit.ZsmveG/target
bash docs/bugbash/sep-16-general/tickets/evidence/allocation-gate-input.sh
```

The Rust harnesses now retain 24 observations/diagnostics (runtime 19, benchmark 5). Only the changed runtime harness was rerun in this pass. Unchanged benchmark/PM fixtures, whole-workspace gates and hosted/Linux qualification were not repeated; earlier receipts remain historical. Green asserts current defects, not corrected behavior. The allocation gate fixture replaces producer stdout/status through a scoped exported Bash function; it does not establish malformed output from the real alloc_probe or execute Cargo under that shim. The current producer emits a valid fixed-decimal line, so the parser issue is P3.

The deadlock observation child used by the existing runtime suite was killed and waited on in each run. The DRR probe drains all its tickets and verifies zero queued/inflight. Final document integrity: 38 unique ticket IDs, priority counts 4/28/6, 44 Markdown files, no broken local links or trailing whitespace.

## Additional audit (5) — TM16-039–040; TM16-028 evidence extension

- Same source HEAD `9ae9547216c70458f54f37368a67661321060886`; tracked diff remains empty. Only the pre-existing untracked docs/bugbash/ tree was extended. No production/test source, root manifests/lockfile, actual PM outputs, commits or pushes changed.
- New records: P2=1/P3=1. Inventory: 40, P1=4/P2=29/P3=7, including TM16-005's existing contract decision. Zero-dwell generator nonprogress was added to existing TM16-028, not counted again.
- Parallel agents: Bernoulli inspected tooling and reproduced PM duplicate-key/path-boundary candidates in temporary fixtures; Herschel inspected contract/composite/inventory/attribution and established no new distinct defect. Main re-read the relevant PM source, independently reran duplicate lint on a retained fixture, and kept output containment as a contract/hardening question. Both agents were closed after integration.
- Main scope: host acquisition/cancellation/executor lifetime, retry arithmetic, memory transitions, workload generation and test-error handling. Existing documented behavior and previously registered causes were not re-counted.

| Executed check | Result / scope |
| --- | --- |
| Source-extracted mixed-soak probe, focused debug | unmodified control and all-disabled mutant both pass; mutant additionally asserts ClassDisabled on all 360 run outcomes |
| Runtime observation harness, locked/offline debug and release | 20/20 each, exit 0 |
| Zero-dwell generator probe, focused debug | positive dwell control completes; zero dwell child enters actual generator and fails to return, killed/waited after 1s |
| Benchmark observation harness, locked/offline debug and release | 6/6 each, exit 0 |
| Actual PM CLI lint on retained duplicate target fixture | exit 0, `pm lint OK: 1 target(s) up to date` |
| `test ! -e .../pm-duplicate-target/MISSING.md` | exit 0, first declared output remains absent |
| Both standalone harnesses `cargo fmt --check` | exit 0 |
| Final document integrity | 40 unique ticket IDs, P1/P2/P3=4/29/7, 48 Markdown files; no broken local links or trailing whitespace |
| `git diff --exit-code`, HEAD/status | exit 0, unchanged HEAD, only untracked docs/bugbash/ |

```sh
cargo test --offline --locked \
  --manifest-path docs/bugbash/sep-16-general/tickets/evidence/repro/Cargo.toml \
  --target-dir /tmp/taskmesh-sep16-audit.ZsmveG/target
cargo test --release --offline --locked \
  --manifest-path docs/bugbash/sep-16-general/tickets/evidence/repro/Cargo.toml \
  --target-dir /tmp/taskmesh-sep16-audit.ZsmveG/target
cargo test --offline --locked \
  --manifest-path docs/bugbash/sep-16-general/tickets/evidence/repro-bench/Cargo.toml \
  --target-dir /tmp/taskmesh-sep16-bench-release-target
cargo test --release --offline --locked \
  --manifest-path docs/bugbash/sep-16-general/tickets/evidence/repro-bench/Cargo.toml \
  --target-dir /tmp/taskmesh-sep16-bench-release-target
PYTHONDONTWRITEBYTECODE=1 .venv/bin/python tools/pm/pm.py \
  --targets docs/bugbash/sep-16-general/tickets/evidence/pm-duplicate-target/targets.yaml \
  --pm-dir docs/bugbash/sep-16-general/tickets/evidence/pm-duplicate-target \
  --repo-root docs/bugbash/sep-16-general/tickets/evidence/pm-duplicate-target lint
```

The new runtime build script extracts the actual mixed-soak function and its helpers. The mutant changes only the three inflight limits to zero and adds precise outcome assertions at the three discarded-result sites; original join/sweep/drain assertions are preserved. A separately named unmodified function is run as a control. This is focused assertion-sufficiency evidence, not a full-workspace mutation campaign or a claim that the normal runtime rejects all work. Initial extractor guards stopped compilation twice because broad matching also counted sweep calls/inner awaits; exact run-result branch selection fixed the harness. These were harness construction errors, not product failures. The final generated helper imports were narrowed and final tests emitted no new warning.

The zero-dwell observation uses the actual seeded generator in an isolated child. Locked rand_distr 0.4.3 source confirms that Exp::new(infinity) stores inverse zero, so the phase_end update cannot advance. The one-second kill establishes bounded observed nonreturn; the infinite-loop conclusion comes from the zero increment and loop condition. Both that child and the existing waker-deadlock child were terminated and waited on. No hung worker was left deliberately running.

There are now 26 retained Rust observations/diagnostics (20 runtime, 6 benchmark). Green asserts reproduced behavior, not remediation. Earlier allocation/nested-template checks, full-workspace gates, hosted checks, Linux IAI counts and deployment qualification were not refreshed. The PM output containment candidate was only reproduced by the agent in its disposable `/tmp/taskmesh-audit5.HWPs41` fixture; main did not overwrite any external user file or independently rerun those writes. Resolution-base semantics are not assumed to be a sandbox promise.

## External advisory attribution (previous audit)

- [RustSec RUSTSEC-2026-0204](https://rustsec.org/advisories/RUSTSEC-2026-0204.html): crossbeam-epoch formatting of invalid pointers; patched at >=0.9.20. Taskmesh exploit reachability was not demonstrated.
- [RustSec RUSTSEC-2026-0173](https://rustsec.org/advisories/RUSTSEC-2026-0173): proc-macro-error2 is unmaintained. Local cargo-deny provides the dependency-chain evidence; this is not classified as a runtime vulnerability.
