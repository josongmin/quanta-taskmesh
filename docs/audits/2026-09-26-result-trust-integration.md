# Result-trust integration review — 2026-09-26

## Source boundary

- Remote base: `7253fab2002f15a65a31a2070641f0c18498912f`.
- Latest local main integrated: `f00c03939ba840d2e287712bbb98b9d5c10a0e1e`.
- Inherited population: 95 commits and 119 changed files (the original 84 commits plus eleven concurrent main commits).
- Result-trust changes were integrated at `e355d93ab19f45663a387d2113d12c7dff88812e` before this additional review.
- The four later benchmark commits were integrated at `e4c261a`: sealed artifact/tail inputs, declared attempt populations, bounded build supervision, effective Cargo profiles, and Rust 1.81 compatibility were retained.
- The review concerns reachable lint, CI, ratchet, source custody and raw-evidence failures. It is not performance calibration, mutation qualification, or release approval.

## Integration defects and structural repairs

| Boundary | Confirmed defect | Repair |
|---|---|---|
| Host source identity | Git index hints hid edits; changing dirty source A to dirty B preserved identity | Common byte/mode check against HEAD plus source-content digest across build, launch, completion and admission; old provenance rejects |
| Frozen host build tools | Version strings and config/environment hashes missed same-path compiler/linker replacement, relative Cargo home settings, and changes during rebuild/oracle execution | Resolve relative Cargo home from the actual build directory and share executable resolution with IAI; build witness schema v3 binds the effective frozen-source tool context before and after each build, verification and oracle |
| IAI build tools | Replacing compiler/wrapper/linker bytes at the same path preserved compatibility context | Executable identities in context, unresolvable controls reject, versioned evidence/cache invalidation |
| Frozen tool descriptions and oracle environment | Version descriptions used the original repository toolchain and the oracle inherited ambient incremental compilation | Resolve version descriptions from the attested compiler/Cargo in frozen source; oracle inherits the frozen effective environment before its explicit, verified test-profile overrides |
| Complete raw run | `drain_ok=false` disabled conservation/retained-ownership checks | Complete evidence requires settled conservation and no held capacity |
| Raw capability inventory | Arbitrary zero-valued capability names could replace the actual runtime catalog | Bind raw final and sampled capability catalogs to freshly resolved runtime topology |
| Integrated local validator | Latest main used `Option::is_none_or` beyond declared Rust 1.81 MSRV and a manual checked division rejected by Clippy | Preserve validation semantics with MSRV-compatible `map_or` and `checked_div` |
| Bounded property execution | One test bundled 3,000 fairness seeds; another bundled all 25 hellgate combinations, exceeding the fixed per-test budget on a heavily loaded host | Six nonoverlapping fairness populations cover seeds 0..3000; five hellgate tests cover the original five seeds and every original regime. Assertions and timeout remain unchanged; all cases enter the mandatory nextest denominator |

## Verification boundary

- Owner tests are focused correctness evidence. The final CI receipt must be generated after all source edits are committed.
- Local macOS proof and hosted synthetic-merge/main proof are separate exact-source receipts.
- Every bounded CI gate must run and pass; no skipped/xfailed cases, missing rows, stale receipts or copied verdicts admit qualification.
- Existing measured-series inputs remain explicit: frozen B00 values, quiet-host repeated observations, privacy-safe H7 profile, equivalent peer and independent rerun. Code completion cannot supply those measurements.

## Inherited file inventory

Each file below belongs to the stated review lane. Runtime and benchmark findings are repaired at their common boundaries; static policy/configuration remains checked by inventory, lint, architecture and exact-source CI. No inherited commit is silently omitted from the integrated branch.

### Rust and Cargo (45 files)

- `Cargo.lock`
- `crates/taskmesh-bench/Cargo.toml`
- `crates/taskmesh-bench/benches/cold_host_lifecycle.rs`
- `crates/taskmesh-bench/benches/governance_tax.rs`
- `crates/taskmesh-bench/benches/multiclass_fairness.rs`
- `crates/taskmesh-bench/benches/overload_stability.rs`
- `crates/taskmesh-bench/benches/queue_scaling.rs`
- `crates/taskmesh-bench/benches/retrieval_saturation.rs`
- `crates/taskmesh-bench/examples/host_closed_loop_probe.rs`
- `crates/taskmesh-bench/examples/host_composite_probe.rs`
- `crates/taskmesh-bench/examples/host_generator_probe.rs`
- `crates/taskmesh-bench/examples/host_load_probe.rs`
- `crates/taskmesh-bench/examples/host_local_probe.rs`
- `crates/taskmesh-bench/examples/host_scenario_validate.rs`
- `crates/taskmesh-bench/examples/host_special_validate.rs`
- `crates/taskmesh-bench/src/artifact.rs`
- `crates/taskmesh-bench/src/closed_loop.rs`
- `crates/taskmesh-bench/src/composite_host.rs`
- `crates/taskmesh-bench/src/generator_calibration.rs`
- `crates/taskmesh-bench/src/host_load.rs`
- `crates/taskmesh-bench/src/host_scenarios.rs`
- `crates/taskmesh-bench/src/lib.rs`
- `crates/taskmesh-bench/src/loadgen.rs`
- `crates/taskmesh-bench/src/local_host.rs`
- `crates/taskmesh-bench/src/minimal_host.rs`
- `crates/taskmesh-bench/tests/artifact_publish.rs`
- `crates/taskmesh-bench/tests/host_closed_loop.rs`
- `crates/taskmesh-bench/tests/host_composite.rs`
- `crates/taskmesh-bench/tests/host_generator_calibration.rs`
- `crates/taskmesh-bench/tests/host_load_accounting.rs`
- `crates/taskmesh-bench/tests/host_load_integration.rs`
- `crates/taskmesh-bench/tests/host_local.rs`
- `crates/taskmesh-bench/tests/host_minimal_recorder.rs`
- `crates/taskmesh-contract/tests/task_plan_validation.rs`
- `crates/taskmesh-engine/src/features/fairness/scheduler.rs`
- `crates/taskmesh-engine/src/shared/mod.rs`
- `crates/taskmesh-engine/tests/identity_authority.rs`
- `crates/taskmesh-engine/tests/request_key_hot_path.rs`
- `crates/taskmesh-engine/tests/shuttle_governance.rs`
- `crates/taskmesh/src/ingress.rs`
- `crates/taskmesh/src/runtime.rs`
- `crates/taskmesh/tests/e2e_chaos.rs`
- `crates/taskmesh/tests/e2e_scenarios.rs`
- `crates/taskmesh/tests/hardening_executor_protocol.rs`
- `crates/taskmesh/tests/strict_ingress.rs`

### Benchmark acquisition and admission (53 files)

- `tools/bench/generator_run.py`
- `tools/bench/host_aa.py`
- `tools/bench/host_admission.py`
- `tools/bench/host_build.py`
- `tools/bench/host_compare.py`
- `tools/bench/host_control_assess.py`
- `tools/bench/host_controls.py`
- `tools/bench/host_observer.py`
- `tools/bench/host_perf.py`
- `tools/bench/host_recorder.py`
- `tools/bench/host_run.py`
- `tools/bench/host_sampler.py`
- `tools/bench/host_special_run.py`
- `tools/bench/host_special_verify.py`
- `tools/bench/host_study.py`
- `tools/bench/minimal_run.py`
- `tools/bench/process_resource.py`
- `tools/bench/scenario_grid.py`
- `tools/bench/scenario_rate.py`
- `tools/bench/scenarios/claim-contract.json`
- `tools/bench/scenarios/h1-blocking-rate-template.json`
- `tools/bench/scenarios/h1-cpu-rate-template.json`
- `tools/bench/scenarios/h1-default-send-smoke.json`
- `tools/bench/scenarios/h1-io-closed-loop-smoke.json`
- `tools/bench/scenarios/h1-io-rate-template.json`
- `tools/bench/scenarios/h2-burst-recovery-smoke.json`
- `tools/bench/scenarios/h3-weighted-smoke.json`
- `tools/bench/scenarios/h4-cpu-blocking-smoke.json`
- `tools/bench/scenarios/h4-local-controls-smoke.json`
- `tools/bench/scenarios/h4-local-smoke.json`
- `tools/bench/scenarios/h4-requested-stack-smoke.json`
- `tools/bench/scenarios/h5-custody-smoke.json`
- `tools/bench/scenarios/h6-composite-smoke.json`
- `tools/bench/scenarios/index.json`
- `tools/bench/tests/test_claim_contract.py`
- `tools/bench/tests/test_generator_run.py`
- `tools/bench/tests/test_host_aa.py`
- `tools/bench/tests/test_host_acquisition_identity.py`
- `tools/bench/tests/test_host_admission.py`
- `tools/bench/tests/test_host_artifact_write.py`
- `tools/bench/tests/test_host_build.py`
- `tools/bench/tests/test_host_compare.py`
- `tools/bench/tests/test_host_control_assess.py`
- `tools/bench/tests/test_host_controls.py`
- `tools/bench/tests/test_host_observer.py`
- `tools/bench/tests/test_host_perf.py`
- `tools/bench/tests/test_host_recorder.py`
- `tools/bench/tests/test_host_sampler.py`
- `tools/bench/tests/test_host_special.py`
- `tools/bench/tests/test_host_study.py`
- `tools/bench/tests/test_minimal_run.py`
- `tools/bench/tests/test_process_resource.py`
- `tools/bench/tests/test_scenario_grid.py`

### Verification tooling (13 files)

- `Justfile`
- `tools/arch/check_crate_boundaries.py`
- `tools/arch/tests/test_crate_boundaries.py`
- `tools/arch/tests/test_manifest_feature_ownership.py`
- `tools/coverage/report.sh`
- `tools/gates/execute_rust_tests.py`
- `tools/gates/rust_test_evidence.py`
- `tools/gates/tests/test_execute_rust_tests.py`
- `tools/verification/mutations.json`
- `tools/verification/run_focused_mutants.py`
- `tools/verification/run_generated_mutants.py`
- `tools/verification/tests/test_run_focused_mutants.py`
- `tools/verification/tests/test_run_generated_mutants.py`

### Documentation and environment (8 files)

- `.config/nextest.toml`
- `CHANGELOG.md`
- `docs/adr/9000-benchmark-strategy.md`
- `docs/benchmarks/host-series.md`
- `docs/plans/sep-25-taskmesh-benchmark/tickets/B04-remaining-audit.md`
- `docs/plans/sep-25-taskmesh-benchmark/tickets/README.md`
- `pyproject.toml`
- `uv.lock`
