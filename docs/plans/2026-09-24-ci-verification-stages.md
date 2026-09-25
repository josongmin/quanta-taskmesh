# Taskmesh verification stages: 2026 audit and implementation plan

Status (2026-09-26): **W3 execution-denominator implementation has clean-HEAD
macOS CI proof. W4's bounded workflow is staged locally; hosted trial,
branch-rule decision, and W5 release qualification remain separate.**
Audit baseline: `main@e5aa4b63a1fedb3b416d33d1ea7ff364258002b5`, clean
before this plan was created on 2026-09-24. The initial draft was untracked and
had no test or CI qualification. A later clean `main@934230d` produced a valid
local macOS CI-profile gate receipt; that source-specific receipt does not
qualify later edits, nightly campaigns, or release. No mutation campaign,
nightly qualification, or release qualification was run for this plan.

## Decision

Use `local -> dev -> ci -> nightly` as feedback and cost scopes, with `release`
as a separate decision. These are project contracts, not standardized names.
Execution location, trigger, and proof strength are independent dimensions:

| Scope | Purpose and source state | Authority |
|---|---|---|
| `local` | Owner-selected edit-loop checks; dirty source allowed | Diagnostic only |
| `dev` | Cross-owner checkpoint on the current worktree; dirty source allowed | Diagnostic only |
| `ci` | All 16 non-nightly required gates on one clean, stable source | Exact-source CI-profile receipt; macOS may be platform-scoped |
| `nightly` | Seven expensive proof gates on an isolated clean source | Separate deep-proof receipt; a subset or interruption is not qualified |
| `release` | All 23 ordinary required gates, plus release-only semver, finding, raw-artifact, and human adjudication requirements | Full release receipt and explicit decision |

`nightly` is a cost/profile name, **not a schedule**. The full `ci.yml`, bench,
and release workflows remain manual. The locally staged `pr-ci.yml` is a
non-required PR/main trial of the bounded CI profile; the validator permits
automatic triggers only there. No hosted run or merge rule is claimed before
the workflow is pushed, enabled, and measured. A scheduled subset must carry a
different receipt/profile name and cannot satisfy full `nightly` or `release`.

The existing `Justfile` is the command authority; `tools/gates/inventory.json`
records gate metadata, and independently authored `tools/gates/required.json`
owns required membership and order. Retain that separation. Do not add a second
hand-maintained list of every Rust test function or pytest case.

## Current-source audit

| Observation | Evidence and consequence |
|---|---|
| The gate graph is already substantially structured. | `required.json` has 23 required IDs, of which seven are `nightly_required`; `run.py` derives the CI complement. Inventory has 24 entries including release-only `semver-release`. `validate_inventory.py` checks Justfile/workflow/required parity, fail-capable invocation, tracked scripts, producer handoff, and trigger policy. Preserve these controls. |
| `fast` is an inventory tier, not an edit-loop runtime promise. | `Justfile:196` includes full Rust `test` and `py-test` in `gate`; `Justfile:207` uses `test-core` and `clippy-core` for `dev`. Do not route an ordinary edit through the inventory `fast` tier by name. |
| Python owner feedback is separate from `just dev`. | `Justfile:87-108` provides selected Ruff/pytest and Rust package/consumer recipes. `dev-python-tests` excludes `qualification`; `py-test` in CI includes it. A `dev` result alone does not cover Python tooling changes. |
| CI has an exact-source local receipt and a staged hosted trial, not an active required PR check. | `verify-macos-ci` writes the clean-source 16-gate receipt. `pr-ci.yml` invokes that recipe in one fail-closed job for PR/main/manual events and validates `$GITHUB_SHA`. The pre-push hook and local workflow file do not establish a GitHub branch rule or a hosted result. |
| Hosted `ci.yml` stays full manual qualification. | It contains curated and generated mutations, fuzz, modelcheck, and a full receipt collector. `pull_request` on that file would run deep producers. The dedicated bounded workflow has no deep campaign and remains non-required until runner budget and hosted proof are reviewed. |
| The release witness's `required_gate` is a proof dependency, not its test executor. | `finding_proof.py:48-79` runs every witness with its own focused Cargo/pytest command; `tools/release/receipt.py:386` also requires the row's named ordinary gate to PASS. For example a Python IAI policy witness names `bench-iai`, while `py-test` runs its module. The code does not document the intended domain relationship, but TM21-020's `test-rayon` association cannot be called a false execution mapping from this field. Retract that claim from the earlier audit and document the two meanings. |
| Baseline Semgrep enrollment was weaker than its full-test-tree wording. | The original `tools/semgrep/check.py` accepted one scanned Rust file per crate `tests/` tree plus six explicit paths. W1 compares every present Rust test file to Semgrep's scanned paths and the policy fixture removes one file from a still-covered tree. The subsequent `934230d` CI-profile run executed both the real Semgrep gate and its pytest policy fixtures. |
| Baseline weighted fairness property proved repeatability but not complete drain. | The original `prop_invariants.rs` could return with tickets left while comparing two same-helper outputs. W1 requires every admission to queue, every ticket to promote, and a zero queued/inflight snapshot. `fairness_weighted.rs` still independently checks exact order. The subsequent `934230d` CI-profile run executed the changed property. |
| Qualified test execution now has one pinned runner. | `test` and `test-rayon` require cargo-nextest 0.9.104, compare machine-readable selected binaries/cases with termination events, and self-report runner, command, catalog, and case digests. The Cargo fallback remains only in `test-core` diagnostic feedback. `deny` still consumes a live advisory input, so its result is source-bound and time-bound. |
| Existing proof limits are intentional. | `coverage-report` means a report was produced, not a coverage threshold. `bench-smoke` checks build/smoke, not latency. macOS skips Linux `bench-iai` as `SKIPPED_PLATFORM`; a platform-scoped result is not full Linux release qualification. The collector samples source before/after, so final runs need an exclusive checkout to exclude edit-and-restore. |

## Authoritative gate placement

Each row names the first routine scope in which the **whole** gate runs. Local
owner checks can run the same underlying command on selected paths or packages.
CI must run all 16 of its IDs regardless of local selection.

| Gate IDs or test surface | `local` / `dev` | `ci` | `nightly` / `release` |
|---|---|---|---|
| `fmt-check`, `clippy` | Package fmt/production Clippy in `dev-rust-fast`; workspace fmt + `clippy-core` in `dev` | Full `fmt-check` and multi-pass `clippy` including test/example/bench and Rayon production code | No repeat solely because nightly ran |
| `test` | Owner crate and impacted consumers; `test-core` in `dev` excludes bench harness and generated doc fixture | Full workspace unit/integration tests including contract, engine, host, Rayon adapter, bench harness, and doc fixture | No second full workspace run solely for nightly |
| `py-lint`, `py-test` | Changed Python files and owner tests via `dev-python-*`; explicitly run these after Python changes | Full Ruff and `pytest tools`, including registered `qualification` and `slow` cases | Producer campaigns use their own evidence; do not substitute pytest PASS for producer PASS |
| `semgrep`, `test-architecture`, `gates-inventory` | `dev` runs real source/policy/parity checks; owner-local rule tests when those tools change | Required again on clean source; Semgrep rule fire/clean fixtures are also in `py-test` | No independent nightly duplication |
| `deny`, `fuzz-check` | Focused when dependency or fuzz harness changes | Advisory/dependency check and stable-toolchain compile/lint of all three fuzz targets | `fuzz-check` is not a fuzz campaign |
| `bench-gate` | Focused after allocation-sensitive changes | Deterministic allocation gate | Separate Linux instruction-count `bench-iai` |
| `test-rayon`, `doctest`, `rustdoc`, `bench-smoke`, `consumer-msrv` | Focused when feature, docs, bench, or public contract changes | Required feature/docs/bench/MSRV matrix; `bench-smoke` has no performance verdict | Semver and human contract adjudication belong to `release` |
| `modelcheck`, `tsan`, `fuzz` | Focused checker/reproducer only when debugging the owner; no campaign in routine edits | Normal concurrency tests plus `fuzz-check` | Complete bounded model exploration/replay, real-memory race check, and semantic fuzz witnesses |
| `mutants-critical`, `mutants-generated`, `coverage-report`, `bench-iai` | Never part of ordinary edit or `dev` checks | Not in CI profile | Separate curated/full mutation denominators; coverage is a report; IAI needs Linux and a qualified comparison |

Cargo's `--lib --tests` does not include doctests; feature-gated model targets
require their checker feature and matching `--cfg`. Keep the dedicated doc,
Rayon, and modelcheck gates rather than assuming the workspace test gate selects
them. `test-rayon` is the host facade's feature check; the independent
`taskmesh-rayon` adapter tests are selected by workspace `test`. The release
witness's `required_gate` can refer to a related proof domain; it must not be
treated as an assertion about test selection.

### Gate-by-gate placement (inventory snapshot)

This table maps every one of the 24 inventory IDs. The gate's recipe in
`Justfile` and required status in `required.json` remain authoritative;
the table is an audit map of selection and evidence meaning.

| Gate | First full routine scope | Selection / proof claim |
|---|---|---|
| `fmt-check` | `dev`; required `ci` | Workspace Rust formatting; owner-local `dev-rust-*` formats one package |
| `clippy` | required `ci` | Full multi-pass Rust production, test/example/bench, Rayon feature, doc fixture and bench harness lint; `clippy-core` in `dev` is narrower |
| `test` | required `ci` | Default library + 72 integration targets and doc-fixture library; `test-core` in `dev` excludes bench and doc fixture |
| `deny` | required `ci` | Dependency/license/advisory policy; live advisory input must be identified |
| `semgrep` | `dev`; required `ci` | Real Rust scan + per-file test-tree enrollment; its pytest policy regression is owned by `py-test` |
| `test-architecture` | `dev`; required `ci` | Crate/manifest boundary policy |
| `py-lint` | required `ci` | Ruff over `tools/`; selected owner Ruff in local Python loop |
| `py-test` | required `ci` | All tracked Python test modules and marks, including `qualification` and `slow` |
| `bench-gate` | required `ci` | Deterministic allocation threshold, not timing |
| `gates-inventory` | `dev`; required `ci` | Justfile/inventory/required/workflow parity and fail-capable invocation |
| `fuzz-check` | required `ci` | Stable Clippy compilation of three fuzz bins, not fuzz execution |
| `test-rayon` | required `ci` matrix | Host Rayon-feature library plus one exact feature-only integration case |
| `doctest` | required `ci` matrix | Default workspace and host Rayon-feature Rust doctests |
| `rustdoc` | required `ci` matrix | Warning-free generated docs, default workspace and host Rayon feature |
| `bench-smoke` | required `ci` matrix | Eight default Criterion bench harnesses with `--test`; no regression threshold |
| `consumer-msrv` | required `ci` matrix | External minimal consumer at declared Rust floor, default + Rayon |
| `mutants-critical` | explicit `nightly` | Curated fault inventory with exact primary oracles and control survivor |
| `mutants-generated` | explicit `nightly` | Separate complete generated mutation denominator |
| `modelcheck` | explicit `nightly` | Required Loom/Shuttle models and replay artifact |
| `tsan` | explicit `nightly` | Selected real-memory race binaries; `status=CLEAN` required |
| `fuzz` | explicit `nightly` | Three bounded semantic fuzz campaigns with validated raw evidence |
| `coverage-report` | explicit `nightly` | Instrumented library report only; no percent threshold |
| `bench-iai` | explicit `nightly`, Linux | Instruction-count comparison; only `QUALIFIED`, not `BASELINE_CREATED`, proves regression status |
| `semver-release` | `release` only | Four-crate compatibility audit; final release additionally needs finding and human adjudication evidence |

`check`, `bench`, `prompt-check`, `lint-*`, and `mutants-focused` are
useful diagnostics or aliases, not additional required gates. In particular,
`lint-py` invokes both `py-lint` and `py-test`; `bench` runs informational
wall-clock benchmarks; and `prompt-check` is exercised by `py-test`.

## Exhaustive target map at this source freeze

This is an **audit snapshot**, not a second authoritative manifest. Cargo
metadata reports 75 Rust integration-test targets, not 77 files: the engine's
`harness/mod.rs` and host's `common/mod.rs` are support modules. Three engine
model targets declare required features; the remaining 72 are selected by the
default-feature workspace `test` recipe. In-crate unit-test functions are
selected through each package's library test binary, so count those from the
runner's machine list rather than source file names. Full `test` also selects
the generated `taskmesh-doc-examples` library tests; that package has no
integration-test target.

Two cases are 64-bit-only via `#[cfg(target_pointer_width = "64")]`:
`taskmesh-contract/tests/topology_validation.rs` checks unrepresentable
resolved capacity and `taskmesh/tests/config_inventory.rs` checks the host CPU
pool bound. The proposed case catalog must record these two conditional cases;
on a 32-bit run it must mark them platform-excluded, not executed.

| Package | Default integration targets in CI `test` | Additional execution |
|---|---|---|
| `taskmesh-contract` (6) | `contract_builders`, `contract_roundtrip`, `error_surface`, `snapshot_oracle`, `task_plan_validation`, `topology_validation` | `doctest`, `rustdoc`, `consumer-msrv`; release semver/adjudication on publication |
| `taskmesh-engine` (37 default) | `admission_queue`, `audit_hardening`, `checkpoint_contract`, `composite_root_attribution`, `concurrency_fuzz`, `concurrency_stress`, `config_validation`, `differential_model`, `fairness_best_effort`, `fairness_drr_deadline`, `fairness_fifo`, `fairness_weighted`, `hardening_child_scope`, `hardening_close_admission`, `hardening_effect_retirement`, `hardening_exact_accounting`, `hardening_fairness_reference`, `hardening_lease_token`, `hardening_lifecycle`, `hardening_memory_epochs`, `hardening_nested_wait`, `hardening_policy_inventory`, `hardening_snapshot_projection`, `leak_sweep`, `memory_modes`, `memory_overcommit`, `pending_resolver`, `permit_lifecycle`, `prop_invariants`, `provenance`, `provenance_preservation`, `recursive_rejection`, `reduce_policy_validation`, `request_key_hot_path`, `retry_after`, `retry_after_property`, `substrate_inventory` | `loom_governance` (`loom`), `shuttle_governance` and `model_replay_fixture` (`shuttle`) via `modelcheck`; selected race binaries via `tsan` |
| `taskmesh` (25) | `cancellation_policy`, `config_inventory`, `deadline_cancel`, `e2e_chaos`, `e2e_proof`, `e2e_scenarios`, `hardening_consumer_surface`, `hardening_deadline_custody`, `hardening_degrade_policy`, `hardening_dispatch_resolution`, `hardening_drain`, `hardening_executor_authority`, `hardening_executor_protocol`, `hardening_intake_bounds`, `hardening_nested_wait`, `host_inferno`, `local_runtime_guard`, `runtime_blocking`, `runtime_cancel_leak`, `runtime_cancel_timeout`, `runtime_cpu`, `runtime_cpu_executor`, `runtime_io`, `runtime_local`, `substrate_enforcement` | `test-rayon` selects host library and the exact Rayon-only case in `hardening_executor_authority`; selected host binaries via `tsan` |
| `taskmesh-rayon` (1) | `rayon_smoke` | `test-rayon` proves the host feature bridge; `tsan` selects this adapter package |
| `taskmesh-bench` (3) | `fairness_property`, `hellgate`, `inferno`; excluded from `test-core` | `bench-smoke` selects eight Criterion benches; `bench-iai` selects feature-gated IAI on Linux |

Use `just dev-rust-tests <owner> <consumers...>` for test-only edits and
`just dev-rust-fast <owner> <consumers...>` for production Rust edits; the
latter adds owner production Clippy. Conservative consumer closures are:
contract -> engine, host, Rayon, bench; engine -> host, bench; host -> bench;
Rayon -> host with `just test-rayon`. Recompute these from Cargo metadata
after dependency changes. `test-rayon` does not replace default Rayon adapter
tests. The current TSan script deliberately selects three engine binaries,
host library plus eleven host binaries, and the adapter package; it is a
targeted race proof, not all 72 default integration targets.

The current catalog discovers 23 Python test modules for `just py-test`:

| Owner directory | Modules | Owner-local and CI mapping |
|---|---|---|
| `tools/arch/tests` | `test_crate_boundaries`, `test_manifest_feature_ownership` (2) | Owner pytest + real `test-architecture`; CI `py-lint`, `py-test`, `test-architecture` |
| `tools/bench/tests` | `test_allocation_gate`, `test_iai_gate`, `test_workflow_policy` (3) | Owner pytest; real `bench-gate` for producer/threshold changes; CI `py-lint`, `py-test`, `bench-gate`; deep `bench-iai` separately |
| `tools/consumer-msrv/tests` | `test_consumer_msrv` (1) | Owner pytest; real `consumer-msrv` for invocation/toolchain changes; CI `py-lint`, `py-test`, `consumer-msrv` |
| `tools/fuzz/tests` | `test_evidence` (1) | Owner pytest; `fuzz-check` for harness/API changes; CI `py-lint`, `py-test`, `fuzz-check`; deep `fuzz` separately |
| `tools/gates/tests` | `test_batch_supervisor`, `test_execution_evidence`, `test_inventory`, `test_scenario_evidence`, `test_target_catalog` (5) | Owner pytest + real `gates-inventory`; CI `py-lint`, `py-test`, `gates-inventory` |
| `tools/modelcheck/tests` | `test_run` (1) | Owner pytest; CI `py-lint`, `py-test`; deep `modelcheck` separately |
| `tools/pm/tests` | `test_check` (1) | Owner pytest + `prompt-check` after policy changes; CI `py-lint`, `py-test` |
| `tools/qualification/tests` | `test_evidence`, `test_plan_bookkeeping`, `test_receipt` (3) | Owner pytest; CI `py-lint`, `py-test`; actual qualification receipt separately |
| `tools/release/tests` | `test_release` (1) | Owner pytest; CI `py-lint`, `py-test`; actual release receipt separately |
| `tools/semgrep/tests` | `test_rules_fire` (1) | Owner pytest + real `semgrep`; CI `py-lint`, `py-test`, `semgrep` |
| `tools/verification/tests` | `test_policy_recipe_drift`, `test_run_focused_mutants`, `test_run_generated_mutants`, `test_run_mutations` (4) | Owner pytest; CI `py-lint`, `py-test`; deep mutation producers separately |

For Python changes, run `just dev-python-lint tools/<owner>` and
`just dev-python-tests tools/<owner>/tests`; `just dev` omits Python.
`dev-python-tests` excludes the `qualification` mark:
`test_iai_gate` is wholly marked and omitted locally, while CI `py-test`
selects it. `test_allocation_gate` has one `slow` case; neither command
excludes `slow`. Record collected and executed case counts so an existing
module with an empty/fully skipped selection does not look like a pass.

The standalone fuzz workspace declares exactly three bins:
`admission_churn`, `policy_topology`, and `wire_formats`. `fuzz-check`
uses stable Clippy `--all-targets`; `fuzz` uses `cargo fuzz list` and
verifies the producer's expected target set before bounded execution. The
benchmark package declares eight default Criterion benches:
`admit_release`, `composite_reduce`, `contention`, `governance_tax`,
`host_edge_paths`, `multiclass_fairness`, `overload_stability`, and
`retrieval_saturation`. `bench-smoke` runs them with `--test`; it has no
performance verdict. `iai_governance` requires `iai` and belongs to
`bench-iai` with a compatible baseline. A fresh baseline is
`BASELINE_CREATED`, not a regression-qualified PASS.

The remaining deep producers have narrower or different denominators:

| Gate | Exact selection / result meaning |
|---|---|
| `modelcheck` | Engine `loom_governance`, `shuttle_governance`, `model_replay_fixture`; `tools/modelcheck/producer-manifest.json` requires five Loom models, seven seeded Shuttle models, and one reproduced failure. A compiled model target alone is not model evidence. |
| `tsan` | Engine `concurrency_stress`, `concurrency_fuzz`, `hardening_effect_retirement`; host library plus `runtime_cpu_executor`, `deadline_cancel`, `hardening_intake_bounds`, `runtime_cancel_leak`, `e2e_chaos`, `hardening_executor_protocol`, `hardening_deadline_custody`, `runtime_cancel_timeout`, `hardening_drain`, `hardening_nested_wait`, `host_inferno`; adapter package. Nightly compiler and `rust-src` required. |
| `fuzz` | All three declared fuzz bins, each with bounded execution, saved raw logs/corpus identity and semantic witness validation. This is not the stable `fuzz-check` compile. |
| `mutants-critical` | Curated exact oracle per entry in `tools/verification/mutations.json`; control survivor and prescribed failure sets have separate meanings. |
| `mutants-generated` | Generated cargo-mutants denominator and its own completion evidence; never merge its count with curated mutations. |
| `coverage-report` | `cargo-llvm-cov` library workspace excluding bench and doc fixture; reports percentages without threshold or qualification claim. |
| `bench-iai` | Linux `iai_governance`, configured runner and compatible baseline; only the qualified comparison satisfies regression proof. |

## Change-to-check routing

Route edits from **the changed owner** and the dependency graph, not by a file
name substring or a global skip list. Unknown, ambiguous, or policy/tooling
changes fall back to the broader `dev` checkpoint and the full clean CI profile.
Impact selection may reduce the edit loop; it cannot remove a CI-required gate.

| Changed owner | Local minimum | Additional clean CI authority |
|---|---|---|
| `taskmesh-contract` public/wire policy | Contract tests, affected engine/host/Rayon/bench consumer tests, format/lint | Full workspace, feature/docs/MSRV matrix; release semver and API/wire adjudication when publishing |
| `taskmesh-engine` governance | Engine unit/integration and host/bench consumers; focused negative oracle for changed slice | Full workspace; modelcheck/TSan remain separate deep proofs for synchronization or lifecycle risk |
| `taskmesh` host facade | Host integration and bench consumers; feature bridge tests for Rayon edits | Full workspace, `test-rayon`, docs/MSRV where public surface changed |
| `taskmesh-rayon` adapter | Adapter tests and host Rayon-feature path | Workspace `test` plus `test-rayon` host feature matrix |
| `taskmesh-bench` | Changed harness tests or benchmark diagnostics | Workspace bench tests, `bench-smoke`, allocation gate as applicable; IAI remains Linux deep proof |
| `tools/**` Python, Semgrep, architecture, gates, receipt, release | Changed Python files and owner tests; real producer smoke if its CLI/schema changed | Full `py-lint`/`py-test` and registered static gates; producer evidence only in its authorized deep profile |
| `fuzz/**`, docs, workflows, gate policy, `Cargo.lock` | Target compile/lint, docs/owner checks, inventory parity, dependent crates | Full clean CI profile. Release-only authority stays separate; a workflow edit cannot self-authorize a missing required gate |

### Path-specific edit-loop rules

These are **minimum diagnostics**; clean CI still executes its 16 required
gates without path filtering. Run the union when a change spans rows.

| Changed paths | Immediate command / selected target | Escalate when |
|---|---|---|
| `crates/taskmesh-contract/{src,tests}/**` | `dev-rust-fast` or `dev-rust-tests taskmesh-contract taskmesh-engine taskmesh taskmesh-rayon taskmesh-bench` | Public/wire signature: `test-rayon`, `doctest`, `rustdoc`, `consumer-msrv`; release only when publishing |
| `crates/taskmesh-engine/{src,tests}/**` | Same owner recipe with `taskmesh-engine taskmesh taskmesh-bench`; focus the changed semantic/negative integration binary first | Shared synchronization, lifecycle, memory: inspect model/TSan owner coverage; full campaigns remain separately authorized |
| `crates/taskmesh/{src,tests}/**` | Same owner recipe with `taskmesh taskmesh-bench`; `test-rayon` for bridge/feature code | Public docs/MSRV matrix or real race-risk change |
| `crates/taskmesh-rayon/{src,tests}/**` | Same owner recipe with `taskmesh-rayon taskmesh`, plus `test-rayon` | Host feature behavior changes; inspect TSan target ownership |
| `crates/taskmesh-bench/{src,tests,benches}/**`, `tools/bench/**` | Bench package tests / owner pytest; `bench-smoke` for bench definitions, `bench-gate` for allocation code/threshold | Linux IAI definition/baseline changes need a separately authorized matched comparison |
| `tools/<owner>/**/*.py` or `tools/<owner>/tests/**` | Owner Ruff + owner pytest; invoke the real registered producer/checker if its entry point or schema changed | Changes in shared `tools/qualification`, `tools/gates`, `tools/process_supervisor.py` require `gates-inventory` and downstream receipt fixture tests; deep producers still separate |
| `tools/semgrep/**`, `.semgrepignore` | Semgrep owner pytest plus real `semgrep` and enrollment negative fixture | Rule/scan-scope changes can drop a whole test subtree or one file within it |
| `fuzz/**` | `fuzz-check`, fuzz owner pytest for corpus/evidence changes | Do not treat compile as semantic fuzz execution |
| `Cargo.toml`, `Cargo.lock`, `rust-toolchain*`, `config/deny.toml`, `config/clippy-*.txt` | `just dev`, `deny`, `fuzz-check`; recalculate crate consumer closure and feature target census | `consumer-msrv` for library dependency/MSRV changes; all CI authority on clean source |
| `Justfile`, `tools/gates/{inventory,required}.json`, `tools/gates/run.py`, `.github/workflows/**`, `.githooks/**` | `gates-inventory`, gates owner pytest; compare recipe expansion and required IDs | Any trigger/check-name change requires hosted branch-rule review; no unqualified self-approval |
| `README.md`, `docs/**`, `crates/taskmesh-doc-examples/**`, public rustdoc | `doctest`, `rustdoc`, doc-example library test; `prompt-check` for prompt policy docs | Wire/API wording changes require contract tests and release adjudication on publication |
| Unclassified or mixed ownership | `just dev` plus Python owner checks where applicable | Full CI always applies; add an owner rule only with a reviewed negative selection case |

Static gate ownership is exact: `fmt-check` covers workspace Rust format;
`clippy-core` covers production library/bin lint in `dev`; full `clippy`
adds test/example/bench lint, Rayon production pass, doc fixture and bench
package in CI. `py-lint` covers `tools/` with Ruff. `semgrep` scans governed
Rust source/test paths and validates enrollment. `test-architecture` checks
crate/manifest architecture. `gates-inventory` checks declared gate/workflow/
required-set parity. `deny` checks dependencies and advisories.
`fuzz-check` lints all standalone fuzz bins. `rustdoc` rejects documentation
warnings, while `doctest` executes examples. None of those substitutes for
its neighbor's evidence.

For every new/renamed path, the implementation must derive an owner from
Cargo metadata, tracked Python module discovery, or an explicit policy path.
An unknown owner fails the mapping audit. Test-path selection must be tested
with a changed test-only file, feature-gated test, new fuzz bin, renamed pytest
module, support `mod.rs`, and a workflow-only change. The expected result is
the **same or broader** local selection and unchanged full CI membership.

## Catalog and receipt invariants to implement

1. Generate Rust target discovery from Cargo metadata and the selected test
   runner's machine-readable compiled-target list (`cargo test --no-run
   --message-format=json` for the Cargo path, nextest's binary list for its
   path), including feature-gated exceptions. Enumerate tracked
   `tools/**/tests/test_*.py` and use pytest collection for Python ownership.
   Reuse the ordinary test build/target directory rather than compiling a
   second workspace solely for the census. Do not manually maintain every
   test function. CI must account for every current target; a new/renamed test
   target requires an explicit owner or fails the mapping check. This census
   is compile/discovery evidence, not a test PASS.
2. Document `finding-proof-spec.json.required_gate` as the related ordinary
   gate that release additionally requires to PASS. Keep its current semantics
   and the separate focused witness command. For test-execution traceability,
   derive the executing CI gate from the Cargo/pytest target census instead of
   inferring it from `required_gate`. Require a negative fixture proving the
   target census rejects a plausible but false executor mapping.
3. Compare Semgrep's scanned paths to all tracked Rust integration-test files
   governed by the rule pack, with an explicit, reviewed exclusion list if a
   file is intentionally outside scope. Add a same-tree partial-omission
   negative case. Do not add a second real-tree Semgrep scan inside pytest.
4. A selected test gate must report nonzero discovered/executed cases and fail
   on a missing required target. `#[ignore]`, pytest skip/xfail, platform/tool
   absence, and test filtering must not silently satisfy a required witness.
   Preserve current platform `SKIPPED_PLATFORM` versus missing-tool `NOT_RUN`
   distinction.
5. Record the actual Cargo/nextest choice and version, rustc/toolchain, Python,
   Semgrep, and advisory snapshot where they affect the result. Caches are
   performance inputs, not proof artifacts. Source/manifest/tool identity must
   bind the receipt; stale or partial receipts remain diagnostic.
6. Keep `local`/`dev` edit feedback distinct from clean-source `ci`, deep proof,
   and final release. Bounded static parallelism is already four workers;
   serialize Cargo builds and isolated mutation/TSan/performance campaigns.
7. Strengthen the weighted fairness property with a complete-drain assertion
   or narrow its stated claim to repeatability. Keep the independent exact
   weighted-order test as the semantic oracle; do not merge two checks into a
   self-comparison helper.

Do not add `prompt-check`, `loom`, or `shuttle` as duplicate required gates:
`py-test` exercises the current prompt check, and registered `modelcheck`
owns Loom/Shuttle plus replay. `coverage-report` and `bench-smoke` retain their
documented report/smoke meanings.

## Implementation sequence and acceptance

| Wave | Files and owner | Acceptance evidence | Stop/reopen condition |
|---|---|---|---|
| W0: freeze and measure | Read-only capture of HEAD/dirty/source digest; `Justfile`, `inventory.json`, `required.json`, existing gate-duration receipts | Static target inventory; measured median/p95 for ordinary gates on an uncontended host before any cost-based move. Deep-gate costs use existing receipts until a separately authorized campaign | Source or dirty fingerprint changes, timings mix hosts/toolchains, or W0 starts mutation testing |
| W1: source-backed gaps | `tools/semgrep/check.py`, `tools/semgrep/tests/test_rules_fire.py`, `crates/taskmesh-engine/tests/prop_invariants.rs`; clarify `required_gate` in release proof documentation | Same-tree Semgrep omission is rejected; weighted drain property cannot pass on a stuck queue; release witness domain/executor meanings are explicit | A second real-tree scan is added, or the fairness change weakens the exact reference oracle |
| W2: stage contract | `docs/release-checklist.md`, `Justfile`, and, only if needed, `tools/gates/inventory.json`, `tools/gates/required.json`, `tools/gates/validate_inventory.py` | `local`/`dev` diagnostic scope and CI/deep receipt authority are explicit; all 24 inventory IDs have one purpose and required status; no redundant gate list | A new alias executes campaigns on ordinary Mac feedback, or CI membership drifts |
| W3: target census and receipts | Add `tools/gates/target_catalog.py` and `tools/gates/tests/test_target_catalog.py`; integrate with `tools/gates/validate_inventory.py`, `tools/gates/tests/test_inventory.py`; touch `Justfile`, `tools/gates/run.py`, `tools/qualification/receipt.py` and their tests only for the selected runner/receipt contract | New test targets and feature-required targets cannot disappear from CI; receipt records runner choice and remains source-bound; existing failure/skip states retain meaning | Empty selection, skipped required witness, or edited summary can pass |
| W4: hosted adoption decision | New bounded `.github/workflows/pr-ci.yml` or an equivalent explicit split; `tools/gates/validate_inventory.py` and its tests; branch rules only after runner/budget decision. Leave current full manual `ci.yml` manual unless separately refactored | Recommended target when capacity exists: an automatic bounded 16-gate PR check, with one unskippable aggregate verdict, read-only token, pinned actions, exact SHA, PR concurrency cancellation, post-merge main check, and `merge_group` if merge queue is used. Deep campaigns remain isolated and outside ordinary PR requirements | Current full `ci.yml` merely gains a PR trigger; required workflow can be skipped by path filtering; a child job's `skipped` result is treated as proof; automatic work exceeds budget; or a hosted result lacks exact SHA |
| W5: qualification | Uncontended clean candidate on macOS for CI profile; separate Linux full release candidate when explicitly authorized | `verify-macos-ci` current-HEAD receipt; full Linux ordinary plus release-only artifacts only after authorized deep campaigns | Any dirty/source drift, `NOT_RUN`, stale receipt, partial mutation, or Linux-only skip is promoted to full release PASS |

W0-W4 are design/implementation waves. This proposal does **not** authorize
mutation campaigns or final release qualification. Do not move a test to a
cheaper stage based on inspection counts alone: require measured cost, a
preserved CI/deep authority, and a negative/semantic oracle where the test is
claiming a production invariant.

W1 and the W3 static slice were tracked by `934230d`, whose clean-source local
macOS CI-profile receipt passed all 16 required gates. The later clean
`3700f3f4dd2e4b783e50ca2e3a8db9b11eba39b0` receipt also passed 16/16 and
validated against that HEAD; its durable copy is under
`/Users/songmin/.codex/artifacts/taskmesh-ss-3700f3f/macos-gates.json`.
Neither receipt qualifies subsequent edits, nightly, or release.

The later clean `2a2f9bd688fd249f26d1a80b0fb035669530fbad` completed
all 16 CI-profile gates. Its source-bound receipt is
`/Users/songmin/.codex/artifacts/taskmesh-ss-2a2f9bd/macos-gates.json`
(SHA-256 `0cc13daf39ac4537b40d619c0b06aa942bb9a5000a5946681914911e4222d57a`).
A subsequent documentation commit `bca879a621f7780de01127a6cc694021a54ae356`
also completed 16/16 and validated against its exact HEAD; its receipt is
`/Users/songmin/.codex/artifacts/taskmesh-ss-bca879a/macos-gates.json`
(SHA-256 `8d21a9adc6fdb1b98bf6840f32857bbde282b74eaaafabf6dbc7f186d98843aa`).
For any later checkout, qualify the current HEAD independently with
`just verify-macos-ci` and `uv run python tools/gates/run.py --validate-receipt
target/verification/macos-gates.json --expected-head "$(git rev-parse HEAD)"`.

The historical W3 static slice made `gates-inventory` derive 113
Cargo/Python/fuzz target records and rejects unowned feature targets or
missing declared recipe selectors. The `test` gate reports its selected
runner/version, and saved PASS validation requires those fields. This does
not establish compiled target selection, case execution, pytest collection,
or a reusable receipt for a different source. The W3 implementation extends
that slice as follows:

- `test` checks Cargo metadata ownership against the nextest binary list and
  verifies every selected non-ignored case started and ended `ok`. It runs
  the regular workspace and doc fixture separately so features do not unify.
  The two existing zero-case library binaries are explicit exceptions; an
  unknown empty target fails. The clean CI run selected 94 targets and
  completed 632 cases.
- `test-rayon` checks its ten explicit feature/exact scopes, including the
  nonzero exact-filter denominator. The focused run selected seven distinct
  targets and completed 33 cases. Filtered-out cases are disclosed in each
  scope and never counted as executed.
- `py-test` collects modules and case IDs under strict markers, requires every
  collected case to pass, and includes `slow` and `qualification` marks. The
  current module count is derived from source, not a second allowlist.
- Both runner summaries bind catalog, selected/executed case, command, and
  runner digests in the gate's saved status line. The local receipt validator
  re-derives the current catalog digest and still enforces exact clean HEAD.
  The static catalog currently discovers 142 records, including ten explicit
  Rayon matrix selectors; this is a snapshot,
  not an authored denominator.
- The new negative fixtures cover added ordinary/model/fuzz targets, Python
  module renaming, empty/ignored/unfinished Rust selections, edited summary
  fields, and bounded workflow trigger bypasses. Existing Semgrep and
  required-set parity fixtures remain in force.

The counts above were recorded by the exact-HEAD CI profile, not merely by
static mapping. Each new commit still needs its own CI-profile receipt.
`doctest`, `bench-smoke`, modelcheck, fuzz, and other
special producers retain their separate gate/producer contracts rather than
being counted as ordinary nextest or pytest cases.

### W3 implementation contract

1. `target_catalog.py` computes a source-bound catalog from root and fuzz
   Cargo metadata plus tracked Python modules. Each record has
   `kind/package/target/path/required_features/owner/executing_gate`; support
   modules are excluded as targets. Static policy owns the small exceptional
   edges: three model targets -> `modelcheck`, Rayon feature case ->
   `test-rayon`, two 64-bit-only cases -> platform conditions, eight Criterion
   targets -> `bench-smoke`, IAI ->
   `bench-iai`, three fuzz bins -> `fuzz-check` and `fuzz`. Do not copy
   the 75 names above into a second committed allowlist. Unknown target kind,
   feature, owner or executor fails closed.
2. Compare declared targets with actual selected binaries for the exact
   qualified `test`/matrix commands. The runner's list is discovery evidence;
   the run's process status and structured per-binary/case result are execution
   evidence. During W0 choose one pinned qualified runner; retain Cargo
   fallback as diagnostic unless it can emit equally complete result evidence.
   A harmless changed-target fixture must prove the selected result count is
   nonzero and matches the discovered denominator. Do not count ignored,
   skipped, filtered or unstarted cases as execution.
3. Collect pytest cases under the CI invocation and compare module ownership
   against the dynamically discovered tracked modules. Preserve the local `not qualification`
   choice and prove CI includes marked cases. Check `slow` explicitly and
   treat a whole-module skip/xfail as a stated exclusion rather than a pass.
   Enable pytest strict-marker validation in both local and CI recipes so a
   misspelled `slow` or `qualification` marker fails collection. `prompt-check`
   stays an indirect `py-test` behavior witness.
4. Bind catalog/selection digest, runner identity, command args and source
   identity in the existing CI receipt, without granting a new catalog
   artifact qualification authority of its own. Keep exact-HEAD validation,
   platform skip semantics, producer envelope checks, and independent
   `required.json` membership.
5. Add negative fixtures to `test_target_catalog.py` and existing gate
   tests: a new ordinary integration binary, a feature-required model target
   with no executor, a new fuzz bin, a renamed Python module, an empty test
   filter, an all-skipped selected module, a `mod.rs` support file, a
   same-tree Semgrep omission, and a workflow-only edit. Each should either
   select its declared authority or fail with the missing owner/target. A
   CI-required gate deleted from inventory must still fail parity validation.

### W4 hosted rollout boundary

The validator now permits `pull_request` and main `push` only in the dedicated
`pr-ci.yml`. It requires one unconditional job, the clean `ci` profile, exact
`$GITHUB_SHA` receipt validation, pinned actions, read-only token, no path
filter, and PR-only cancellation. `ci.yml` and all deep producers stay manual.
The job is a single aggregate verdict, so there is no child-job skip or
artifact-handoff success path to mistake for qualification.

The workflow is only staged locally and is deliberately **not required** in
branch protection. Local gate duration varied sharply under unrelated Cargo
and mutation work on the shared host; that is not a valid hosted median/p95 or
runner-capacity measurement. After a push is authorized, obtain non-required
PR and main trial runs, record queue/median/p95 and exact-SHA receipt, then
decide the merge rule. Add `merge_group` only if merge queue is in use. No
scheduled deep subset is configured.

## 2026 reference points

- [Cargo test target and feature selection](https://doc.rust-lang.org/cargo/commands/cargo-test.html): `--lib`, `--tests`, `--doc`, and features select different surfaces.
- [Cargo metadata](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html) and [Cargo JSON artifacts](https://doc.rust-lang.org/cargo/reference/external-tools.html): derive declared targets and actual compiled targets without hand-maintaining test names.
- [cargo-nextest machine-readable test and binary lists](https://nexte.st/docs/machine-readable/list/): use the binary list for selection parity; listing is discovery, not execution proof.
- [pytest marks and strict markers](https://docs.pytest.org/en/stable/how-to/mark.html): marks require explicit `-m` selection and can be checked against misspellings.
- [GitHub workflow triggers](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows): PR, manual, merge queue, and schedule are distinct events.
- [GitHub required-check skip behavior](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/skip-workflow-runs): a path-filtered required workflow can remain Pending.
- [GitHub Actions secure use](https://docs.github.com/en/actions/reference/security/secure-use): least privilege and full commit-SHA action pins.
- [GitHub Actions workflow concurrency](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax): cancel superseded PR runs while protecting full proof/release campaigns from unwanted cancellation.
