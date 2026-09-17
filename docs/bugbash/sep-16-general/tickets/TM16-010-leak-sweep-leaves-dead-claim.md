# TM16-010 — Leak sweep 후 reclaimed permit의 ticket이 claim 가능하다

- Severity: P1
- Status: OPEN
- Lane: engine-lifecycle
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

crates/taskmesh-engine/src/features/memory/mod.rs:152-154; crates/taskmesh-engine/src/engine/state.rs:238-270,314-318; crates/taskmesh-engine/src/engine/governor.rs:115-117

## Trigger / 관찰

LeakDetecting class에서 holder 때문에 두 번째 요청을 queue하고 holder release로 promote한다. 두 번째 ticket을 claim하지 않은 채 stale window를 넘겨 sweep한다.

debug에서는 'granted ticket maps to a permit that no longer exists' panic. release에서는 reclaimed permit ID가 claim으로 반환되며 permit_provenance는 None, inflight는 0이다. retained probe가 debug/release 모두 재현한다.

## 원인 / 영향 / 범위

unwind가 permits에서 permit을 제거하지만 granted ticket→permit mapping은 남긴다. host waiter가 이후 dead permit을 받아 작업을 실행하면 governed accounting 없이 실행한다. 단순 mapping 삭제만 하면 awaiter가 영구 대기할 수 있어 terminal state/wakeup도 필요하다.

## 보완 계획

ticket/permit ledger를 하나의 lifecycle 전이로 invalidate한다. claim은 live ownership만 반환하고 reclaimed ticket에는 terminal acquisition outcome을 전달한다. generic release 경로와 sweep 모두 promoted-unclaimed ownership을 일관되게 처리한다.

## Acceptance / 회귀 검증

promote→sweep→claim, sweep와 claim/abandon 동시성, 반복 sweep, child recursion guard/root attribution 정리, governor와 host awaiter 종료를 debug/release에서 확인한다.

