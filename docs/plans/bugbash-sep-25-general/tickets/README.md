# Bugbash Sep-25 general — final resolution plan

- Status: **PARTIAL** overall. Repository implementation is complete; exact-source CI qualification, D1–D9 human contract review, and external ingress/wire adoption remain separate authorities. The 104-row manifest is a static candidate map, not an execution receipt.
- Audit source: `76559c483bdd1d6b0b5226f7c5b5591b919afae9`, tree `215a968630e9f0675bfd1e42f0919bd2047bded2`. Implementation incorporated the later strict-ingress and cross-Governor reproduction history before final source-bound proof.
- Scenario source: [104-case audit](../../../misc/tmp-engine-checklist-sep-25.md). Its static result is K 51 / P 34 / G 19; all 53 P/G scenarios are assigned exactly once here.
- Predecessor: `docs/plans/sep-25-engine-coverage/tickets/` is an untracked draft. This directory is the final bugbash execution packet; do not execute both as separate plans.
- Scope: contract, engine, Tokio facade, Rayon adapter, benchmark harness, and proof rails. Distributed scheduling, durable workflow recovery, automatic checkpoint execution, caller-owned fan-out/reduce execution, and undeclared closure waits remain out of scope.

## Read order

1. [AUDIT](AUDIT.md) — confirmed problems, contract decisions, and proof-only gaps.
2. [ARCHITECTURE](ARCHITECTURE.md) — AS-IS and structural TO-BE.
3. [DECISIONS](DECISIONS.md) — decisions that must be accepted before public API work.
4. [EXECUTION](EXECUTION.md) — dependency DAG, parallel lanes, and write ownership.
5. [COVERAGE](COVERAGE.md) — all 53 P/G scenarios mapped to one owner ticket.
6. [VERIFICATION](VERIFICATION.md) — evidence and exact-source closure rules.
7. [COMMANDS](COMMANDS.md) — exact owner-local and integration commands per ticket.
8. [FINAL REVIEW](FINAL-REVIEW.md) — SOLID/minimality and last-gap audit.
9. [plan.json](plan.json) — machine-readable ownership and dependency graph.

## Tickets

| Ticket | Priority | Objective | Depends on |
|---|---|---|---|
| [BG25-001](BG25-001-contract-boundaries.md) | P0 | Freeze trust, identity, deadline, and executor contracts | — |
| [BG25-002](BG25-002-strict-ingress.md) | P0 | Add bounded strict ingress without breaking raw DTO compatibility | 001 |
| [BG25-003](BG25-003-wire-consumer-boundaries.md) | P1 | Make wire/version/error consumer behavior explicit | 001 |
| [BG25-004](BG25-004-identity-handles.md) | P1 | Replace or constrain cross-Governor raw identities | 001 |
| [BG25-005](BG25-005-executor-authority.md) | P0 | Freeze the validated executor descriptor and context boundary | 001 |
| [BG25-006](BG25-006-deadline-custody.md) | P0 | Separate caller response, runtime cleanup, and worker custody | 001, 005 |
| [BG25-007](BG25-007-root-child-lifetime.md) | P1 | Specify caller-owned stages and detached child lifetime | 001, 006 |
| [BG25-008](BG25-008-admission-capacity.md) | P1 | Prove compound capacity and blocker precedence | 001, 005 |
| [BG25-009](BG25-009-queue-races.md) | P1 | Prove queue, fairness, waker, and drain histories | 001, 008 |
| [BG25-010](BG25-010-memory-ledger.md) | P1 | Prove combined memory ledger histories | 008, 009 |
| [BG25-011](BG25-011-load-evidence.md) | P1 | Separate simulator metrics from real host load evidence | 005, 006, 008 |
| [BG25-012](BG25-012-proof-integration.md) | P1 | Bind every implemented scenario to selected gates and exact source | 002–011 |

## Completion boundary

A ticket's implementation can complete independently of proof and external adoption. The packet remains partial until BG25-012 inventories all 104 scenarios, preserves the 51 existing K baselines, maps the 53 assigned P/G rows, and a clean unchanged-HEAD CI-profile receipt proves the selected gates. D1–D9 human review and external deployment adoption remain explicit open states. Nightly/modelcheck/TSan/fuzz/mutation remain separate explicit-cost proof and are not implied by CI-profile completion.
