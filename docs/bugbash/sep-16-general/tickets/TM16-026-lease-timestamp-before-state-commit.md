# TM16-026 — Lease timestamp가 state commit보다 앞서 조기 stale 회수를 만든다

- Severity: P2
- Status: OPEN / controlled-interleaving reproduced
- Lane: E — engine ledger/lifecycle
- Baseline: `9ae9547216c70458f54f37368a67661321060886`, 2026-09-16

## 근거

- `crates/taskmesh-engine/src/engine/governor.rs:109-112`: clock을 읽은 뒤 IDs 생성과 state lock을 수행한다.
- `crates/taskmesh-engine/src/engine/state.rs:222-228`: grant 시 전달된 과거 값을 leased_at/last_touched에 그대로 기록한다.
- `crates/taskmesh-engine/src/engine/governor.rs:201-215`, `features/memory/mod.rs:60`: reconcile도 lock 이전 timestamp로 last_touched를 덮는다.
- `crates/taskmesh-engine/src/features/memory/mod.rs:31-32`: stale 판정은 기록된 timestamp를 신뢰한다.
- [Retained observation](evidence/repro/src/lib.rs): `admission_timestamp_precedes_actual_lease_commit`.

## Trigger / 관찰

1. A가 clock=0을 캡처한 직후, governor state mutation 전에 멈춘다.
2. B가 clock=1000으로 admission을 완료한다.
3. A를 재개하면 A의 lease가 B보다 나중에 생성되지만 timestamp는 0이다.
4. 곧바로 clock=1000에서 stale window=100으로 sweep하면 방금 생성된 A만 회수한다. A reconcile은 false, B reconcile은 true다.

Injected Clock 내부 barrier로 clock-read 이후 preemption을 모사했다. 실제 SystemClock/OS preemption의 빈도나 wall-clock stall은 측정하지 않았다. clock 값 자체가 뒤로 점프하는 NTP 문제와는 별개다.

같은 probe에서 B의 reconcile이 time=1000을 읽고 지연되는 동안 다른 reconcile이 time=2000으로 성공하도록 했다. 늦은 reconcile이 뒤늦게 commit되면 B도 immediately stale로 회수된다. provider의 실제 clock 값은 0→1000→2000으로만 증가한다.

## 원인 / 영향 / 범위

샘플 시점과 lease 생성/갱신의 linearization 시점이 다르다. 지연된 admission은 시작부터 오래된 lease를 만들며, 지연된 reconcile은 더 최신 heartbeat를 과거로 돌릴 수 있다. LeakDetecting opt-in에서 정상 작업의 조기 resource reclaim으로 이어진다. TM16-009의 stage touch 누락, TM16-010의 dead-ticket mapping과 독립된 원인이다.

## 보완 계획

- admission/heartbeat timestamp authority와 state commit 시점의 관계를 명시한다.
- 늦게 도착한 timestamp가 최신 lease activity를 후퇴시키지 않도록 monotonic watermark/refresh 정책을 설계한다. 신규 lease의 lock-wait 시간도 stale age에 잘못 포함하지 않아야 한다.
- 단순히 driven Clock 호출을 mutex 안으로 옮기면 blocking/re-entrant port가 deadlock을 만들 수 있다. provider 계약과 lock discipline을 함께 고정한다.

## Acceptance / 회귀 검증

- 위 순서의 corrected test에서 A/B 모두 immediately stale로 회수되지 않는다.
- 두 reconcile의 read/commit 순서를 뒤집어도 activity가 후퇴하지 않는다.
- 실제 lock contention, monotonic clock, clock jump를 각각 구분하여 테스트한다. stale-window 경계와 existing release/promote custody도 보존한다.
