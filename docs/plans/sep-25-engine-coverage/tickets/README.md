# Sep-25 engine coverage — superseded draft

> **SUPERSEDED:** 이 초안의 실행 권위는
> [`docs/plans/bugbash-sep-25-general/tickets/`](../../bugbash-sep-25-general/tickets/README.md)로
> 이전됐다. 역사적 감사 상세는 보존하지만 이 디렉터리의 티켓을 별도 계획으로 실행하거나
> 새 패킷과 합산하지 않는다.

- 상태: **SUPERSEDED**. 소스 수정, 새 회귀 실행, CI qualification은 이 초안에 포함되지 않았다.
- 감사 기준: `main@76559c483bdd1d6b0b5226f7c5b5591b919afae9`, 2026-09-25. 기준 시 tracked tree는 clean, `docs/misc/`는 기존 untracked 작업물이었다. 실행 전 HEAD와 dirty owner를 다시 고정한다.
- 원본 시나리오: [engine checklist](../../../misc/tmp-engine-checklist-sep-25.md). 104개 중 K 51/P 34/G 19. 이 계획은 P/G **53개를 한 번씩** 주 담당 티켓에 배정한다. K는 모든 교차 조합의 통과를 뜻하지 않는다.
- 목적: 엔진의 실제 보장, 공개 계약, 외부 ingress/consumer 책임, 증거 강도를 정렬한다. fixture 개수나 coverage 백분율만으로 완료 처리하지 않는다.

## 읽는 순서

1. [목표 경계와 AS-IS → TO-BE](ARCHITECTURE.md)
2. [계약 결정안](DECISIONS.md)
3. [의존성·소유권·실행 순서](EXECUTION.md)
4. [시나리오 배정](COVERAGE.md) 및 [기계 판독 inventory](plan.json)
5. [증거·gate 기준](VERIFICATION.md)
6. 아래 티켓의 실제 변경 범위와 acceptance

## 티켓

| ID | 작업 | 우선 | 선행 |
|---|---|---|---|
| [S25-001](S25-001-contract-boundaries.md) | ingress·identity·deadline·executor 계약 고정 | P0 | — |
| [S25-002](S25-002-strict-ingress.md) | untrusted JSON 승격 경계 | P0 | 001 |
| [S25-003](S25-003-wire-and-input-boundaries.md) | wire·입력·consumer 실패 의미 | P1 | 001 |
| [S25-004](S25-004-identity-and-plan-scope.md) | identity·parent plan·Governor handle | P1 | 001 |
| [S25-005](S25-005-executor-authority.md) | 고정 executor 선언·accepted custody·공유 pool | P0 | 001 |
| [S25-006](S25-006-deadline-and-custody.md) | 응답 기한과 lease 종료 분리 | P0 | 001, 005 |
| [S25-007](S25-007-root-child-boundary.md) | stage 선언과 detached child 수명 | P1 | 001, 006 |
| [S25-008](S25-008-admission-capacity.md) | 복합 capacity·boundedness | P1 | 001, 005 |
| [S25-009](S25-009-queue-races.md) | queue·scheduler·drain 경합 | P1 | 001, 008 |
| [S25-010](S25-010-memory-ledger.md) | stage memory·reconcile 원장 | P1 | 008, 009 |
| [S25-011](S25-011-load-evidence.md) | simulator·실제 host 부하 | P1 | 005, 006, 008 |
| [S25-012](S25-012-gate-integration.md) | scenario 증거·feature·CI 통합 | P1 | 002–011 |

의존성은 **통합 완료 선행**이다. 다른 lane의 read-only 조사와 독립 fixture 설계는 먼저 가능하다. 공용 파일 수정은 [EXECUTION](EXECUTION.md)의 단일 write owner가 적용한다.

## 판정 규칙

- **source-backed 실패/계약 후보:** H30(제출 때 executor 선언 재조회와 `expect`), H34(정상 requested-stack 결과가 owned runtime teardown 뒤 전달). H34의 `RunFor` 실행 예산은 현행 Accepted 계약이므로 `CompleteBy` 응답 범위와 분리해 결정한다. 반례를 재현하고 목표 계약에서 실제 실패한 경로만 수정한다.
- **계약/신뢰 경계:** D21/D22/D25/H33, B22/B28/B25 등은 현행 동작과 목표 동작을 분리한다. 라이브러리 raw Serde와 배포 ingress를 같은 보장으로 합치지 않는다.
- **증거 공백:** 남은 P/G는 핵심 반례와 독립 oracle이 없는 상태다. 현행 소스 결함이라는 선언이 아니다.
- **종료:** 티켓의 acceptance마다 exact-source fixture/계약 근거, 관련 owner-local 결과, 깨끗한 동일 HEAD의 CI profile을 연결한다. nightly/release, 외부 consumer, hosted merge/activation은 각기 다른 증거다.

"SOTA++"는 이 계획의 설계 목표인 단일 권위, 유한 소유권, 정확한 trust boundary, 독립 oracle, source-bound gate를 뜻한다. 외부 라이브러리보다 우월한 성능을 측정했다는 뜻이 아니다.
