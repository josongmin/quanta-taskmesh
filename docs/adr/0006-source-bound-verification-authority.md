# 0006. Source-bound verification authority

- Status: Accepted for current local verification policy
- Date: 2026-09-25
- Source: [Sep-25 scenario inventory](../misc/tmp-engine-checklist-sep-25.md),
  [BG25 implementation map](0007-sep-25-implementation-closure.md),
  and `Justfile` / `tools/gates/{inventory,required}.json`

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
5. Historic receipts remain under `docs/evidence`; superseded plan text remains
   in Git history. A new source or tool revision needs a new receipt, and a
   consumer path dependency needs an exact dependency identity before it can
   support source attribution.
6. `local` and `dev` are diagnostic feedback scopes; `ci` is the 16-gate
   clean-source profile; `nightly` is the seven-gate costly proof profile; and
   `release` additionally needs release-only artifacts and human adjudication.
   Profile, trigger and proof strength are independent. `nightly` is not a cron.
7. The bounded `.github/workflows/pr-ci.yml` runs on PR/main and emits one
   aggregate `required 16-gate verdict`. At the 2026-10-03 observation, GitHub
   `main` protection required that context with strict status checks; its
   [main-push run](https://github.com/josongmin/quanta-taskmesh/actions/runs/36261855351)
   passed at `c4bcb2f`. This is an exact-source merge control, not nightly,
   release, performance or consumer adoption approval.

## Completed verification changes

- The Sep-22 test-optimization tickets added focused cancellation,
  backpressure, drain, Rayon and benchmark structural oracles plus Python-floor
  and Semgrep enrollment checks. Their owner-local results remain scoped to
  their recorded source; no broad release verdict is inferred.
- The Sep-24 W1–W3 work strengthened real-tree Semgrep enrollment, complete
  fairness drain assertions, discovered Rust/Python/fuzz target selection, and
  source-bound gate receipts. W4 added the bounded hosted PR/main workflow and
  the strict required branch rule described above.
- A gate's static target catalog, a focused test result, a 16-gate receipt,
  nightly producer artifacts and a release decision retain distinct
  denominators. The command authority remains the current inventory and
  `Justfile`; historical stage/ticket plans are available in Git history.

## Retired plan disposition

| Removed planning packet | Durable decision or evidence | Open authority |
|---|---|---|
| Jun-4 startup and Sep-16 hardening | ADR 0001–0003; Sep-16 audit and receipts under `docs/audits` and `docs/evidence/sep16` | Current plan and release checklist |
| Sep-22 test optimization and Sep-24 CI stages | This ADR; historical measurement JSON under `docs/evidence/sep22` | New exact-source receipt for later commits |
| S25/BG25 implementation | ADR 0004–0007; 104-case static mapping under `docs/evidence/sep25` | D1–D9 adoption, performance and release in current plan |
| B04/B07 benchmark packets | ADR 9000 for measurement and completed diagnostic boundaries | Measured admission and qualified series in current plan |
| SEP-27 SDK proposals | No unimplemented proposal was promoted to an ADR | Adopted facade error exports and Rayon constructor change in the current source; further DX work needs a concrete consumer contract |

## Consequences

The [release checklist](../release-checklist.md) remains the operational
procedure. W5 release qualification and external deployment adoption remain
open in the [current plan](../plans/2026-10-03-current-source-remediation.md).
The [scenario manifest](../evidence/sep25/scenario-evidence.json) and validators
remain executable static evidence, even after old plans are removed.
