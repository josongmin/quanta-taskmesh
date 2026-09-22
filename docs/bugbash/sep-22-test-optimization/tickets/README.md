# SEP-22 test optimization execution tickets

Baseline: `main@7023e945e1c9b3e7e7cca1b26f7af8467df6ab60`, tree
`e49ed06e109525a9da243c47b9207919a74ed15e`.

These tickets implement the eight open findings in [`../README.md`](../README.md). They optimize
test execution without weakening semantic or qualification authority.

## Ticket map

| Ticket | Finding | Priority | Owner lane | Depends on |
| --- | --- | --- | --- | --- |
| [SEP22-T01](SEP22-T01-capability-binding-oracle.md) | TO-01 | P0 | runtime-cancel | — |
| [SEP22-T02](SEP22-T02-cancellation-handshakes.md) | TO-02 | P1 | cancellation-sequencing | — |
| [SEP22-T03](SEP22-T03-backpressure-handshake.md) | TO-03 | P1 | e2e-scenarios | — |
| [SEP22-T04](SEP22-T04-drain-completion-boundary.md) | TO-04 | P2 | e2e-scenarios | T03 |
| [SEP22-T05](SEP22-T05-python-floor.md) | TO-05 | P0 | python-compat | — |
| [SEP22-T06](SEP22-T06-semgrep-scan-ssot.md) | TO-06 | P1 | semgrep-proof | — |
| [SEP22-T07](SEP22-T07-rayon-bounded-rendezvous.md) | TO-07 | P1 | rayon-adapter-test | — |
| [SEP22-T08](SEP22-T08-hellgate-structural-budget.md) | TO-08 | P1 | bench-validity | — |

## Execution waves

| Wave | Parallel work | Serial boundary |
| --- | --- | --- |
| W0 | T01, T02, T03, T05, T06, T07, T08 | Re-freeze HEAD and verify exclusive paths before editing. |
| W1 | T04 | Starts only after T03 settles `e2e_scenarios.rs`. |
| W2 | Integrator | Rebase/reconcile, run focused suites, then exact-source `just gate` and `just matrix`. |
| W3 | Qualification | Run `just verify-local`; closure requires its exact-source receipt, not W0/W1 passes. |

## Ownership and patch-on-patch prevention

- `runtime-cancel`: only `runtime_cancel_timeout.rs`.
- `cancellation-sequencing`: only `cancellation_policy.rs` and the named cancellation cases in
  `deadline_cancel.rs`.
- `e2e-scenarios`: T03 then T04 in one writer lane; T04 also owns the named settle delay in
  `e2e_chaos.rs`.
- `python-compat`: `pyproject.toml`, `uv.lock`, and the generated-mutation import boundary.
- `semgrep-proof`: `test_rules_fire.py`; no rule YAML changes.
- `rayon-adapter-test`: `rayon_smoke.rs`; no production executor changes unless the real adapter
  fails after the test harness is bounded.
- `bench-validity`: `hellgate.rs` and measurement evidence only; no production governor changes.
- Shared `Justfile`, gate inventory, workflows, qualification receipt, and production semantic files
  are not owned by these tickets. If a ticket proves they must change, stop and reopen the plan with
  a dedicated integrator owner.

## Common implementation rules

1. Replace scheduler guesses with observable state or one-shot handshakes. Do not replace one sleep
   with a shorter sleep, repeated `yield_now`, or an unbounded poll.
2. Every wait introduced by a test must have an attributable local timeout.
3. Preserve typed verdict, conservation, exact drain, and fail-closed assertions. Runtime reduction
   cannot come from sampling fewer semantic checkpoints unless the ticket explicitly owns a
   measurement experiment.
4. Test helpers stay local to the smallest owner module. Do not create a global compatibility or
   fixture layer solely to share a few lines.
5. No production change is allowed to make a test easier to schedule. If a deterministic test
   exposes a product defect, stop the test-optimization ticket and open the product owner.
6. Before and after timings use the same command, profile, host class, target directory policy, and
   test selection. Record median and worst case, not a single wall-clock sample.

## Per-ticket evidence handoff

Every ticket handoff must record:

- start and end HEAD/tree plus tracked/untracked dirty paths;
- files changed and acceptance IDs satisfied;
- exact commands, exit codes, selected/executed test counts;
- before/after timings where the ticket claims savings;
- any skipped or unavailable rail;
- source-before/source-after equality for measured runs;
- whether evidence is focused, gate-level, matrix-level, or receipt-qualified.

## Final verification

```sh
uv run python docs/bugbash/sep-22-test-optimization/tickets/validate_plan.py --structure-only
uv run pytest tools -q --durations=40
cargo test --locked -p taskmesh --test runtime_cancel_timeout --test cancellation_policy \
  --test deadline_cancel --test e2e_scenarios --test e2e_chaos
cargo test --locked -p taskmesh-rayon --test rayon_smoke
cargo test --locked -p taskmesh-bench --test hellgate
just gate
just matrix
just verify-local
```

Focused and gate passes are not qualification. `just verify-local` must emit a current-source receipt
for closure; missing/NOT_RUN required rails keep the campaign open.

## Plan validation

```sh
# Before implementation: validates frozen source plus structure.
uv run python docs/bugbash/sep-22-test-optimization/tickets/validate_plan.py

# After intentional source edits: validates mapping, citations, links, dependencies, and acceptance.
uv run python docs/bugbash/sep-22-test-optimization/tickets/validate_plan.py --structure-only
```
