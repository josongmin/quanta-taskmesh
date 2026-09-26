# BG25 active proof and adoption packet

- Status: **PARTIAL**. BG25-001–012 repository implementation is recorded in
  [ADR 0007](../../../adr/0007-sep-25-implementation-closure.md). This packet
  retains the unclosed proof and deployment work.
- The [104-case checklist](../../../misc/tmp-engine-checklist-sep-25.md) has
  K 51 / P 34 / G 19 historical classifications. All 53 P/G IDs have exactly
  one static owner in [plan.json](plan.json) and [COVERAGE.md](COVERAGE.md).
  Static selection does not establish execution or a qualified current HEAD.
- The original implementation narratives are available in Git at
  `cbf9764f08eb9f307f3ee5a51bad5e5dabeaf4a6`.

## Current authorities

| Question | Authority |
|---|---|
| Implemented library boundaries | [ADR 0004](../../../adr/0004-sep-25-ingress-plan-identity-and-wire.md), [ADR 0005](../../../adr/0005-sep-25-execution-response-and-custody.md), [ADR 0007](../../../adr/0007-sep-25-implementation-closure.md) |
| Source-bound verification | [ADR 0006](../../../adr/0006-source-bound-verification-authority.md), [scenario-evidence.json](scenario-evidence.json), [COMMANDS.md](COMMANDS.md) |
| Open proof, CI adoption, nightly/release | [OPEN-FOLLOWUPS.md](OPEN-FOLLOWUPS.md), [CI-stage plan](../../2026-09-24-ci-verification-stages.md), [release checklist](../../../release-checklist.md) |
| Consumer applicability and rollout | [EXTERNAL-ADOPTION.md](EXTERNAL-ADOPTION.md) |

`plan.json` retains the 12-ticket dependency and status inventory because its
proof and external fields remain open. `validate_plan.py` checks the mapping,
dependency DAG, ADR rows and live links; `validate_scenario_evidence.py` checks
the selected cases and gate inventory. A clean exact-HEAD CI receipt, W4
branch-rule adoption and D1–D9 deployment decisions remain separate work.
