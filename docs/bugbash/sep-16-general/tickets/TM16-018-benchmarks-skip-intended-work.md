# TM16-018 — Allocation/IAI benchmark가 non-admission을 성공 비용처럼 측정한다

- Severity: P2
- Status: OPEN
- Lane: verification-benchmark
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

crates/taskmesh-bench/examples/alloc_probe.rs:48-65; crates/taskmesh-bench/benches/iai_governance.rs:76-80; crates/taskmesh-bench/benches/composite_reduce.rs:40-42; tools/bench-gate.sh:17-31

## Trigger / 관찰

예상 admit→release benchmark의 fixture/implementation regression이 Queued/Rejected로 바뀐다.

if let Admitted만 실행하며 나머지는 무시한다. allocation denominator는 attempted n=200,000이어서 거부/누락된 roundtrip가 더 낮은 비용으로 계산될 수 있다. source control-flow 증거; forced-mutated benchmark full gate 실행은 미실행이다.

## 원인 / 영향 / 범위

performance metric이 동일한 성공 작업량을 전제하지만 success counter/typed expected decision/accounting 후조건이 없다. reduce_validation의 is_ok 측정도 정상 spec을 assert하는 behavioral 전제와 분리할 필요가 있다.

## 보완 계획

각 측정 op가 예상 verdict와 완료 cycle을 만족해야 한다. 실패하면 즉시 bench를 실패시키고 completed count==attempted count 및 final held/inflight==0을 검증한다. behavioral preflight를 측정 범위 밖에 두되 실제 measured branches도 같도록 고정한다.

## Acceptance / 회귀 검증

forced Reject/Queue fixture가 gate를 실패시키는 negative test, 정상 admitted cycle 수, final accounting 및 allocation metric 분모 검증을 추가한다.

