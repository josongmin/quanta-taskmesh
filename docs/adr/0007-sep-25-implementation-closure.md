# 0007. Sep-25 implementation closure and surviving proof authorities

- Status: Accepted as the repository implementation record; qualification remains open
- Date: 2026-09-27
- Supersedes: BG25-001–012 implementation narratives archived at
  `docs/archive/2026-09-25/bugbash-sep-25-general/tickets/` in Git commit
  `cbf9764f08eb9f307f3ee5a51bad5e5dabeaf4a6`
- Decisions: [0004](0004-sep-25-ingress-plan-identity-and-wire.md),
  [0005](0005-sep-25-execution-response-and-custody.md),
  [0006](0006-source-bound-verification-authority.md), and
  [9000](9000-benchmark-strategy.md)

## Context

The twelve BG25 tickets reached `IMPLEMENTED` in the repository. Their long
implementation, audit, and handoff narratives duplicated the accepted decisions.
The implementation is complete in repository scope. Its
[ticket map](../evidence/sep25/ticket-map.json) is static ownership evidence,
not an active plan. Exact-source verification, consumer adoption and
nightly/release qualification retain separate authorities.

## Decision

1. Keep the implemented contract in ADR 0004–0005, benchmark interpretation in
   ADR 9000, and source-bound proof rules in ADR 0006. Keep this compact ticket
   map as the index of what was implemented; do not recreate ticket-level plans.
2. Keep the 104-row scenario manifest, its validators and ticket ownership map
   as evidence. Current open work belongs to one current-source plan. `MAPPED`
   is a static relationship; it is not an executed scenario or a CI PASS.
3. Keep historical exact-source receipts at their recorded source identities.
   A later HEAD needs a new complete clean-source receipt. The removed narratives
   remain inspectable in the Git commit named above; their status is historical.

## Completed ticket map

| Ticket | Implemented repository boundary | Durable authority |
|---|---|---|
| BG25-001 | Separated trust, identity, deadline, executor, and proof decisions. | 0004–0006; D1–D9 deployment review remains open. |
| BG25-002 | Added bounded strict task/config bytes ingress without silently tightening raw DTO Serde. | 0004; deployed bytes ingress remains an owner decision. |
| BG25-003 | Made wire/version/error behavior explicit for consumers. | 0004; downstream compatibility and migration remain open. |
| BG25-004 | Bound opaque permit/ticket authority to its Governor; foreign same-sequence handles and exhaustion reject. | 0004. |
| BG25-005 | Froze one validated executor descriptor across preflight, dispatch, and exposed topology; missing Tokio context rejects before work. | 0005. |
| BG25-006 | Separated caller response, deadline, worker cleanup, lease release, and one-way drain. The later H12/H31 fixtures cover the corrected pre-drain custody order. | 0005; no dropped-caller response or post-drain admission is inferred. |
| BG25-007 | Kept stages/reduce as metadata and child/fan-out lifetime with the caller unless explicitly awaited or governed. | 0005; no implicit reducer execution. |
| BG25-008 | Reserved semantic and physical capacity in one Governor transition; covered blocker precedence and exact boundary values. | 0005; cross-runtime bounds require shared authority. |
| BG25-009 | Added bounded queue, fairness, waker, cancellation, and drain-history oracles without a scheduler rewrite. | 0005; bounded histories do not enumerate every interleaving. |
| BG25-010 | Added combined memory-ledger and release-history oracles. | 0005; no production defect was inferred solely from missing coverage. |
| BG25-011 | Separated deterministic simulator accounting from public-host response/custody evidence. | 9000; H28's small host/simulator case is not performance qualification. |
| BG25-012 | Mapped K 51 / P 34 / G 19 scenarios to versioned selectors and exact-source proof roles. | 0006; final clean HEAD CI receipt remains required. |

The per-scenario ownership and executable selectors remain in
[ticket-map.json](../evidence/sep25/ticket-map.json) and
[scenario-evidence.json](../evidence/sep25/scenario-evidence.json). These are
static evidence inventories, not duplicate implementation tickets.

## Evidence and remaining work

Older macOS 16/16 and focused consumer receipts are valid only for their exact
source and proof type. The implementation record, static 104-row mapping, and
owner-local runs do not qualify a later HEAD. W4 branch-rule adoption is
complete at the 2026-10-03 observation. The [current plan](../plans/2026-10-03-current-source-remediation.md)
owns new clean-source CI, D1–D9 consumer review, conditional H28 performance,
and explicitly requested nightly/release producers. Mutation,
modelcheck, TSan, fuzz, coverage, IAI, and release are not completed by this ADR.
