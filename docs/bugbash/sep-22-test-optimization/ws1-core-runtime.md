# WS1 — core runtime: actionable findings

Baseline: `7023e945e1c9b3e7e7cca1b26f7af8467df6ab60`.

## TO-01 — substrate cancellation test does not prove its binding limit (P0)

Evidence:

- `runtime_cancel_timeout.rs:10-23` configures `blocking_threads(1)` and
  `max_inflight(1)` for the same class.
- `runtime_cancel_timeout.rs:132-145` starts a holder that owns both capacities.
- The second submission at `:147-159` encounters both limits simultaneously. The 20 ms sleep at
  `:161` does not prove the blocking capability, rather than the class limit, is the binding reason.
- The asserted `CancelledBeforeSubmit` result is also valid for cancellation while queued at the
  governor, so the test can pass without exercising the behavior named at `:126`.

Required fix:

- Use a separate fixture with `max_inflight >= 2`, `blocking_threads(1)`, and no other earlier
  binding limit.
- Start the holder and wait for its closure-start signal.
- Start the waiter and observe it queued while class inflight capacity remains available and the
  blocking capability is occupied. Its closure must not start.
- Cancel, require prompt `CancelledBeforeSubmit`, then release the holder and prove exact drain.

## TO-02 — mid-run cancellation state is inferred from sleeps (P1)

Affected tests:

- `cancellation_policy.rs:41-72`: sleeps 50 ms before cooperative cancel.
- `cancellation_policy.rs:75-98`: sleeps 20 ms before cancel while work sleeps 80 ms.
- `deadline_cancel.rs:563-588`: a timer sleeps 40 ms before cancelling local pending work.
- `deadline_cancel.rs:613-650`: sleeps 40 ms before cancelling a stalled CPU submission.

Failure mode:

- If the spawned task has not crossed admission, cancellation exercises pre-submit rejection rather
  than the intended mid-run policy.
- The expected verdict can then fail for scheduler load rather than a product regression.
- In the stalled-executor case, the test also assumes the executor has accepted and retained the
  closure before cancellation.

Required fix:

- For IO/local tests, signal from the first poll of the user future.
- For `PreSubmitOnly`, wait for first poll, cancel, then release the future explicitly and require
  the successful value.
- For the stalled CPU executor, expose a bounded `accepted` signal from `spawn` before cancelling.
- Preserve the existing terminal verdict and exact drain assertions.

## TO-03 — backpressure sequencing and failure tail are timer-owned (P1)

`e2e_scenarios.rs:260-312` says the holder runs for about 120 ms, but the holder actually waits on a
oneshot. The test sleeps 50 ms to assume the holder started, sleeps another 80 ms before releasing
it, and allows 1,000 retries with 5 ms sleeps.

Required fix:

- Add a holder-start signal from inside the blocking closure.
- Signal the first observed `CpuSaturated` result from the client.
- Release only after that witness.
- Bound the entire client with one timeout. The successful retry and `attempts > 0` oracle remain.

## TO-04 — post-completion settle sleeps (P2)

- `e2e_scenarios.rs:358-366`: all 64 handles are joined, then the test sleeps 60 ms before drain.
- `e2e_chaos.rs:356-366`: all 200 handles are joined, then the test sleeps 50 ms before drain.

The public `run_*` completion boundary should define whether capacity has been returned. A fixed
delay both costs wall time and can hide a late-accounting defect.

Required fix:

- First remove the sleeps and assert drain immediately.
- If the documented contract intentionally allows asynchronous retirement, use a bounded snapshot
  predicate with an attributable timeout; do not restore an unconditional delay.

## Explicit non-findings

- The cancelled-before-submit tests are not duplicates: blocking vs IO, cancellation-policy
  independence, and preflight-vs-contention precedence are separate surfaces.
- The poll/drain helpers are not one helper repeated seven times. They differ in sync/async context,
  state predicate, timeout, and owner crate. `claim_acquisition_tests.rs` cannot import an
  integration-test `common/mod.rs`.
- `governance_invariants_via_governor` includes a memory-overcommit oracle absent from the composite
  scenario. Removing its second half has negligible measured value and splits one proof narrative.
- `contains` checks over `GovernorError::PolicyViolation(Cow<'static, str>)` cannot be converted to
  typed equality without changing the public error model. That is an API decision, not test cleanup.
- The 30-second sleep in `hardening_degrade_policy` is inside a cancellable future and is not paid on
  the green path.
