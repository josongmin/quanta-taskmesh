# Audit remediation — 2026-10-04

Scope: the twelve actionable items from the four audits pasted by the user.
Baseline: `f2808d379b743fc5eaa7e295987c906dd7bc2957` on
`codex/taskmesh-current-source-remediation`. The audited owner paths match
`origin/main@7a5f5db0df686336d0781a92e623353acfb6713f`.

The existing changes in `docs/ssot/README.md`, `docs/taskmesh-library-spec.md`,
`docs/taskmesh-sota-audit-checklist.md`, and `docs/ssot/engine-status-v1.md`
belong to the starting checkout and must be preserved. Only a contradictory
checkpoint sentence in the library spec will be corrected independently.

## Requirements and acceptance

| ID | Owner | Structural repair | Required evidence | State |
|---|---|---|---|---|
| A01 | engine | Compensate undelivered admission/claim only with dispatch ownership proof; fence abandon against worker leases | Unused waker panic refunds capacity; reentrant leased-worker panic/abandon retains capacity until token release; follower cannot promote early | OWNER_UT_PASS |
| A02 | engine / bench | Index queued identities, capability predecessors and awaited children; centralize enqueue/removal | Independent prefix oracle and existing FIFO/identity/cycle regressions; separate 1k/10k fill/promotion measurements | OWNER_UT_PASS |
| A03 | contract | Checked snapshot conservation arithmetic and started/admitted ordering | Overflow and started>admitted negative cases; valid snapshots accepted | OWNER_UT_PASS |
| A04 | contract | State checkpoint metadata semantics accurately in public API and spec | Current implementation and API/spec agree that automatic hooks do not exist | SOURCE_CHECKED |
| A05 | host | Finalize caller response after custody release; recheck absolute deadline and cancellation | IO/local/CPU/shared and dedicated blocking/requested-stack late/on-time controls; RunFor worker budget and cancellation precedence | OWNER_UT_PASS |
| A06 | release tools | Supervise process groups, retain binary logs and reject incomplete execution for every leader code | Real timeout/child cleanup, capture limit and producer/receipt negatives; orphaned exit0/100 fails on single/batch paths | OWNER_UT_PASS |
| A07 | ingress | Preserve field presence and reject explicit `blocking_dispatch:null` | Null rejected; omission and valid blocking dispatch accepted | OWNER_UT_PASS |
| A08 | engine | Sticky accounting fault; terminalize and wake queued work without releasing live worker leases | New work refused; no-deadline host waiter receives AccountingFault; 4097 tickets retain typed reasons; repeated fault cannot add records; drain preserves custody | OWNER_UT_PASS |
| A09 | engine | Reject DRR quantum zero at construction | Checked constructor negative case and positive quantum controls | OWNER_UT_PASS |
| A10 | bench tools | Recheck source/toolchain identity after retained-validator execution | Each identity drift rejects; real retained validator writing to a temporary Git checkout rejects; unchanged/historical replay passes | OWNER_UT_PASS |
| A11 | engine tests | Execute permitted stage memory releases under concurrency with an observable positive oracle | Exact freed-unit assertions, nonzero releases and completion-only refusals; final accounting drains | OWNER_UT_PASS |
| A12 | bench tests | Generate valid per-tier fairness policies through checked Governor API | Original 3000 deterministic seeds retained; invalid mixed/scavenger policies rejected explicitly | OWNER_UT_PASS |

Root ID reuse while an earlier composite execution still owns descendants is
outside the declared root identity contract; it is not an additional runtime
repair requirement.

## Verification order and limits

1. Focused positive/negative owner UTs before broad builds.
2. Review the combined diff, index invariants, cancellation/custody precedence,
   and producer failure paths. Check newly discovered targets against inventory.
3. Run the normal development/CI checks for the final stable source. Record
   source identity and exact commands; distinguish dirty-source feedback from a
   clean-source CI receipt.
4. Measure queue operations separately from fixture construction and cleanup.
   This is local diagnostic cost evidence, not host latency qualification.
5. Preserve user changes and record failed/unrun checks. Mutation campaigns,
   final release qualification, external adoption, publication and deployment
   are separate scopes and are not authorized by this implementation request.

## Verification record

Owner feedback ran on the shared dirty checkout based on `f2808d3`:

- `cargo test -p taskmesh-engine`: 304 default-feature cases passed; doc tests
  selected zero cases.
  `cargo clippy -p taskmesh-engine --all-targets -- -D warnings` passed.
- `cargo test -p taskmesh-contract --test snapshot_oracle`: 11 passed.
  `cargo test -p taskmesh --test strict_ingress --no-default-features`: 15 passed.
  `cargo test -p taskmesh-bench --test fairness_property`: 8 passed, including
  the 3000-seed property denominator.
- Host custody/response/fault tests: 19 + 5 + 1 passed; the added TicketGuard
  lease-retention unit test also passed.
- `uv run pytest tools/release/tests tools/bench/tests/test_host_special.py
  tools/gates/tests/test_batch_supervisor.py
  tools/verification/tests/test_run_mutations.py -q`: 201 passed. This executes
  supervisor/inventory UTs, not a mutation campaign. Five existing source
  anchors were refreshed after the queue/abandon refactor; no campaign result
  is claimed.
- Gate inventory, architecture boundary, formatting and diff hygiene passed.
- `cargo bench --locked -p taskmesh-bench --bench queue_scaling -- --test`:
  10 smoke IDs passed. Raw measurements and source/toolchain tuples:
  `target/a02-queue-cost-20261004/{baseline,candidate}.tsv`, `metadata.txt`.

Local queue diagnostic medians (baseline -> final candidate):

| Completed operation | 1k backlog | 10k backlog |
|---|---|---|
| N unique queued admissions | 1.389 -> 0.722 ms | 110.687 -> 7.666 ms |
| One promotion, three built-in pools | 0.144 -> 0.033 ms | 1.307 -> 0.407 ms |
| One promotion, synthetic N-pool inventory | 5.099 -> 0.561 ms | 506.566 -> 8.104 ms |

Five repetitions per arm, except synthetic 10k used three. Fixture creation
and cleanup are untimed. Host load/frequency are uncontrolled; these numbers
are diagnostic, not a performance qualification. Synthetic pools exercise a
direct-engine worst case and do not prescribe a production host topology.

The initial remediation's normal CI authority is the generated clean-source receipt from the isolated
verification snapshot at
`/Users/songmin/.codex/worktrees/audit-structural-verification/quanta-taskmesh`:
`target/verification/macos-gates.json`, produced by the declared bounded CI
toolchain and profile:

```sh
RUSTUP_TOOLCHAIN=1.99.0 CARGO_BUILD_JOBS=4 uv run python tools/gates/run.py \
  --profile ci --require-clean-source --allow-platform-skips --keep-going \
  --receipt target/verification/macos-gates.json
```

This runs the same sixteen gates as `just verify-macos-ci`; keep-going retains
all gate results if a gate fails.
That artifact owns the snapshot SHA and gate verdicts. Executed case counts
are in `target/verification/rust-test-execution.json` and
`target/verification/pytest-execution.json`. Toolchain selection and observed
versions are recorded in `target/audit-remediation/closure.json`; the gate
receipt does not bind the compiler binary's digest. This plan is not a
verification receipt.

## Session hardening

The later `$session-hardening` review found two introduced regressions:

- Same-domain queue indexes used IDs allocated before the state lock. A later
  allocator can enqueue first; domain FIFO now follows lock-committed queue
  arrival independently of the permit/ticket identity. The regression oracle
  deliberately enqueues IDs out of order and compares physical deque order.
- Supervised semver/finding commands consumed SIGINT/SIGTERM during child
  cleanup, then their producer loops launched subsequent commands. The
  producers now retain the interrupted row, stop launching, and report an
  incomplete manifest. Actual signal tests cover both producers and both
  signals; uninterrupted controls still execute the complete inventory.

The new clean-source CI exposed a pre-existing acquisition cleanup defect:
macOS `psutil.Process.environ()` raised `SystemError`, aborting registry and
descendant cleanup. A transient environment failure is now retried once;
persistent observation failure records incomplete custody and cleanup still
continues. Registered-process observation errors also fail closed. Tests use
real child groups to prove continued termination and an incomplete result.

The Criterion fill-group comment now identifies its sample as an N-admission
batch. The earlier queue timing tuple remains diagnostic for its original
source. The initial CI receipt is historical after these edits; later proof
is retained separately under `target/session-hardening/`.

The first snapshot (`c3a0127`) stopped at Semgrep on twelve new-test quality
findings. Exact panic/timeout/decoded-value checks and checked fixture lookups
were added; the failure receipt/log are retained as `ci-attempt-1.*` beside the
final CI log. A separate dirty-source Clippy attempt used default Rust 1.95
and rejected the configured Rust 1.99 lint; final CI selects installed 1.99
explicitly. Queue timing tuples precede these test assertion repairs and
remain diagnostic evidence for their recorded source.

The second snapshot (`11e13f0`) passed 727 Rust cases and reached the full
Python gate (1359 passed, one failed). The failing environment UT inherited
the CI toolchain selector while testing campaign isolation. Its parent input
is now a controlled fixture, with an explicit RUSTUP_TOOLCHAIN rejection case;
all six focused environment cases passed under Rust 1.99 selection. Campaign
admission policy was not relaxed. The second receipt/log are retained as
`ci-attempt-2.*`; final CI authority remains the generated receipt above.

## Operational and release implications

- Queue indexes add O(N*k) bounded state, k <= 32. Candidate selection remains
  linear in queued work; awaited-child queues still need cycle assessment.
- AccountingFault is fail-stop. Existing workers must settle, then the host
  replaces the Governor. Faulted ticket records have a fixed queue-depth bound
  and remain until claim/abandon or Governor destruction; they are not counted
  in the ordinary terminal retention ring.
- `AbandonOutcome::HeldByLease` changes downstream exhaustive matches. Release
  requires explicit API/semver adjudication; no version bump or compatibility
  campaign is implied by these UTs.
- Existing user-owned docs were preserved. The shared checkout remains dirty;
  a clean-source CI receipt belongs to the isolated snapshot, not the shared
  HEAD. Mutation campaigns, final release qualification, external consumers
  and deployment/activation remain outside this request.
