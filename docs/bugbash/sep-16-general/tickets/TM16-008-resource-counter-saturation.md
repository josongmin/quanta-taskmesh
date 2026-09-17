# TM16-008 — Resource counter saturation이 MAX budget의 초과 admission을 숨긴다

- Severity: P2
- Status: OPEN
- Lane: engine-accounting
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)
- Evidence: evidence/repro/src/lib.rs (현재 잘못된 동작을 assert하는 observation probe)

## 근거

crates/taskmesh-engine/src/engine/state.rs:148-169,192-210,238-250,278-305; crates/taskmesh-contract/src/resource.rs:units_for

## Trigger / 관찰

global CPU budget=u32::MAX, cost=2^31, max_inflight>=2인 동일 class에서 두 root를 admit한다.

두 작업이 모두 Admitted다. 실제 합 4,294,967,296은 budget 4,294,967,295보다 크다. held counter는 u32::MAX로 포화되고 invariant의 permit 합도 포화되어 이 시점에는 문제를 감추는다.

## 원인 / 영향 / 범위

saturating_add(cost)>budget은 budget이 MAX일 때 overflow를 detect하지 못한다. release에서 포화합에 단일 cost를 빼면 남은 permit ledger와 held가 달라질 수 있다. memory/global/class/root counter도 같은 수학을 사용한다. 통상 작은 budgets가 아니라 public 입력 경계값에서 발생한다.

## 보완 계획

capacity는 checked_add 또는 u64 합으로 검증하고 ledger/global/class/root totals는 충분한 폭으로 보존한다. overflow를 정상 값으로 coercion하지 않는다. snapshot u32 호환이 필요하면 typed invalid config/overflow 처리 경계를 명시한다.

## Acceptance / 회귀 검증

CPU와 memory 각각 MAX-1/MAX 경계, 2개 이상의 큰 cost, admission/reconcile/release 순열을 debug/release 모두에서 검증한다. 합계 불변식은 테스트에서 u64 oracle로 계산한다.

