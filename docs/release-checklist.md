# Release Checklist — first stable (0.1.x)

## Proof gate (must be green)

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo check --workspace`
- [ ] `cargo test --workspace` (incl. proptest invariants + OS-thread stress)
- [ ] `cargo test -p taskmesh --features rayon` (feature-wire path)
- [ ] `cargo test --doc -p taskmesh`
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace`
- [ ] `cargo bench -p taskmesh-bench -- --test` (benches smoke-run)
- [ ] `RUSTFLAGS="--cfg loom" cargo test -p taskmesh-engine --test loom_governance` (concurrency model-check)
- [ ] hell-gate e2e: `crates/taskmesh/tests/e2e_proof.rs`, `e2e_scenarios.rs`

## Invariant proofs

- Resource accounting `Σpermit == held` and per-class==global enforced by
  `GovernedState::assert_consistent` (debug) on every grant/unwind/reconcile.
- `prop_invariants.rs`: drains-to-zero + cap-respect (200 cases) and fairness
  determinism (120 cases).
- `loom_governance.rs`: exhaustive interleaving — unique ids, no leak, inflight→0.

## Required proof scenarios

1. unknown class reject — `e2e_proof`, `admission_queue`
2. max inflight saturation — `admission_queue`, `e2e_proof`
3. queue depth saturation — `admission_queue`, `e2e_proof`
4. retry-after fixed/adaptive — `retry_after`, `e2e_proof`
5. memory overcommit reject/queue/degrade — `memory_overcommit`, `e2e_proof`
6. child permit root attribution — `composite_root_attribution`, `e2e_proof`
7. recursive admission rejection — `recursive_rejection`, `e2e_proof`
8. deterministic reduce enforcement — `reduce_policy_validation`, `e2e_proof`
9. `run_local` non-`Send` path — `runtime_local`, `e2e_proof`
10. snapshot class/resource/substrate state — `substrate_inventory`, `e2e_proof`
11. config validation on impossible budgets — `config_validation`, `e2e_proof`
12. docs example compile — `taskmesh` lib doctest

## API & docs sync

- [ ] Public type names unchanged: `TaskSpec`, `TaskClass`, `TaskStage`,
      `SubstrateHint`, `AdmissionVerdict`, `GovernorError`, `RunError`,
      `Snapshot`, `SubstrateRecord`, `Runtime`.
- [ ] Zero placeholder public types.
- [ ] README / library-spec / external-interface describe the same contract.
- [ ] `taskmesh-rayon` included; workspace green with and without the `rayon`
      feature.
- [ ] Crate versions in sync (workspace `version`).

## Semver scope

- Public surface = `taskmesh` re-exports (contract + selected engine items).
  Adding ports/verdict variants is additive; renames are breaking.

## Known residue (intentional, tracked)

- `BlockingPoolCpuExecutor` is the default CPU executor; Rayon is opt-in. The
  blocking-pool path is a *residual* default behind the port, not a final shape
  claim (ADR 9000 / T09).
- Same-key admission dedupe is out of scope for the startup set; the recursion
  guard is keyed on `(root_operation_id, stage)` and assumes unique root ids.
- `admit` success path allocates (root-id `to_string`, permit record). 0-alloc
  would require a storage redesign (interning / `Arc<str>`) — a separate ADR.
