# Release Checklist

## Proof gate (must be green)

The gate set is `tools/gates/inventory.json`; the required subset is
`tools/gates/required.json`. Run them through `just` — the same recipes CI runs:

- [ ] `just gate` — fmt-check, strict clippy (3 passes), test, deny, semgrep
      (real integration tests enrolled), architecture checker, py-lint, py-test,
      pm-lint, allocation gate, gates-inventory parity
- [ ] `just test-rayon` / `just doctest` / `just rustdoc` / `just bench-smoke`
- [ ] `just mutants-critical` — every mutation in
      `tools/verification/mutations.json` KILLED (and the control CONTROL_GREEN)
- [ ] `just loom` / `just shuttle`
- [ ] `just tsan` — `status=CLEAN` (ThreadSanitizer over the production engine and
      host concurrency tests; needs nightly + rust-src, otherwise NOT_RUN which is
      not a pass)
- [ ] `just coverage-report` — `status=REPORTED`; read the uncovered production
      lines in `target/coverage/lcov.info` and account for each (unreachable by
      design, or a gap). The percentage is information for this review, never a
      threshold.
- [ ] `just consumer-msrv` — PASS on the declared `rust-version` (NOT_RUN is
      not a pass)
- [ ] `just bench-iai` on Linux — `status=QUALIFIED`. The first run for a given
      fingerprint (bench definition, deps, toolchain, valgrind, runner) records a
      baseline and reports `BASELINE_CREATED`; the *next* run with the same
      fingerprint qualifies. CI keys its baseline cache on
      `tools/bench-iai.sh fingerprint`, so a PR that does not change those inputs
      is compared against main's baseline.
- [ ] `uv run python tools/qualification/receipt.py collect --out <receipt>` on
      an **immutable checkout** (under `uv run`, so the pytest-runner mutations
      have their environment), and `validate` reports `QUALIFIED`. `validate`
      re-derives the source identity (digest, HEAD, dirtiness) and the receipt's
      internal consistency; it does not re-run gates. The receipt of record is
      the one `collect` produced on that checkout — never a file handed over
      for `validate` alone.
- [ ] hell-gate e2e: `crates/taskmesh/tests/e2e_proof.rs`, `e2e_scenarios.rs`

## Invariant proofs

- Resource accounting `Σpermit == held` (exact `u128`), per-class==global,
  phase gauges partition `inflight`, `admitted == inflight + terminated`, and
  capability occupancy == live permits per pool, all enforced by
  `GovernedState::assert_consistent` (debug) on every transition — and
  recomputed by an *independent* oracle over `Governor::permit_ledgers()` in
  `hardening_exact_accounting.rs` / `hardening_snapshot_projection.rs`.
- `prop_invariants.rs`: drains-to-zero + cap-respect (200 cases) and fairness
  determinism (120 cases).
- `hardening_fairness_reference.rs`: DRR order equals an independent
  visit-by-visit reference; WFQ order is scale-invariant.
- `loom_governance.rs` / `shuttle_governance.rs`: interleavings on the production
  `Governor` — exactly-once handoff, lost-wakeup freedom, claim-vs-reap, and the D08
  promotion-gap rule under concurrent admits (shuttle, 5,000 schedules each).
- `differential_model.rs`: the admission/promotion contract as a ~150-line
  executable specification, checked against the engine after every random op.
- Mutation gate: 62 entries — 61 single-edit reintroductions of fixed defects
  (54 cargo incl. 2 shuttle-model targets and 1 differential-model target, 7 pytest
  against the Python tooling), each killed by its named regression for its named
  reason found in that test's own output, plus one behaviour-preserving control
  that must stay green
  (`receipts/local-2026-09-18.mutations.json`).

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
13. bounded intake / no gate queue — `hardening_intake_bounds`
14. ticket lifecycle terminal outcomes — `hardening_lifecycle`,
    `runtime/claim_acquisition_tests`
15. deadline custody (blocking RunFor, inline executor, acquire budget, release
    fence, cleanup-before-response) — `hardening_deadline_custody`
16. stack request consumes `large_stack` — `hardening_dispatch_resolution`
17. worker setup / awkward class names — `hardening_executor_protocol`
18. topology validation — `contract/topology_validation`; registry validation —
    `hardening_policy_inventory`

## API & docs sync

- [ ] Public type names unchanged: `TaskSpec`, `TaskClass`, `TaskStage`,
      `SubstrateHint`, `AdmissionVerdict`, `GovernorError`, `RunError`,
      `Snapshot`, `SubstrateRecord`, `Runtime`.
- [ ] Zero placeholder public types.
- [ ] README / library-spec / external-interface describe the same contract.
- [ ] Every ```rust block in README / external-interface compiles against the
      facade: `cargo test -p taskmesh-doc-examples` (part of `just test`; the
      build script extracts the blocks verbatim — a new fragment that needs a
      new placeholder gets it in `tools/doc-examples/src/lib.rs::prelude`).
- [ ] `taskmesh-rayon` included; workspace green with and without the `rayon`
      feature.
- [ ] Crate versions in sync (workspace `version` in the root `Cargo.toml`,
      the `[workspace.dependencies]` path entries that pin it, the root
      `Cargo.lock`, and `tools/consumer-msrv/Cargo.lock`). Update the locks
      with `cargo update -p taskmesh -p taskmesh-engine -p taskmesh-contract
      -p taskmesh-rayon -p taskmesh-bench --offline` (and the same, minus
      `-p taskmesh-bench`, under `cargo +<msrv>` inside `tools/consumer-msrv`)
      — never a wholesale regenerate. `cargo metadata --locked` must succeed
      in both places on stable and on the declared MSRV toolchain.
- [ ] `CHANGELOG.md` has a section for the version being released, with
      `### Breaking changes and migration` carrying before/after code for every
      breaking item (signature, wire, *and* behavioural).

## Semver gate

- [ ] `cargo semver-checks check-release -p <crate> --baseline-rev <last release>`
      for each library crate (`taskmesh-contract`, `taskmesh-engine`, `taskmesh`,
      `taskmesh-rayon`), with `--default-features` (the consumer surface; the
      baseline may lack newer optional features). Pass
      `--release-type minor` while the major is `0` so every *major*-level
      finding is listed instead of being waived by the 0.x bump. Every finding
      must map to a CHANGELOG entry; the raw output is kept with the release
      notes.
- [ ] Reconcile the tool's blind spots by hand: `cargo-semver-checks` does not
      compare types, so a changed return type, a widened field (`u32 → u128`),
      a serde representation change, or a behavioural change (a verdict that
      now fires where none did, a deadline that is now refused) is invisible to
      it. Diff the `pub` items of `crates/*/src` against the baseline and read
      the contract tests (`crates/*/tests/hardening_*.rs`) for behaviour. The
      `taskmesh` facade is all re-exports, which the tool does not follow — a
      clean facade report says nothing about the surface consumers use.
- [ ] `just consumer-msrv` PASS after the fixture in `tools/consumer-msrv`
      has been extended to exercise every migrated shape named in the
      CHANGELOG (it *runs* and fails loudly on a wrong outcome).

## Semver scope

- Public surface = `taskmesh` re-exports (contract + selected engine items).
  Adding ports/verdict variants is additive; renames are breaking.
- `GovernorError`, `AdmissionVerdict`, `TopologyError`, `ExecutionPhase`,
  `TerminalReason`, `ResourceConversionError`, `ExecutorCapabilities`,
  `StageReleaseOutcome` and `ReconcileOutcome` are `#[non_exhaustive]`: adding
  a variant/field is additive for compilation, but a new variant that
  *replaces* where an existing arm used to fire (0.2.0: worker failures moved
  out of `PolicyViolation`) is a behavioural break and goes in the migration
  section. `ClaimOutcome`, `ReleaseOutcome` and `CapacityBlock` are exhaustive
  on purpose (a waiter loop must handle every outcome); adding a variant to
  them is a major change.

## Known residue (intentional, tracked)

- `BlockingPoolCpuExecutor` is the default CPU executor; Rayon is opt-in. The
  blocking-pool path is a *residual* default behind the port, not a final shape
  claim (ADR 9000 / T09).
- Same-key admission dedupe is out of scope for the startup set; the recursion
  guard is keyed on `(root_operation_id, stage)` and assumes unique root ids.
- `admit` success path allocates (root-id `to_string`, permit record): 3
  allocations per admit→release at the current baseline (`just bench-gate`).
  Capability names are interned so the unified authority added none. 0-alloc
  would require a storage redesign (root-id interning) — a separate ADR.
- `run_blocking` + `RunFor` bounds the caller's wait only; a started blocking
  job is not aborted (documented, tested).
- Declared nested wait cycles (parent awaiting a child on the capability it
  holds) are unsupported and not detected (ADR 0003, D12).
- No coverage gate exists. `just mutants-critical` covers the enrolled
  regressions only; property tests outside `tools/verification/mutations.json`
  are not mutation-checked.
