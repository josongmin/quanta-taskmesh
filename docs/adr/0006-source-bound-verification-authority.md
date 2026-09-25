# 0006. Source-bound verification authority

- Status: Accepted for current local verification policy
- Date: 2026-09-25
- Source: [Sep-25 scenario inventory](../misc/tmp-engine-checklist-sep-25.md),
  [BG25 verification](../archive/2026-09-25/bugbash-sep-25-general/tickets/VERIFICATION.md),
  and [verification stages](../plans/2026-09-24-ci-verification-stages.md)

## Context

A path-to-test mapping, a selected test invocation, and a complete gate receipt
answer different questions. A PASS attached to one source revision cannot
qualify a later document or code commit. An external consumer run does not prove
which mutable path dependency was built unless that dependency is bound in its
receipt.

## Decision

1. The 104-scenario inventory is the oracle index. Its K 51 / P 34 / G 19
   baseline and the 104-row manifest describe static target, case, oracle,
   feature, platform, and gate selection. `MAPPED` means that relationship was
   checked; it does not mean the case executed or the scenario passed.
2. Owner-local and `dev` results are diagnostic. The CI profile is qualified
   only by a complete clean-source receipt with all applicable required gates
   PASS, no required NOT_RUN or FAIL, and stable HEAD, tree, and counted path
   digest. The receipt is valid for its exact source only.
3. Nightly producers, release adjudication, performance comparisons, and
   external deployment adoption have their own denominators and authorities.
   A focused run, partial campaign, simulator metric, or static ticket status
   cannot substitute for them.
4. Gate membership and commands remain in `tools/gates/required.json`,
   `tools/gates/inventory.json`, and `Justfile`; the scenario manifest is a
   selected-case provenance map, not a second gate inventory.
5. Historic receipts and superseded plans remain immutable evidence. A new
   source or tool revision needs a new receipt, and a consumer path dependency
   needs an exact dependency identity before it can support source attribution.

## Consequences

The [release checklist](../release-checklist.md) remains the operational
procedure. The [verification-stage plan](../plans/2026-09-24-ci-verification-stages.md)
still has open W3 execution-denominator work, W4 hosted adoption, and W5 release
qualification; this ADR does not mark those waves complete. The scenario
manifest and validators remain executable evidence, even when completed ticket
narratives are archived.
