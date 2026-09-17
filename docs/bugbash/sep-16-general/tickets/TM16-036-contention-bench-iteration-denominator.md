# TM16-036 — Contention benchmark의 실제 op 수와 Criterion iteration 분모가 다르다

- Severity: P3
- Status: OPEN / source-guarded integer diagnostic
- Lane: B — wall-clock measurement normalization
- Baseline: `9ae9547216c70458f54f37368a67661321060886`, 2026-09-16

## 근거

- `crates/taskmesh-bench/benches/contention.rs:19-24`: `ops_per_thread=(iters/t).max(1)`로 작업 수를 반올림하고 actual batch duration을 그대로 반환한다.
- `crates/taskmesh-bench/src/loadgen.rs:267-285`: 실제 수행 횟수는 `threads * ops_per_thread`다.
- Locked Criterion 0.5.1 primary `src/analysis/mod.rs:124-127`: 평균 시간은 `elapsed / iters`다.
- [Retained diagnostic](evidence/repro-bench/src/lib.rs): `contention_batch_count_differs_from_criterion_iterations`. producer expression을 source guard로 확인하는 산술 검증이며 actual Criterion timing은 아니다.

## Trigger / 관찰

- t=8, requested iters=1 → 실제 8 ops, duration은 8 ops 전체인데 분모는 1. per-op time이 8배로 정규화된다.
- t=8, requested iters=9 → 실제 8 ops, 분모는 9. per-op time은 실제의 8/9로 계산된다.
- t=8, requested iters=8 → 작업 수와 분모가 일치한다.

## 원인 / 영향 / 범위

integer division/minimum batch 때문에 `iter_custom`의 요청 횟수와 실제 작업 수가 다르다. 큰 batch에서는 오차가 작을 수 있으므로 전체 contention curve가 8배 잘못됐다고 단정하지 않는다. USL probe가 `contention_throughput`을 직접 사용하는 경로에는 이 Criterion 분모 오류가 없다. 현재 trend gate도 해당 contention bench를 입력으로 쓰지 않는다.

## 보완 계획

- 총 iters를 worker별 quotient/remainder로 정확하게 나누거나 반환 시간을 requested iteration 수 기준으로 보정한다.
- iters<t일 때 zero-op worker를 허용할지 batch metric으로 바꿀지 명시한다. integer overflow/cast도 별도로 검사한다.
- thread startup/key preparation 포함 여부를 구분하여 timing 단위를 문서화한다.

## Acceptance / 회귀 검증

- t=1/2/4/8에서 iters=1,t-1,t,t+1 및 nonmultiple batch의 작업 수/분모를 검증한다.
- 큰 batch의 normalized cost가 actual completed count 기준과 일치한다.
- 실제 wall-clock 평가에는 scheduling/thread startup 오차를 별도로 보고한다. 산술 probe만으로 scalability improvement를 주장하지 않는다.
