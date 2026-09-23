# Review prompt: SEP21 validator repair + SEP22-T01 fix + gate lint fixes

Review the implementation and tests below against their stated intent. Trace code
semantics, owners, and proof. Do not rubber-stamp: attempt to falsify each claim.

## Scope (exact diffs)

1. `docs/bugbash/sep-21/tickets/plan.json` — checklist_sha256 pin
   `80cc61ad…` → `9911323a…` (committed).
2. `docs/bugbash/sep-21/tickets/validate_plan.py` — `zip(strict=True)` →
   plain `zip` + comment (committed). Claim: the two slices always have equal
   length by construction, and the repo `.venv` is Python 3.9 where `strict=`
   does not exist.
3. `crates/taskmesh/tests/runtime_cancel_timeout.rs` (committed, ±28 lines):
   - `cancel_while_waiting_for_substrate_…` waits for `queued==1` instead of
     `inflight==2`, asserts `inflight==1` as the binding-limit proof, adds
     exact drain asserts (`inflight/queued/blocking.in_use == 0`).
   - Deleted now-unused `wait_until_inflight` helper.
   - Core mechanism claim: a waiter behind an occupied blocking capability
     stays governor-queued (capability-blocked) and never reaches `inflight`.
4. `crates/taskmesh/tests/deadline_cancel.rs` (uncommitted): bind the
   `MutexGuard` before `if let … take()` (clippy significant-Drop fix).
5. `crates/taskmesh/tests/e2e_scenarios.rs` (uncommitted): `Box::pin` one
   18KB `join!` future at the call site (clippy large-futures fix).

Out of scope (do not review as mine): peer TO-01…TO-08 implementations, the
dirty production wave (`runtime.rs`/`builder.rs`/`config.rs`/`metrics.rs`),
and all `docs/plans/sep-16*` edits.

## Falsification targets

- T01 mechanism: is `queued==1 ∧ inflight==1` a sound proof that the binding
  limit is the blocking capability, not the class? Construct a schedule where
  it passes vacuously, or confirm none exists. Check `acquire()` /
  `await_promotion()` / `admit_validated_requirements` ordering.
- Probe validity: author flipped `max_inflight(2)→1` in the zero-budget
  fixture and observed `SubstratePoolTimedOut` → `PermitAcquireTimedOut`.
  Is that flip a spec-conformant discrimination or an intake-precedence
  accident? What if precedence changes?
- The fixed cancel test asserts `CancelledBeforeSubmit` — the same verdict
  both contention paths return. Does the ticket's A01 (paired verdict oracle)
  actually rest on the two timeout tests instead, and is that consistent
  with the ticket text?
- Lint fixes: prove the guard-binding change is semantics-identical (lock
  hold duration, drop order) and that `Box::pin` at one call site cannot
  alter poll behavior of the non-`Send` local future.
- Flake risk: `wait_until_queue_depth` polls at 1ms under a 1s timeout;
  the cancel fires immediately after observing `queued==1`. Race window
  between observation and cancel? Between cancel and the 100ms join budget?
- Validator fix: any other 3.10+ runtime syntax in `validate_plan.py` that
  still breaks on 3.9? Is updating the checklist pin (rather than reverting
  the checklist) the correct authority decision?

## Proof to re-run (do not trust the handoff counts)

```sh
.venv/bin/python docs/bugbash/sep-21/tickets/validate_plan.py --structure-only
.venv/bin/python docs/bugbash/sep-22-test-optimization/tickets/validate_plan.py --structure-only
cargo test --locked -p taskmesh --test runtime_cancel_timeout
cargo test --locked -p taskmesh --test deadline_cancel --test e2e_scenarios
cargo test --locked -p taskmesh-rayon --test rayon_smoke
cargo test --locked -p taskmesh-bench --test hellgate
cargo fmt -p taskmesh -- --check
```

Handoff claims: 9/9 (5/5 repeat on the fixed test), 15/15, 9/9, 4/4, 6/6;
fmt clean; `-p taskmesh --tests` clippy with repo allows clean. Re-run and
confirm or refute. Semgrep and IAI rails are unavailable on a sandboxed
macOS host (semgrep binary CA crash, `mktemp` sandbox denial) — verify they
fail for those environmental reasons only, and do not accept that as code
proof.

## Output contract

- Verdict per item (1–5): ACCEPT / REJECT with file:line evidence.
- For every REJECT: the failing schedule, input, or command that reproduces it.
- Ticket acceptance mapping: which of SEP22-T01 A01–A04 is satisfied by what
  evidence, and what (if anything) is still open before W3 receipt.
