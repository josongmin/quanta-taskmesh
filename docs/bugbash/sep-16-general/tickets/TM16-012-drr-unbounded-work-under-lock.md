# TM16-012 — DRR 선택의 작업량이 cost/quantum에 비례해 mutex를 장시간 점유한다

- Severity: P2
- Status: OPEN
- Lane: fairness
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

crates/taskmesh-engine/src/features/fairness/scheduler.rs:155-197; crates/taskmesh-engine/src/engine/governor.rs:150-155,167-188

## Trigger / 관찰

한 runnable class, deficit=0, quantum=1, head CPU cost=K를 promote한다.

loop는 한 방문당 deficit를 1 늘려 K번 방문해야 한다. public policy는 K=u32::MAX를 허용하므로 4,294,967,295회 방문이 가능하다. 작은 K=1000의 source execution probe는 subagent가 수행했다. 최대값의 wall-clock 실행은 수행하지 않았다.

## 원인 / 영향 / 범위

finite loop라는 주석이 operational bound를 보장하지 않는다. promotion은 global governor mutex 안에서 실행하므로 그동안 admission/release/claim/snapshot이 모두 대기할 수 있다. 극단 configuration-dependent scalability defect다.

## 보완 계획

비어 있는 round를 산술적으로 건너뛰어 deficit를 일괄 credit하고 ring order/proportional service를 유지한다. max visits per dispatch를 class 수에 대해 bound한다. 단순 quantum normalization은 이 문제를 해결하지 못한다.

## Acceptance / 회귀 검증

작은 state의 기존 알고리즘 oracle과 결정적 선택 순서 비교, 여러 클래스/cost/quantum/cursor/drop 조합, MAX boundary에서 operation-count 상한을 검증한다. 느린 giant loop를 CI에서 실제로 돌리지 않는다.

