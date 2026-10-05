# Taskmesh remaining work after the 2026-10-03 source audit

Evidence snapshot on 2026-10-06 before PR #19's final publication. Later
verdicts belong to receipts for their exact source. [The 2026-10-03 audit](../audits/2026-10-03-current-source-final.md)
and its `aadfa07`/PR #9 receipts are historical, source-bound evidence.
Implementation, CI, release qualification, performance, consumer acceptance and
activation have separate owners and verdicts.

## Source and evidence

- Taskmesh `main@27609dd0210d06c7b2b72682ad5d4b4bab119271` includes PRs
  #12–18. Its actual-main CircleCI `ci` profile passed 16/16 gates. Each PR
  and main receipt applies only to the source identity it records.
- A successor custody repair is on PR #19. Dirty patch `f118b7ab` over
  `80ee68c` passed 209 focused cases on each of macOS and Linux. Committed
  candidate `24509a9f7f865a24158f5169833ef8d2cc75391c` then passed its
  clean macOS `ci` profile 16/16, including
  Python 1418/1418, and the exact-head receipt validated. Hosted CI, PR merge
  and actual-main proof are pending. This candidate receipt cannot qualify a
  later main SHA or this document's eventual commit.
- The last full 23-gate run on actual `main@27609dd` was interrupted after
  22 PASS; generated mutation was incomplete (330/2653). Verdict:
  `NOT_QUALIFIED`. Run the full profile afresh on one clean, final source.
  Historical gates and producer outputs cannot be combined across SHAs.
- Taskmesh is a library. Consumer qualification, deployment and activation
  need separate downstream evidence and owner decisions.

## T1 — Final-source CI and full qualification

**Owner:** Taskmesh. Finish PR #19's hosted CI, then freeze and verify
the actual merged-main SHA, tree and clean checkout. Run the required 23-gate
CI plus nightly union on that exact Linux source, including curated/generated
mutation, modelcheck, TSan, fuzz, coverage and Linux IAI. Retain complete raw outputs and
the canonical qualification receipt. A bounded 16-gate CI pass is a merge check,
not full qualification; a partial mutation run is diagnostic only. Human release
adjudication and the final release receipt remain separate in T2.

**Stop:** dirty or changed source, required FAIL/NOT_RUN/SKIPPED, incomplete
generated mutations, or a receipt bound to another SHA.

## T2 — Proposed 0.3 compatibility and finding adjudication, human decision pending

**Owner:** Taskmesh release. Version 0.3 remains a technical proposal pending
human approval. The unreleased API includes typed Rayon
`try_new/try_from_topology` in place of the removed panic constructors;
the current public/wire/behavior surface also needs final-source review.

1. Rerun pinned `cargo-semver-checks 0.50.0` for all four public crates against
   immutable 0.2.0 baseline `39bee682d7daa1efaf1c10993ba6221fd0a90871`.
   Run the 23 finding-proof witnesses on the same final source.
2. The actual `main@27609dd` semver output is `REPORTED`, with 25 raw
   locators across four crates; finding proof is 23/23 PASS. These are
   historical for any successor. The proposal has 17 pending compatibility
   decisions, four unreviewed crate outputs and no human approval. Rebind
   locators and raw hashes to the final source without carrying over an
   approval.
3. Have the release reviewer decide Rust API, facade, wire and behavioral
   breaks against the raw output and CHANGELOG anchors. The release receipt
   requires that adjudication and the same-source 23-gate proof.

**Stop:** missing raw output, unmapped finding, changed source, or pending
reviewer decision. A semver tool result does not approve compatibility.

## T3 — Measured host performance

**Owner:** Taskmesh benchmark and consumer owners. The source includes a
hash-bound producer-lag validator; its focused tests do not establish host
performance. [ADR 9000](../adr/9000-benchmark-strategy.md) owns the protocol.

1. Register B00 consumer SLOs, completion floor, precision/MDE, rate grid,
   representative trace and per-rate control budgets before measurement.
2. On a quiet host, collect repeated matched baseline/candidate attempts,
   failed attempts, raw provenance, sampler distortion and resource headroom.
3. Admit only the complete sealed series. Report SLO-goodput, completion,
   latency populations and run-level uncertainty. B00 and a qualifying series
   are still pending; no performance verdict is claimed.

## T4 — High-cost proof operation

**Owner:** Taskmesh operations. `nightly` names an explicit high-cost profile,
not a cron schedule. This request authorized one-time final qualification work
including mutation testing and repair reruns. No recurring schedule was requested.
A future schedule, if requested, needs its own runner budget, concurrency, retention,
alerts and receipt authority; a CI-only run cannot satisfy full release proof.

## T5 — Semantica consumer integration

**Owner:** Semantica deployment owner, with Taskmesh paired proof. Taskmesh
provides `run_blocking_response_by`/`run_cpu_response_by` for an absolute
acquisition-to-caller-response boundary. Synchronous `CompleteBy` rejects,
and a started worker keeps its lease until termination.

The observed Semantica branch `codex/taskmesh-main276-consumer-cfg-boundary`
at `423882f6e31817013b6ae9cacc25954ecd2d1cc1` (base
`4324e2a6143c9c301ec4d1b12c0683424faf013b`) is pushed as draft PR #301.
It pins Taskmesh `27609dd`.
On exact source snapshot `0063c7439ab2ff8e275cd92fec3c89ec3516b2669dd4216d3971d3ea5029f251`,
focused SDK 1, runtime 5 and parser 1 checks passed. SDK and parser each
filtered 451 tests; runtime ran its whole five-test target. These results do
not establish whole-SDK or final-source paired qualification. The original shared source was untouched.
Its 11 GitHub Actions jobs did not start because of a billing/spending-limit
block; the CircleCI state has not supplied a passing result.

After the Taskmesh main SHA is frozen, update the consumer manifest and
lockfile together, retain source/reason and root identity, and test the selected
blocking/stack path, expiry, queue/grant/start races, held worker, caller drop,
panic, drain and successor capacity. Keep a negative synchronous
`CompleteBy` test. Record D1–D9 applicability and proof for actual ingress,
planner/wire, handle, custody, Tokio/executor and serialization boundaries.
The consumer owner decides acceptance and activation separately. Billing
admission must be resolved before hosted checks can establish executed proof.

## Historical scope

- The `aadfa07` local/hosted 16-gate results and PR #9 links in the
  [2026-10-03 audit](../audits/2026-10-03-current-source-final.md) remain
  historical evidence. They do not qualify current or future source.
- M1 validator, S1 facade errors, S3 Rayon constructor repair, response
  boundary and subsequent queue/custody repairs are implementation work.
  Source presence and focused checks do not close T1–T5.
- Sep-21 plan, tickets and packets remain at their source-bound paths because
  finding proof and release receipts consume their exact bytes. The relevant
  decisions remain in [ADR 0003–0007](../adr/README.md) and
  [ADR 9000](../adr/9000-benchmark-strategy.md).
