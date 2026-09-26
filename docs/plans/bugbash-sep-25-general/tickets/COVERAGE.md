# Scenario ownership

Every P/G scenario in the 104-case audit has exactly one primary ticket. Existing K scenarios remain regression baselines and are not duplicated here.

| Ticket | Scenarios | Count |
|---|---|---:|
| [BG25-002](../../../adr/0007-sep-25-implementation-closure.md#completed-ticket-map) | D04, D21, D22, D25, H33 | 5 |
| [BG25-003](../../../adr/0007-sep-25-implementation-closure.md#completed-ticket-map) | B18, B24, D03, D13, D14, D17, D23 | 7 |
| [BG25-004](../../../adr/0007-sep-25-implementation-closure.md#completed-ticket-map) | B21, B22, B28, H18 | 4 |
| [BG25-005](../../../adr/0007-sep-25-implementation-closure.md#completed-ticket-map) | B25, H15, H27, H30, D16 | 5 |
| [BG25-006](../../../adr/0007-sep-25-implementation-closure.md#completed-ticket-map) | D05, D15, H11, H12, H19, H20, H31, H34 | 8 |
| [BG25-007](../../../adr/0007-sep-25-implementation-closure.md#completed-ticket-map) | A11, H21, H35 | 3 |
| [BG25-008](../../../adr/0007-sep-25-implementation-closure.md#completed-ticket-map) | A05, B05, B20, H01, H02, H07, H32, D01 | 8 |
| [BG25-009](../../../adr/0007-sep-25-implementation-closure.md#completed-ticket-map) | B23, B27, H04, H06, H13, H14, H16, H25, D19 | 9 |
| [BG25-010](../../../adr/0007-sep-25-implementation-closure.md#completed-ticket-map) | H10, D20 | 2 |
| [BG25-011](../../../adr/0007-sep-25-implementation-closure.md#completed-ticket-map) | H17, H28 | 2 |
| **Total** | A 2 + B 10 + H 26 + D 15 | **53** |

BG25-001 owns cross-cutting decisions; BG25-012 owns evidence integration. Neither owns scenario IDs.

`scenario-evidence.json` retains all 104 rows, their historical K/P/G origin, a candidate test case, feature/platform selector, gate, source path, and static status. `validate_scenario_evidence.py` rejects missing IDs, duplicate ownership, nonexistent cases, uninventoried gates, summary drift, and a Rayon selector absent from `test-rayon`. Current status is 104 MAPPED: this is inventory, not semantic sufficiency or execution PASS. CI and nightly qualification remain separate receipts; nightly is explicitly NOT_RUN.

2026-09-26 semantic re-audit at clean `0588d26847ba575f1c059f8d86a37d8420ea5589`: A05 now links the existing input-derived cross-class permit ledger. H12's corrected combined barrier case is the primary mapping and passed focused 1/1 plus `just dev` 558/558 on the candidate tree; clean-HEAD CI qualification remains [open](OPEN-FOLLOWUPS.md). H28's selected case proves only a controlled host/simulator admission-count comparison; class/path response populations, surviving custody, and warmup/environment belong to separate host performance qualification. The corrected scenario oracles are in the [checklist](../../../misc/tmp-engine-checklist-sep-25.md). No engine defect was established by this audit.
