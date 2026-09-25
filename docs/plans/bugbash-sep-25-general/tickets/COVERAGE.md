# Scenario ownership

Every P/G scenario in the 104-case audit has exactly one primary ticket. Existing K scenarios remain regression baselines and are not duplicated here.

| Ticket | Scenarios | Count |
|---|---|---:|
| [BG25-002](BG25-002-strict-ingress.md) | D04, D21, D22, D25, H33 | 5 |
| [BG25-003](BG25-003-wire-consumer-boundaries.md) | B18, B24, D03, D13, D14, D17, D23 | 7 |
| [BG25-004](BG25-004-identity-handles.md) | B21, B22, B28, H18 | 4 |
| [BG25-005](BG25-005-executor-authority.md) | B25, H15, H27, H30, D16 | 5 |
| [BG25-006](BG25-006-deadline-custody.md) | D05, D15, H11, H12, H19, H20, H31, H34 | 8 |
| [BG25-007](BG25-007-root-child-lifetime.md) | A11, H21, H35 | 3 |
| [BG25-008](BG25-008-admission-capacity.md) | A05, B05, B20, H01, H02, H07, H32, D01 | 8 |
| [BG25-009](BG25-009-queue-races.md) | B23, B27, H04, H06, H13, H14, H16, H25, D19 | 9 |
| [BG25-010](BG25-010-memory-ledger.md) | H10, D20 | 2 |
| [BG25-011](BG25-011-load-evidence.md) | H17, H28 | 2 |
| **Total** | A 2 + B 10 + H 26 + D 15 | **53** |

BG25-001 owns cross-cutting decisions; BG25-012 owns evidence integration. Neither owns scenario IDs.

`scenario-evidence.json` retains all 104 rows, their historical K/P/G origin, the exact selected test case, feature/platform selector, gate, source path, and current status. `validate_scenario_evidence.py` rejects missing IDs, duplicate ownership, nonexistent cases, uninventoried gates, and summary drift. Current deterministic status is 104 PASS; nightly qualification remains separately recorded as NOT_RUN.
