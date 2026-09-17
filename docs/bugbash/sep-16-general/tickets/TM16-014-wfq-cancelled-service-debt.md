# TM16-014 — Abandoned WFQ 요청이 phantom service debt를 남긴다

- Severity: P2
- Status: OPEN
- Lane: fairness
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

crates/taskmesh-engine/src/features/fairness/scheduler.rs:33-41; crates/taskmesh-engine/src/engine/governor.rs:129-142

## Trigger / 관찰

capacity를 점유한 상태에서 a의 요청 enqueue/abandon을 10회 반복한다. 이후 동일 weight a를 먼저, b를 나중 queue한다.

어떤 a 작업도 실행되지 않았지만 a.last_finish_tag가 누적되어 나중 b가 먼저 promote된다. retained probe 재현; b 10개를 추가하면 이들 뒤까지 a service가 밀릴 수 있다.

## 원인 / 영향 / 범위

queue에서 제거할 때 virtual service debt를 조정하지 않는다. 취소/timeout이 많은 class가 실제 service와 무관하게 불리해진다. production fairness semantics가 canceled reservation의 debt를 의도하는지 명시되어 있지 않다.

## 보완 계획

empty queue에서 future baseline을 global virtual time/실제 service debt에 맞추거나 queue tag를 cancellation-safe하게 계산한다. head/middle/tail removal의 relative order를 유지한다.

## Acceptance / 회귀 검증

abandon burst 뒤 fresh equal-weight enqueue, nonempty queue cancellation, timeout/drop, actual dispatched work와의 credit 차이를 검증한다.

