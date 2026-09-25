# H16-005 — Memory reservation·measurement epoch·lease clock

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P1 (원본 bug severity 변경 아님)
- 책임 역할: E — Engine owner (실제 assignee 미지정)
- 선행 완료: [H16-004](H16-004-exact-resource-accounting.md)
- 원본 finding: [TM16-005](../../../../bugbash/sep-16-general/tickets/TM16-005-memory-release-policy-not-enforced.md), [TM16-009](../../../../bugbash/sep-16-general/tickets/TM16-009-stage-activity-does-not-touch-leak-lease.md), [TM16-011](../../../../bugbash/sep-16-general/tickets/TM16-011-estimated-reconcile-undoes-stage-release.md), [TM16-026](../../../../bugbash/sep-16-general/tickets/TM16-026-lease-timestamp-before-state-commit.md)
- 배타적 write lease: `contract`, `engine`; [적용 순서](EXECUTION.md) 준수

## 목적

reservation 반환·measurement·heartbeat·reap가 합성될 때 자원이 부활하거나 실행 중 용량이 조기 반환되지 않게 한다.

## 변경 범위

- 기존: [crates/taskmesh-engine/src/features/memory/mod.rs](../../../../../crates/taskmesh-engine/src/features/memory/mod.rs)
- 기존: [crates/taskmesh-engine/src/engine/governor.rs](../../../../../crates/taskmesh-engine/src/engine/governor.rs)
- 기존: [crates/taskmesh-engine/src/engine/state.rs](../../../../../crates/taskmesh-engine/src/engine/state.rs)
- 기존: [crates/taskmesh-contract/src/ports.rs](../../../../../crates/taskmesh-contract/src/ports.rs)
- 기존: [crates/taskmesh-contract/src/policy.rs](../../../../../crates/taskmesh-contract/src/policy.rs)
- 제안 경로: `crates/taskmesh-engine/tests/hardening_memory_epochs.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [x] original_estimate/remaining_reservation/current_measurement/measurement_epoch를 분리한다. Estimated reconcile은 remaining reservation을 되살리지 않는다.
- [x] Measured reading은 현재 총 사용량으로 정의한다. stage reservation 반환을 실제 메모리 감소로 이중 차감하지 않는다. Hybrid floor는 D02의 remaining-reservation 계약을 따른다.
- [x] measurement/heartbeat epoch와 sequence를 검증해 stale update를 명시적으로 거부한다. accepted activity만 commit-time monotonic touch에 반영한다.
- [x] production clock은 내부 trusted monotonic read를 lock 획득 후 사용한다. arbitrary Clock callback을 lock 아래 호출하는 대안은 금지한다. custom Clock 호환은 D07 adapter로 격리한다.
  → lock 안 trusted read 대신 lock 밖 sampling + commit-time monotonic watermark(D07)
- [x] strict runtime-owned running lease는 stale 시 Suspected/StopRequested로 두고 실제 termination 이전 reclaim 금지. unstarted reservation의 terminal cancellation 및 legacy direct-governor force-reclaim은 구분한다.
- [x] stage release 정책/override를 typed API로 정리한다. TM16-005는 D02 채택 후 enforcement 또는 trusted-override 문서화 중 선택한 계약을 구현한다.
- [x] stage release event에는 단일 소유권 또는 idempotency key/sequence 계약을 둔다. 중복·역순 delta와 오래된 measurement epoch를 accounting에 두 번 적용하지 않는다.

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- ledger가 `original_estimate_units` / `remaining_reservation_units` /
  `effective_units`를 분리한다. `Estimated`는 *remaining*을 과금하므로 reconcile이
  반환된 예약을 되살리지 않는다.
- measurement epoch와 stage sequence를 도입했다. stale/중복 delta는 적용되지 않고
  typed outcome으로 보고된다.
- stage release가 lease를 touch한다(0 unit release 포함). `MemoryReleasePolicy`는
  강제 계약이며 `OnTaskCompletion`은 `PolicyForbids`를 반환한다(D02).
- `Clock`은 lock **밖에서** 샘플링하고 transition 안에서 monotonic watermark로 clamp한다.
  늦게 도착한 샘플이 이미 commit된 시각보다 과거를 기록할 수 없다.
- sweep는 executor에 도달하지 않은 permit만 회수한다. 실행 중인 lease는
  `retained_active`로 보고하고 charged 상태로 둔다 — stale은 dead가 아니다.

Regression: `crates/taskmesh-engine/tests/hardening_memory_epochs.rs` (10).

## 검증 / 완료 조건

- [x] `H16-005-A01` 8→release6→reconcile이 2 유지; 반복도 동일
- [x] `H16-005-A02` Estimated/Measured/Hybrid × stage release × epoch matrix
- [x] `H16-005-A03` clock 지연·heartbeat 역순에서 조기 stale/시간 후퇴 없음
- [x] `H16-005-A04` zero/unknown stage release의 touch 정책 명시·시험
- [x] `H16-005-A05` runtime-owned live worker는 sweep 후에도 charged
- [x] `H16-005-A06` promoted-unclaimed sweep가 terminal notification 유지

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
cargo test -p taskmesh-engine --test memory_modes --test memory_overcommit --test leak_sweep
cargo test -p taskmesh --test runtime_cancel_leak --test e2e_chaos
```

## 호환성 / 실패 모드

- 진짜 시간 기반 TTL과 가상 DeadlineAware 시간은 epoch가 다르면 직접 비교하지 않는다.
- stale≠dead는 기존 opt-in force-reclaim 의미 변경이므로 legacy migration을 생략하지 않는다.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-19.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)

