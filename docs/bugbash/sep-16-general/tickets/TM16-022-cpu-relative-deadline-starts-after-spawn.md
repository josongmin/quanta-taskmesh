# TM16-022 — CpuExecutor::spawn 완료 뒤 상대 deadline을 시작해 늦은 결과를 성공 처리한다

- Severity: P2
- Status: OPEN
- Lane: runtime-topology
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)
- Evidence: evidence/repro/src/lib.rs

## 근거

crates/taskmesh/src/runtime.rs:593-630,769-780; crates/taskmesh-contract/src/ports.rs:56-64

## Trigger / 관찰

CooperativeWithDeadline + RunFor=5ms이고 custom CpuExecutor::spawn이 inline으로 60ms job을 실행한 뒤 반환한다.

job이 50ms 이상 실행되어도 Ok(7)을 반환한다. retained probe가 재현한다. spawn 지연/inline delivery에 대한 nonblocking/liveness contract는 public trait에서 enforce되지 않는다.

## 원인 / 영향 / 범위

relative timer는 job submission 이전에 고정되지 않고 cpu.spawn 이후 run_cancellable에서 만들어진다. blocking spawn 자체를 async timer로 선점할 수는 없지만 완료시각이 budget을 넘은 결과를 성공으로 판정하는 것은 별도 문제다. 일반 Rayon의 짧은 spawn에서 이 만큼의 latency가 관찰됐다는 주장은 아니다.

## 보완 계획

relative authority를 worker 실행/제출 중 어느 시점에 시작할지 고정하고 Instant를 submission 전에 보존한다. completion timestamp로 deadline 초과를 판정한다. CpuExecutor의 nonblocking submission/worker-start 신호/오류 계약을 명시한다.

## Acceptance / 회귀 검증

inline/느린 spawn/정상 queued executor, 이미 완료한 결과, overflow, spawn panic과 worker lease 회수를 검증한다. caller deadline 응답 지연과 late-success 판정을 구분한다.

