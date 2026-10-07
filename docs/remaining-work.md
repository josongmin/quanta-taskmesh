# Taskmesh remaining work

This list owns unfinished work. Completed implementation belongs to
[ADR 0008](adr/0008-audit-implementation-record.md) and
[0009](adr/0009-runtime-and-evidence-hardening.md). Current verification status is
read from exact-source artifacts; historical ticket states and old plan snapshots
cannot qualify the current HEAD. Archived details remain in
[document history](evidence/document-history.json).

## T1 — Final-source proof

**Owner: Taskmesh verification.** Obtain a complete clean-source CI16 receipt,
actual hosted merge/main CI, and complete Linux full23 (CI + nightly union)
evidence for the source being qualified. Preserve raw identity sets/outcomes,
producer envelopes, source tuple, failed attempts and IAI baseline continuity.
Finding/release inputs must bind the current ADR sections and ticket map.
Historical lifecycle and recorded `closure_status` are checkpoint metadata;
exact-source execution owns the current verdict.

Ordinary owner checks and `verify-macos-ci` are normal feedback. Mutation-bearing
full23/nightly requires explicit authorization in the current request under
AGENTS.md; this backlog is not authorization. [ADR 0006](adr/0006-source-bound-verification-authority.md)
owns proof boundaries and the inventory owns gate selection.

## T2 — Human compatibility and release decision

**Owner: Taskmesh release owner and human reviewer.** Run pinned
`cargo-semver-checks 0.50.0` for all four public crates against immutable 0.2.0
`39bee682d7daa1efaf1c10993ba6221fd0a90871`. Bind every raw locator/digest to
reviewer-owned Rust API, facade, wire and behavior adjudication. All 17 tracked
dispositions in `tools/release/adjudication.json` remain PENDING. Require human
identity, version/rollback approval and same-source full23 before a final release
receipt. Tool reports and AI-prepared inventories do not supply human approval.
[Release checklist](release-checklist.md) owns the procedure.

## T3 — Measured host performance

**Owner: B00/performance.** Register consumer SLO, completion floor, precision/MDE,
absolute rate grid and representative privacy-safe H7 trace. Acquire repeated
matched quiet-host baseline/candidate measurements with generator/observer
controls, complete attempt accounting, provenance and failed-run retention.
Equivalent peer comparison and independent rerun need separate evidence.
No qualifying B00 series is recorded. Linux instruction-count continuity and
longitudinal recovery claims require their own measurements.
[ADR 9000](adr/9000-benchmark-strategy.md) and
[host series](benchmarks/host-series.md) own protocol and executable acquisition.

## T4 — Semantica consumer acceptance

**Owner: Semantica consumer/deployment.** Freeze current consumer source, update
all Taskmesh manifest/lock pins together, and rerun paired SDK/runtime/parser
rails with actual QBC execution root, source digest and raw identities. Selected
paired checks do not prove whole SDK or product acceptance. Obtain actual hosted
source-bound acceptance and record D1–D9 applicability at ingress, planner, wire,
executor/capacity and deadline/custody boundaries. Keep synchronous `CompleteBy`
refusal and caller-response/worker-termination semantics explicit. Historical
consumer SHAs/jobs do not establish current external state. The consumer owner
approves acceptance and activation separately.

CI16, full23, human compatibility, performance, consumer acceptance and deployment
retain separate verdicts. This list approves no version/tag/release, activation,
mutation campaign or recurring schedule.
