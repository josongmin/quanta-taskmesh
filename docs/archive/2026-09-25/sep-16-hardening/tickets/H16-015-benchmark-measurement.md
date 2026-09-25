# H16-015 — Benchmark 실제 작업량·측정 경계·분모

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P1 (원본 bug severity 변경 아님)
- 책임 역할: B — Benchmark owner (실제 assignee 미지정)
- 선행 완료: 없음
- 원본 finding: [TM16-018](../../../../bugbash/sep-16-general/tickets/TM16-018-benchmarks-skip-intended-work.md), [TM16-034](../../../../bugbash/sep-16-general/tickets/TM16-034-iai-input-teardown-in-measurement.md), [TM16-036](../../../../bugbash/sep-16-general/tickets/TM16-036-contention-bench-iteration-denominator.md)
- 배타적 write lease: `bench`; [적용 순서](EXECUTION.md) 준수

## 목적

setup/op/teardown과 실제 성공 작업 수를 명시해 오측정·no-op regression green을 막는다.

## 변경 범위

- 기존: [crates/taskmesh-bench/examples/alloc_probe.rs](../../../../../crates/taskmesh-bench/examples/alloc_probe.rs)
- 기존: [crates/taskmesh-bench/benches/iai_governance.rs](../../../../../crates/taskmesh-bench/benches/iai_governance.rs)
- 기존: [crates/taskmesh-bench/benches/composite_reduce.rs](../../../../../crates/taskmesh-bench/benches/composite_reduce.rs)
- 기존: [crates/taskmesh-bench/benches/contention.rs](../../../../../crates/taskmesh-bench/benches/contention.rs)
- 기존: [crates/taskmesh-bench/src/loadgen.rs](../../../../../crates/taskmesh-bench/src/loadgen.rs)
- 기존: [crates/taskmesh-bench/benches/host_edge_paths.rs](../../../../../crates/taskmesh-bench/benches/host_edge_paths.rs)
- 기존: [crates/taskmesh-bench/benches/governance_tax.rs](../../../../../crates/taskmesh-bench/benches/governance_tax.rs)
- 제안 경로: `crates/taskmesh-bench/tests/measurement_contract.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [x] 각 bench에 measurement definition/version, expected verdict, expected attempted/completed counts, post-ledger 조건을 선언한다.
- [x] 측정 branch 자체에서 unexpected Reject/Queue/TaskError를 실패시킨다. preflight만 정상이고 timed loop가 다른 branch를 타지 않도록 한다.
- [x] IAI input ownership을 함수 밖으로 반환하거나 명시적 entry point로 op-only 경계를 만든다. snapshot output drop과 input governor drop을 별도 단위로 다룬다.
- [x] Criterion iters를 quotient/remainder로 정확히 나누거나 requested-iters 기준 duration 보정식을 고정한다. iters<workers·nonmultiple·integer overflow를 포함한다.
- [x] holder fixture는 ready signal로 동기화한다. worker-key interning/setup/thread creation 포함 여부와 op fixture의 실제 outstanding state를 명시한다.
- [x] 측정 정의가 변경되면 measurement schema version을 올리고 H16-017의 이전 baseline compatibility를 끊는다. mem::forget로 fixture를 leak시키지 않는다.

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- `alloc_probe`: non-Admitted 즉시 exit 2, `completed == attempted`, 최종 inflight 0 검증, 구조화된
  한 줄 출력(`taskmesh-alloc-probe schema=1 ...`). 측정 정의 version = 1.
- `iai_governance`: 벤치 함수가 input을 **반환**하고 `teardown` hook이 파괴 → governor teardown이
  측정 구간 밖(TM16-034). 각 op의 verdict를 assert(TM16-018). `MEASUREMENT_SCHEMA = 2`로 이전
  baseline과 호환 단절.
- `composite_reduce`: non-Admitted panic, reduce validation 결과 assert.
- `contention`: `contention_run(t, iters)`가 iters를 quotient+remainder로 정확히 분배하고
  `completed_ops == iters`를 단언; worker는 barrier 뒤에서 시작해 spawn latency를 측정 밖으로(TM16-036).
- `tools/bench/perf-gate.json`: threshold·schema·runner version·fingerprint input 단일 source.
- 이 과정에서 allocation gate가 **내가 도입한 회귀(3→5 allocs/op)** 를 잡았고 capability name
  interning으로 3.0으로 복귀시켰다. threshold를 올리지 않았다.

Regression: `crates/taskmesh-bench/src/loadgen.rs` tests (`ops_are_distributed_exactly`,
`a_contention_run_completes_exactly_the_requested_count`), `tools/bench/tests/test_allocation_gate.py`,
mutation `contention-ops-rounded-to-thread-multiple`.

## 검증 / 완료 조건

- [x] `H16-015-A01` forced Reject에서 real producer(`TASKMESH_ALLOC_PROBE_FORCE=reject`) exit 2 →
      gate가 그 코드를 전파; attempted==completed; probe는 `counter_check` self-test를 보고한다
- [x] `H16-015-A02` IAI input drop이 teardown hook; snapshot output drop 포함 여부 명시
- [x] `H16-015-A03` iters 1,t-1,t,t+1 × t=1/2/4/8 정확 분배
- [x] `H16-015-A04` contention holder는 barrier ready 신호로 동기화
- [ ] `H16-015-A05` Linux instruction-count proof는 macOS에서 미실행 → 별도 receipt 없음 (H16-022 BLOCKED)
  → 예외 대장: [EXCEPTIONS.md](EXCEPTIONS.md)

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
cargo test -p taskmesh-bench
cargo bench -p taskmesh-bench -- --test
bash tools/bench-gate.sh
```

## 호환성 / 실패 모드

- op-only와 full roundtrip은 서로 다른 성능 지표다. 좋아 보이는 수치를 위해 baseline 의미를 바꾸지 않는다.
- 계측 instrumentation overhead를 actual production hot-path 비용으로 섞어 설명하지 않는다.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-19.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)

