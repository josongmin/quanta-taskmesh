# Taskmesh remaining work — source snapshot 2026-10-07

This is an operational snapshot for clean Taskmesh `main@e5c209ce853fa1f02028478100ce0a61368cc632` (tree `acee80d957f63f4fdf8934e477d085a13155ca87`, 537 paths). A later source needs its own CI, producer evidence and qualification. The [2026-10-03 audit](../audits/2026-10-03-current-source-final.md) and earlier receipts remain historical.

## Delivered source and current verdict

- PRs #12–26 are normally merged on `main`. PRs #19–26 added process/custody, generated discovery, cancellation, Linux thread, and composite/deadline test repairs. Current owner source is the authority for API behavior; historical plans do not reopen implemented code.
- Actual-main hosted CircleCI passed the required 16-gate `ci` profile; its Linux receipt was independently validated on exact `e5c209ce` (Rust 742/742, including bench 130; Python 1450/1450, excluded 0). CI16 is a merge check, not release qualification.
- Full attempt 16 on clean `e5c209ce` ended `NOT_QUALIFIED`: 22 PASS, `mutants-generated` FAIL. Fresh discovery selected 2,653 of 2,665 mutants (12 regex exclusions). Structured raw contains 359 Caught, 64 Unviable, one Missed, baseline Success; it stopped early after the miss. The miss is `crates/taskmesh-bench/src/composite_host.rs:229`, `||`→`&&` in `CompositeFailureRaw::validate_against`. The per-child **key or path** check has no independent invalid-key/path witness that catches this mutation. The 424 classified outcomes are a partial diagnostic, not a passing denominator.
- Full16 terminal archive reports `canonical receipt source identity differs from frozen source` because its helper compares the receipt's four-field `source_after` object with the full frozen `source` object. The actual receipt's four identity fields match clean `e5c209ce`; the canonical failure is the generated mutation miss. Repair the ignored archival helper/schema comparison separately, preserve full16 raw, and never turn this into a PASS claim.

## T1 — Close the reachable composite failure oracle, then qualify final source

**Owner: Taskmesh.** On an isolated branch, construct a valid retained `CompositeFailureRaw` with a deterministic scenario and three resolved child paths, then independently corrupt only one child key and only one child path. Each must fail validation for the intended reason while all other fields remain valid. A positive serialized round trip should pass. Prove the focused regression RED against the exact old validator/mutant and GREEN with the new oracle; keep the production guard `||` intact unless source evidence finds a production defect. Check adjacent identity/population/time guards only for reachable uncovered branches, without broad implementation-mirror tests.

After normal commit, current-source CI16, hosted PR CI, merge and actual-main refreeze, run one fresh canonical 23-gate Linux qualification (CI + explicit high-cost nightly union) on that exact clean main. Retain complete raw lists/outcomes, producer envelopes, source tuple, IAI baseline continuity and receipt. Every required gate must PASS; interrupted or partial mutation results cannot be composed with other attempts. The current request authorizes one-time qualification work and repair reruns, not a recurring schedule.

## T2 — Release evidence and human compatibility decision

**Owner: Taskmesh release and human reviewer.** On `e5c209ce`, pinned `cargo-semver-checks 0.50.0` against immutable 0.2.0 baseline `39bee682d7daa1efaf1c10993ba6221fd0a90871` produced four `REPORTED` crate outputs. Finding proof is 23/23 PASS. The source/raw-bound draft maps 25 locators, but all 17 compatibility dispositions are `PENDING`, all four crate-output reviews are false, reviewer is null and proposed 0.3 remains a human decision. These reports become historical after the next code merge; rerun and rebind on final main before release review. A tool report or AI-prepared index is not approval. Final release receipt additionally requires same-source full23 qualification.

## T3 — Measured host performance

**B00 performance owner:** Register consumer SLO, completion floor, precision/MDE, rate grid and representative trace before collecting repeated matched quiet-host baseline/candidate series. Retain failed runs, provenance, sampler distortion and headroom. No qualifying B00 series or verdict exists. [ADR 9000](../adr/9000-benchmark-strategy.md) owns the measurement protocol.

## T4 — High-cost proof operation

`nightly` is an explicit one-time profile. No recurring schedule was requested.
Runner admission, owned writer custody, unchanged gate budgets and exclusions,
raw retention and exact-source receipts remain required for each attempt.

## T5 — Semantica consumer integration

**Semantica consumer owner:** Draft PR #301 at `1e18857d63725cc6da58e03502ae3e1e445366d1` pins Taskmesh `e5c209ce`. An exact private snapshot passed selected SDK 1, runtime 5 and parser 1 checks; SDK and parser each filtered 451 tests. This is seven selected checks, not whole-SDK or product qualification. Hosted GitHub Actions 11/11 jobs did not execute because of billing admission; Circle reported a zero-start task-information failure with cause unknown. Do not label hosted code RED or PASS. After the final Taskmesh merge, update all consumer manifest/lockfile pins together
and rerun the selected paired rails on that exact dependency. Retain source digest,
actual QBC execution root and raw outcome identity. Keep synchronous `CompleteBy`
rejection and the acquisition-to-response deadline / started-worker lease distinction.
Resolve hosted admission and record D1–D9 applicability and evidence for actual
consumer boundaries. The consumer owner decides acceptance and activation separately.

## Release and activation boundary

Implementation, CI16, full23, compatibility review, B00 and consumer acceptance have separate verdicts. No version/tag/release, deployment or activation is approved by this snapshot. [Release checklist](../release-checklist.md) retains the source-bound final receipt and human decision requirements.
