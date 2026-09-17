# TM16-005 — Stage 조기 반환의 release policy 계약이 불분명하다

- Severity: P3
- Status: CONTRACT_DECISION
- Lane: engine-memory
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

crates/taskmesh-contract/src/policy.rs:41-47; crates/taskmesh-engine/src/engine/governor.rs:230-244; crates/taskmesh-engine/src/features/memory/mod.rs:91-121

## Trigger / 관찰

`OnTaskCompletion` class에 memory_units=10을 예약한 permit으로 release_stage_memory(p,10)을 호출한다.

inflight=1 상태에서 held memory=0이 된다. stage release가 memory_release_policy를 읽지 않는다. LeakDetecting 여부만 sweep에서 읽으므로 OnStageBoundary의 독립 동작이 없다.

## 원인 / 영향 / 범위

'When held memory units are returned'라는 public policy 설명과 무관하게 advanced caller가 조기 반환 가능하다. 하지만 advanced API는 trusted host hooks로도 해석할 수 있으므로 이 관찰 하나로 독립적인 runtime 결함을 확정하지 않는다. 정책이 metadata인지 강제 계약인지 명확히 해야 한다. 이것이 실제 프로세스 메모리를 자동 추적한다는 뜻은 아니다.

## 보완 계획

policy를 authoritative로 만들면 OnStageBoundary/허용된 LeakDetecting에서만 stage release를 허용한다. 강제 조정 API가 필요하면 별도 이름/권한/설명으로 분리한다. stage activity의 touch 및 reconcile와의 상호작용을 함께 설계한다.

## Acceptance / 회귀 검증

세 release policy의 matrix, 반환 후 reconcile, stage activity 후 stale sweep, root/class/global 합과 promote를 확인한다.
