# Bugbash Sep-25 general — final resolution plan

- Status: **PARTIAL** overall. BG25-001–012의 저장소 구현과 104행 정적 매핑은 완료됐다. 저장소 CI 자격은 BG25-012의 현재 clean HEAD 외부 영수증 검증으로만 판정한다. D1–D9 사람 검토·외부 채택은 OPEN이고 nightly/release는 미실행이다. [현재 후속 작업](OPEN-FOLLOWUPS.md)과 [아카이브 감사 기록](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/FINAL-REVIEW.md)을 구분한다.
- Implementation ticket narratives are [archived](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/README.md); [open follow-ups](OPEN-FOLLOWUPS.md), the scenario manifest, and external adoption ledger remain active here. Durable library decisions are in [ADR 0004](../../../adr/0004-sep-25-ingress-plan-identity-and-wire.md), [ADR 0005](../../../adr/0005-sep-25-execution-response-and-custody.md), and [ADR 0006](../../../adr/0006-source-bound-verification-authority.md).
- Audit source: `76559c483bdd1d6b0b5226f7c5b5591b919afae9`, tree `215a968630e9f0675bfd1e42f0919bd2047bded2`. Implementation incorporated the later strict-ingress and cross-Governor reproduction history before final source-bound proof.
- Scenario source: [104-case audit](../../../misc/tmp-engine-checklist-sep-25.md). Its static result is K 51 / P 34 / G 19; all 53 P/G scenarios are assigned exactly once here.
- Predecessor: [`docs/archive/2026-09-25/sep-25-engine-coverage/tickets/`](../../../archive/2026-09-25/sep-25-engine-coverage/tickets/README.md) is the superseded draft. This directory is the final bugbash execution packet; do not execute both as separate plans.
- Scope: contract, engine, Tokio facade, Rayon adapter, benchmark harness, and proof rails. Distributed scheduling, durable workflow recovery, automatic checkpoint execution, caller-owned fan-out/reduce execution, and undeclared closure waits remain out of scope.

## Read order

1. [AUDIT](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/AUDIT.md) — confirmed problems, contract decisions, and proof-only gaps.
2. [ARCHITECTURE](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/ARCHITECTURE.md) — AS-IS and structural TO-BE.
3. [DECISIONS](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/DECISIONS.md) — historical D1–D9 table; ADR 0004–0005 are the current library decision record.
4. [EXECUTION](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/EXECUTION.md) — dependency DAG, parallel lanes, and write ownership.
5. [COVERAGE](COVERAGE.md) — all 53 P/G scenarios mapped to one owner ticket.
6. [VERIFICATION](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/VERIFICATION.md) — evidence and exact-source closure rules.
7. [COMMANDS](COMMANDS.md) — exact owner-local and integration commands per ticket.
8. [FINAL REVIEW](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/FINAL-REVIEW.md) — SOLID/minimality and last-gap audit.
9. [EXTERNAL ADOPTION](EXTERNAL-ADOPTION.md) — observed consumer candidate and unverified deployment boundaries.
10. [OPEN FOLLOW-UPS](OPEN-FOLLOWUPS.md) — owner actions after implementation archive.
11. [plan.json](plan.json) — machine-readable ownership and dependency graph.

## Tickets

| Ticket | Priority | Objective | Depends on |
|---|---|---|---|
| [BG25-001](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/BG25-001-contract-boundaries.md) | P0 | Freeze trust, identity, deadline, and executor contracts | — |
| [BG25-002](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/BG25-002-strict-ingress.md) | P0 | Add bounded strict ingress without breaking raw DTO compatibility | 001 |
| [BG25-003](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/BG25-003-wire-consumer-boundaries.md) | P1 | Make wire/version/error consumer behavior explicit | 001 |
| [BG25-004](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/BG25-004-identity-handles.md) | P1 | Replace or constrain cross-Governor raw identities | 001 |
| [BG25-005](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/BG25-005-executor-authority.md) | P0 | Freeze the validated executor descriptor and context boundary | 001 |
| [BG25-006](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/BG25-006-deadline-custody.md) | P0 | Separate caller response, runtime cleanup, and worker custody | 001, 005 |
| [BG25-007](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/BG25-007-root-child-lifetime.md) | P1 | Specify caller-owned stages and detached child lifetime | 001, 006 |
| [BG25-008](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/BG25-008-admission-capacity.md) | P1 | Prove compound capacity and blocker precedence | 001, 005 |
| [BG25-009](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/BG25-009-queue-races.md) | P1 | Prove queue, fairness, waker, and drain histories | 001, 008 |
| [BG25-010](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/BG25-010-memory-ledger.md) | P1 | Prove combined memory ledger histories | 008, 009 |
| [BG25-011](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/BG25-011-load-evidence.md) | P1 | Separate simulator metrics from real host load evidence | 005, 006, 008 |
| [BG25-012](../../../archive/2026-09-25/bugbash-sep-25-general/tickets/BG25-012-proof-integration.md) | P1 | Bind every implemented scenario to selected gates and exact source | 002–011 |

## Completion boundary

A ticket's implementation can complete independently of proof and external adoption. BG25-012 has inventoried all 104 scenarios, preserved the 51 K baselines, and mapped the 53 P/G rows to declared test cases and explicit supporting cases. The validator rejects ignored or feature-disabled cases and verifies exact Rayon selectors; it does not execute tests. 현재 HEAD의 clean macOS CI 자격은 저장소 외부 영수증을 `--expected-head`로 재검증해서만 확정한다. D1–D9 외부 채택은 OPEN이다. Nightly/modelcheck/TSan/fuzz/mutation은 별도 요청과 증거가 필요하다.
