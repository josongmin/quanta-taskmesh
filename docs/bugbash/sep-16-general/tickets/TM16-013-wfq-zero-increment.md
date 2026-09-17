# TM16-013 — WFQ weight precision 소실로 weighted share가 FIFO로 퇴화한다

- Severity: P2
- Status: OPEN
- Lane: fairness
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

crates/taskmesh-engine/src/features/fairness/scheduler.rs:20,35-41,84-86; crates/taskmesh-engine/src/engine/governor.rs:353-463

## Trigger / 관찰

CPU cost=1의 두 class weight를 1:2에서 2,000,000:4,000,000으로 동일 비율 scaling하고 강제 queue 후 순서를 비교한다.

1:2에서는 높은 weight의 나중 b가 먼저 promote되지만 큰 weights에서는 먼저 enqueue된 a가 먼저 promote된다. increment=(cost*1,000,000)/weight의 정수 나눗셈이 둘 다 0이다. retained probe 재현.

## 원인 / 영향 / 범위

accepted public weight 범위에 대한 precision 보장이 없고 unsupported 값의 silent coercion/zero service tags가 발생한다. zero-tag replenishment는 positive-tag class 앞에 계속 들어갈 수 있다. starvation의 장기 runtime 실험은 수행하지 않았다.

## 보완 계획

유효 weight/precision 범위를 validate해 fail-closed하거나 fractional credit/충분한 fixed-point precision을 보존한다. 모든 increment에 max(1)만 적용하면 다양한 높은 weight가 다시 같아지므로 ratio invariant를 함께 설계한다.

## Acceptance / 회귀 검증

threshold 양측과 weight ratio scaling, 서로 다른 task cost, 지속 replenish에서 proportionality/진행성, stable tie break를 검증한다.

