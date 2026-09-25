# Bugbash Sep-25 general — final resolution plan

- Status: **PARTIAL** overall. BG25-001–012의 저장소 구현과 104행 정적 매핑은 완료됐다. 저장소 CI 자격은 BG25-012의 현재 clean HEAD 외부 영수증 검증으로만 판정한다. D1–D9 사람 검토·외부 채택은 OPEN이고 nightly/release는 미실행이다. [최종 재감사](FINAL-REVIEW.md)를 참조한다.
- Audit source: `76559c483bdd1d6b0b5226f7c5b5591b919afae9`, tree `215a968630e9f0675bfd1e42f0919bd2047bded2`. Implementation incorporated the later strict-ingress and cross-Governor reproduction history before final source-bound proof.
- Scenario source: [104-case audit](../../../misc/tmp-engine-checklist-sep-25.md). Its static result is K 51 / P 34 / G 19; all 53 P/G scenarios are assigned exactly once here.
- Predecessor: `docs/plans/sep-25-engine-coverage/tickets/` is a tracked superseded draft. This directory is the final bugbash execution packet; do not execute both as separate plans.
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
9. [EXTERNAL ADOPTION](EXTERNAL-ADOPTION.md) — observed consumer candidate and unverified deployment boundaries.
10. [plan.json](plan.json) — machine-readable ownership and dependency graph.

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

A ticket's implementation can complete independently of proof and external adoption. BG25-012 has inventoried all 104 scenarios, preserved the 51 K baselines, and mapped the 53 P/G rows to declared test cases and explicit supporting cases. The validator rejects ignored or feature-disabled cases and verifies exact Rayon selectors; it does not execute tests. 현재 HEAD의 clean macOS CI 자격은 저장소 외부 영수증을 `--expected-head`로 재검증해서만 확정한다. D1–D9 외부 채택은 OPEN이다. Nightly/modelcheck/TSan/fuzz/mutation은 별도 요청과 증거가 필요하다.
