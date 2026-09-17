# TM16-009 — Stage memory activity가 leak lease를 갱신하지 않아 최근 작업을 stale로 회수한다

- Severity: P2
- Status: OPEN
- Lane: engine-memory
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)
- Evidence: evidence/repro/src/lib.rs (현재 잘못된 동작을 assert하는 observation probe)

## 근거

crates/taskmesh-engine/src/features/memory/mod.rs:54-58,94-121,135-145; crates/taskmesh-engine/src/engine/governor.rs:231-237

## Trigger / 관찰

clock=0에서 LeakDetecting permit을 admit하고 59,999ms에 stage memory 1 unit을 반환한다. 60,001ms에 stale window=60,000ms로 sweep한다.

2ms 전에 stage activity가 있었지만 reclaimed_permits=1이다. reconcile은 last_touched_ms를 갱신하고 stage release는 갱신하지 않는다. Governor는 now를 계산하지만 release 함수에는 넘기지 않는다.

## 원인 / 영향 / 범위

stale 판정은 'untouched past window'인데 중요한 permit activity 한 종류가 touch에서 제외된다. 실제 worker 종료를 관찰해 stale를 판단하는 기능은 아니므로 opt-in sweep이 live work의 semantic capacity를 조기 회수할 위험도 있다. 모든 live task가 stale window보다 길면 결함이라는 광범위 주장은 하지 않는다.

## 보완 계획

stage release에 now를 전달하고 성공한 stage activity에서 ledger.last_touched_ms를 갱신한다. trusted host용 heartbeat/touch와 leak reclaim의 안전 전제를 명시한다. runtime-owned worker permit을 강제 회수하려면 worker custody와 별도 협약이 필요하다.

## Acceptance / 회귀 검증

stage activity 직후 sweep no-reclaim, window 경계 이후 reclaim, unknown permit/zero release의 touch 의미, reconcile/heartbeat/worker 종료 matrix를 검증한다.

