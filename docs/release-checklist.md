# Release Checklist

## Proof gate (must be green)

The ordinary gate set is `tools/gates/inventory.json`; the ordinary required subset is
`tools/gates/required.json`. Release-only requirements are independently pinned in
`tools/release/release-required.json`. An ordinary `QUALIFIED` receipt is a necessary
input, not a release verdict. Run the registered recipes through `just`:

- [ ] `just gate` — fmt-check, strict clippy (3 passes), test, deny, semgrep
      (real integration tests enrolled), architecture checker, py-lint, py-test,
      pm-lint, allocation gate, gates-inventory parity
- [ ] `just test-rayon` / `just doctest` / `just rustdoc` / `just bench-smoke`
- [ ] `just mutants-critical` — curated single-edit inventory의 non-control 104개가
      KILLED이고 control 1개가 CONTROL_GREEN인지 확인한다. 이것은 cargo-mutants
      전체 생성 sweep의 score가 아니다.
- [ ] `just mutants-generated` — full current-source cargo-mutants workspace sweep.
      planned/executed/categorized IDs가 완전히 일치하고 baseline이 green이어야 한다.
      전체 denominator에는 `unviable` ID도 남긴다. 다만 cargo-mutants가 컴파일하지 못한
      `unviable`은 명시적 비채점 한계이며 quality denominator에서 제외한다. 최소 한 개의
      caught가 있고 missed/timeout/equivalent가 모두 0이어야 PASS다. `REPORTED`는 denominator
      disclosure일 뿐 quality PASS가 아니다.
- [ ] `just loom` / `just shuttle`
- [ ] `just modelcheck` — bounded Loom/Shuttle run identity와 독립 clean-process replay.
- [ ] `just tsan` — `status=CLEAN` (ThreadSanitizer over the production engine and
      host concurrency tests; needs nightly + rust-src, otherwise NOT_RUN which is
      not a pass)
- [ ] `just coverage-report` — `status=REPORTED`; lines/regions/functions/
      instantiations를 확인하고 branch·MCDC가 `NOT_COLLECTED`면 미수집 proof gap으로
      명시한다. `target/coverage/lcov.info`의 uncovered production line도 분류한다.
      어떤 percentage도 threshold가 아니다.
- [ ] `just consumer-msrv` — PASS on the declared `rust-version` (NOT_RUN is
      not a pass)
- [ ] `just bench-iai` on Linux — `status=QUALIFIED`. The first run for a given
      fingerprint (bench definition, deps, toolchain, valgrind, runner) records a
      baseline and reports `BASELINE_CREATED`; the *next* run with the same
      fingerprint qualifies. CI keys its baseline cache on
      `tools/bench-iai.sh fingerprint`, so a PR that does not change those inputs
      is compared against main's baseline.
- [ ] `uv run python tools/qualification/receipt.py collect --hosted-ci --out <receipt>`
      in the isolated GitHub Actions qualification job (under `uv run`, so the
      pytest-runner mutations have their environment), and `validate` reports
      `QUALIFIED`. Local collection is exact-source evidence but cannot emit a
      hosted qualification. `validate`
      re-derives the source identity (digest, HEAD, dirtiness) and the receipt's
      internal consistency; it does not re-run gates. The receipt of record is
      the one `collect` produced on that checkout — never a file handed over
      for `validate` alone. `mutants-critical` and `mutants-generated` are separate
      required denominators. A curated PASS never implies a generated score.
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
- Fuzzing: `just fuzz` — three libFuzzer targets (`fuzz/`): every public
  `Governor` transition in random order with the published invariants checked
  after each step and quiescence at the end; the policy / topology / builder
  front doors; the JSON wire formats. `just fuzz-check` keeps the targets
  compiling on stable. NOT_RUN without nightly + cargo-fuzz; only
  `taskmesh-fuzz status=PASS` with all target-specific semantic witnesses is PASS.
- Mutation gate: 105 curated entries — 104 single-edit fault probes
  (90 cargo incl. 3 shuttle-model targets and 1 differential-model target, 15 pytest
  against the Python tooling and the bench workflow scripts), each killed by its named regression for its named
  reason found in that test's own output, plus one behaviour-preserving control
  that must stay green. The historical schema-v1 receipt contains the first 100;
  five receipt/coverage/runner integrity probes were added with schema v2.

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

- [ ] Public type names and signatures diffed/adjudicated against the immutable baseline:
      `TaskSpec`, `TaskClass`, `TaskStage`,
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

- [ ] Version decision: `tools/release/release-policy.json` names immutable
      `39bee682d7daa1efaf1c10993ba6221fd0a90871` (the 0.2.0 release commit;
      there is no tag) as baseline but leaves `candidate_version` and reviewer
      `PENDING`. Do not infer a version from a green test or silently reuse 0.2.0.
- [ ] `just semver-release` runs pinned `cargo-semver-checks 0.50.0` separately for
      `taskmesh-contract`, `taskmesh-engine`, `taskmesh`, and `taskmesh-rayon`,
      `--default-features --release-type minor --baseline-rev <full SHA>`. It
      requires a clean candidate and records exact command/exit/tool/source and
      crate-specific stdout/stderr digests in `target/release/semver/`.
      Exit `0` means CLEAN, `100` means deny-level findings requiring human
      mapping, and `101`/timeout/tool errors fail. Completed four-crate audits
      are REPORTED, not automatically release-compatible.
- [ ] Reconcile the tool's blind spots by hand: `cargo-semver-checks` does not
      compare types, so a changed return type, a widened field (`u32 → u128`),
      a serde representation change, or a behavioural change (a verdict that
      now fires where none did, a deadline that is now refused) is invisible to
      it. Diff the `pub` items of `crates/*/src` against the baseline and read
      the contract tests (`crates/*/tests/hardening_*.rs`) for behaviour. The
      `taskmesh` facade is all re-exports, which the tool does not follow — a
      clean facade report says nothing about the surface consumers use.
      Use tracked `tools/release/adjudication.json` only as a non-approving
      template. Submit a reviewer-owned copy as the manual release workflow's
      `adjudication_json` input; the workflow stores it under ignored
      `target/release/input/adjudication.json`. Bind every item to the final
      candidate SHA,
      with explicit decision, reviewer and CHANGELOG anchor for every accepted break.
      The reviewer must bind each crate's stdout/stderr digest, attest all tool
      findings were reviewed, and map each exit-100 finding via an exact raw
      locator to an approved break item. Omitted findings are a human review
      failure; this raw-text tool has no complete machine finding list.
- [ ] Download the hosted `qualification-receipt` artifact from an exact-source CI
      run and execute `python3 tools/release/receipt.py collect`. The independent
      release receipt revalidates the ordinary 26-gate receipt, four-crate semver
      raw outputs, all 23 finding/ticket links, coverage and Linux IAI raw
      digests. Missing/NOT_RUN/SKIPPED/TIMEOUT, dirty or stale source, unapproved
      version or behavior break yield `NOT_QUALIFIED`. The `release.yml` workflow
      is manual and main-only; no PR check auto-approves a release.
- [ ] The manual release job also runs `tools/release/finding_proof.py` from the
      clean final source. Its tracked 23-finding spec names an actual negative
      or regression test for each finding; each selected test must execute
      exactly once and pass. The receipt checks complete stdout/stderr digests,
      test-source bytes, ordinary required gate linkage and before/after source
      identity. This is machine execution evidence, not a human approval or a
      substitute for the ordinary hosted receipt. Missing final-source output
      keeps A06 `NOT_QUALIFIED`.
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
- `admit` success path: the SEP-21 integrated governor measures 8 allocations
  per admit→release (`just bench-gate`, 200,000 completed operations with the
  counter self-check; three current-source probes were 8.000/8.000/8.000).
  The previous committed threshold was 3, so this explicitly accepts a +5
  allocation regression for the added governance state, not a performance
  improvement. Borrowed validation and the exact-operation permit index reduced
  an intermediate 16 to 8 without changing the measurement definition. The
  no-regression gate now uses exactly 8 with no cushion; Linux IAI comparison
  remains separate release evidence. A 0-allocation path needs a storage-model
  redesign (root-id interning), not a threshold adjustment.
- `run_blocking` + `RunFor` bounds the caller's wait only; a started blocking
  job is not aborted (documented, tested).
- Declared nested wait cycles (parent awaiting a child on capacity held only by
  that root) are rejected before queueing. Undeclared and cross-root cycles are
  still outside the detection contract (ADR 0003, D12).
- No coverage threshold gate exists. `just mutants-critical` covers the curated
  enrolled faults only; property tests outside `tools/verification/mutations.json`
  are not mutation-checked, and generated cargo-mutants breadth is a separate
  campaign.

## Rollout / rollback (H16-022 A06–A07)

This repository ships a library; nothing here deploys it. The steps below are
what a *consumer* owner runs, and they are gated on that owner's explicit
approval — a green proof surface in this repository is not activation.

Staged rollout (per consumer, in this order):

- [ ] Pre-flight the configuration the consumer will run, fail-closed, before
      any traffic: `TopologyConfig::validate()`, then `Builder::build()` — a
      policy the engine rejects (`GovernorError::PolicyViolation`,
      `InvalidTopology`, `ExecutorDeclaresFewerWorkers`) must be fixed here.
- [ ] Inventory diff, old vs new, from `Runtime::snapshot()` at idle:
      `schema_version` (1 → 2), the `substrates` set, and `capabilities`
      (every registered pool with its `limit`; `0` means ungated). A pool the
      old deployment did not gate and the new one does is a behaviour change
      for that consumer — record it.
- [ ] Canary with the consumer's real classes and watch, per class:
      `queued` and `inflight` (with the `dispatch_reserved` / `accepted` /
      `running` / `cleanup_pending` split), the rejection verdicts the caller
      sees (`QueueFull`, `CpuSaturated`, `MemorySaturated`,
      `SubstrateSaturated` vs `SubstratePoolTimedOut`), `cpu_units_held`, and
      `conservation_violation()` (must stay `None`). A rise in
      `SubstrateSaturated` for a class that used to wait is the D01 change
      (no unbounded waiting space) showing up: give the class a queue
      (`OverflowPolicy::QueueWithinDepth`) or accept the shed.
- [ ] Shadow, if used, is metadata-only: replay `TaskSpec`s through a second
      governor for its verdicts; never re-execute the consumer's jobs.
- [ ] Widen only after the canary window with `conservation_violation() ==
      None` throughout and no `WorkerPanicked` / `JobAbandoned` attributable
      to the runtime.

Rollback:

- [ ] Compatibility first: the previous artifact, its config, and its
      inventory must still build against the consumer (the 0.1.0 → 0.2.0
      breaks in CHANGELOG.md are the list of what the old code cannot see).
- [ ] The wire `Snapshot` is schema 2 (`u128` decimal strings, phase gauges,
      totals, capabilities); a 0.1.0 reader does not parse it. A rollback of a
      *reader* (dashboard, collector) therefore needs either drain-and-restart
      of the producer on the old version or a forward fix of the reader — a
      mixed-version window is not supported.
- [ ] Runtime state is process-local (no persisted governor state), so a
      rollback of the runtime itself is a restart: `TokioRuntime::drain(timeout)`
      (D17 — closes admission in the engine, then waits for `queued == 0` and
      `inflight == 0` in every class; `Err(NotDrained)` lists what is still
      charged, per class, and the runtime stays draining), then drop the handle
      (the only teardown), then start the previous version.
