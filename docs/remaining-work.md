# Taskmesh remaining work

Cleanup started on 2026-10-07 at clean `main@2123a462764a3d6445fd536a1e5a5f99d019425e`.
PR #28 merged concurrently; reconciled owner source is now
`main@e503b6c74c39209bbbafb31f8619e85122a6e714` plus this documentation/tooling overlay.
This cleanup changes documentation and release input paths; it has no current
clean-source qualification receipt. [ADR 0008](adr/0008-audit-implementation-record.md)
and [0009](adr/0009-runtime-and-evidence-hardening.md) own completed implementation.

## T1 — Final-source proof

**Owner: Taskmesh verification.** PR #27 implements independent composite child
key/path and adjacent raw/time/warmup oracles. Remove that implementation task
from the queue. PR #28 also implements fail-closed ownership classification for
disappearing procfs entries; unrelated or confirmed vanished tasks do not falsely
invalidate the held-group observation. PR #28 candidate `18a6a447` clean macOS
CI16 and synthetic merge `7b1560ea` hosted CI16 passed, with independently
validated raw receipts. Actual `e503b6c` main hosted CI16 also passed; its
receipt was replay-validated in an isolated native checkout on 2026-10-07.
This documentation/release-authority overlay needs its own final clean-source
CI16, hosted merge/main CI and one complete Linux full23 (CI + nightly union) receipt. Preserve raw
identity lists/outcomes, source tuple, producer envelopes, IAI baseline continuity
and every failed attempt. The historical V01/V02 ticket states remain unqualified.

The `e5c209ce` campaign stopped early with a composite key/path miss (359 Caught,
64 Unviable, one Missed; 424 completed of 2,653 selected, from 2,665 unfiltered
with 12 excluded). It is diagnostic and cannot be composed with later attempts.
The full16 archival helper compared a four-field `source_after` with a full source
object; that helper was repaired and checked with 29 malformed/source controls.
Its preserved historical terminal error is separate from the canonical gate verdict.

Full17 on actual `2123a462` stopped at py-test: eight gates passed, one failed,
and fourteen did not run; Python reported 1,449 passed and one failed. The custody
diagnostic was `ESRCH` during procfs enumeration. PR #28 classifies a disappeared
nonleader by kernel group membership while retaining fail-closed behavior for
owned members, the held leader and other read errors. Twelve deterministic
controls, corrected predecessor RED (8 failed/4 passed), and macOS/Linux owner
suites (98 passed each) support the repair; the historical syscall was not retained.
Full17 has no generated campaign discovery or mutation denominator.

Run costly mutation/full23 work only with explicit authorization in the current
request under AGENTS.md. This document cleanup provides no campaign authorization.
Moving the Sep-21 input to schema-2 `docs/evidence/sep21/ticket-map.json` invalidates
old finding/release input identities. Regenerate them on final clean source.

## T2 — Human compatibility and release decision

**Owner: Taskmesh release owner and human reviewer.** Rerun pinned
`cargo-semver-checks 0.50.0` across all four public crates against immutable 0.2.0
`39bee682d7daa1efaf1c10993ba6221fd0a90871`. Bind every raw locator/digest to a
reviewer-owned adjudication, including Rust API, facade, wire and behavior.
All 17 tracked compatibility dispositions remain PENDING. Four-crate review,
reviewer identity, version/rollback approval and same-source full23 evidence are
required before the final release receipt.

The latest preserved `2123a462` 23/23 finding proof, four REPORTED semver outputs
and draft 25-locator mapping are historical after PR #28 and this input migration. A tool
report or AI-prepared inventory is not human approval. [ADR 0009](adr/0009-runtime-and-evidence-hardening.md)
retains the technical surfaces; [release checklist](release-checklist.md) owns procedure.

## T3 — Measured host performance

**Owner: B00/performance.** Register consumer SLO, completion floor, precision/MDE,
rate grid and representative/privacy-safe H7 trace before repeated matched
quiet-host baseline/candidate acquisition. Retain failed attempts, provenance,
sampler distortion and headroom. Equivalent peer comparison and independent
rerun require their own evidence. No qualifying B00 series/verdict is recorded.
[ADR 9000](adr/9000-benchmark-strategy.md) owns protocol. Linux instruction-count
control/injected-regression and longitudinal recovery measurements cannot be
replaced with macOS or short structural diagnostics.

## T4 — Semantica consumer acceptance

**Owner: Semantica consumer/deployment.** Draft PR #301 was normally pushed at
`f193c55799fc32a20b373a9221e0bb28a8313a6b`, pinning Taskmesh `2123a462` in all
13 manifest/lockfile entries. Its private, clean source passed paired SDK (one),
runtime (five) and parser (one) selected checks; SDK/parser each filtered 451 cases.
Actual QBC source digest was
`b0e5feac60d9fd1d6cdaaa726b48cb4e7590bd9b74166cb837a9bcffa0ee3155`.
This proves those seven checks for that source pair, not whole SDK or product.
After final Taskmesh full23 PASS, update every pin together and rerun exact paired
rails with actual QBC root, source digest and preserved raw identities.

Fresh `f193c557` Circle status was PENDING; its admission cause was not established.
Historical eleven GitHub jobs on predecessor `1e18857d` never started due to
billing; that cause must not be attributed to the current head. Record current
hosted acceptance from actual receipts. Record D1–D9 applicability and evidence
at strict ingress, planner, wire, executor/capacity and deadline/custody boundaries.
Keep synchronous `CompleteBy` refusal and caller-response/worker-termination
semantics distinct. Consumer owner decides acceptance and activation separately.

## Completion rules

CI16, full23, human compatibility, B00, consumer acceptance and deployment have
separate verdicts. No version/tag/release or activation is approved here.
`nightly` is an explicit high-cost profile; no recurring schedule is configured
or authorized by this list. Runner admission, writer custody and unchanged budgets
apply to each proof attempt, rather than forming a duplicate implementation ticket.
