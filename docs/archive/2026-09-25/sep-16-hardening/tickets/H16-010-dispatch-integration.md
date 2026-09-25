# H16-010 — 단일 dispatch authority 통합

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P0 (원본 bug severity 변경 아님)
- 책임 역할: I — 통합/계약 owner (실제 assignee 미지정)
- 선행 완료: [H16-007](H16-007-fairness-reference-and-credit.md), [H16-009](H16-009-bounded-intake.md), [H16-012](H16-012-deadlines-and-cleanup.md)
- 원본 finding: 통합·계약·품질 작업; 독립 신규 결함 수에 가산하지 않음
- 배타적 write lease: `engine`, `host`, `contract`, `rayon`; [적용 순서](EXECUTION.md) 준수

## 목적

semantic resource와 adapter dispatch credit을 동일 reservation 결정에 연결하고 경쟁 arbitration을 제거한다.

## 변경 범위

- 기존: [crates/taskmesh/src/runtime.rs](../../../../../crates/taskmesh/src/runtime.rs)
- 기존: [crates/taskmesh-engine/src/engine/governor.rs](../../../../../crates/taskmesh-engine/src/engine/governor.rs)
- 기존: [crates/taskmesh-engine/src/engine/state.rs](../../../../../crates/taskmesh-engine/src/engine/state.rs)
- 기존: [crates/taskmesh-engine/src/features/fairness/scheduler.rs](../../../../../crates/taskmesh-engine/src/features/fairness/scheduler.rs)
- 기존: [crates/taskmesh-engine/src/features/admission/mod.rs](../../../../../crates/taskmesh-engine/src/features/admission/mod.rs)
- 제안 경로: `crates/taskmesh/src/dispatch.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음
- 제안 경로: `crates/taskmesh/tests/hardening_dispatch_interleavings.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [x] H16-008 frozen capability requirement를 pending record에 보존한다. semantic capacity와 adapter credit을 모두 만족하는 class만 scheduler 후보로 만든다.
- [x] 같은 class의 mixed-substrate head-of-line 정책은 기존 strict FIFO를 기본 보존한다. 우회가 필요하면 per-class subqueue로 몰래 바꾸지 말고 D08 결정을 수정한다.
- [x] fairness selection→reservation→accept/start의 debit 시점을 D08에 따라 commit한다. reserve failure, before-start cancel에서 credit refund/sequence를 reference에 반영한다.
- [x] execution effect가 lock 밖에서 지연될 때 request generation/attempt identity와 start authorization을 검증한다. cancel 후 stale dispatch effect는 실행되지 않는다.
  → generation counter 대신 lease 자체가 authorization: 회수된 lease는 `LeaseReclaimed`로 시작 거부(D14) + loom claim-vs-reap 모델
- [x] host FIFO semaphore waiter와 engine scheduler의 경쟁 실행 순서를 제거한다. adapter가 공유하는 capacity authority 한 곳에서 credit을 가져오며 logical mirrors를 별도 가산하지 않는다.
- [x] driver budget 소진 후 continuation·new capacity notification을 coalesce하고 모든 runnable work의 재평가를 보장한다.
- [x] 선언된 nested wait의 same-capacity/cross-pool cycle 정책을 시험한다. opaque closure가 만들 수 있는 모든 wait graph를 추론한다고 약속하지 않는다.
  → 처음엔 D12 범위 밖으로만 문서화했다가 마무리 검증에서 구현: `awaited_child_of`로 선언된 child가 자기 root의 permit만이 전부 쥔 capacity(class inflight·pool·cpu·memory)를 기다리게 되면 `NestedWaitCycle { held_by_root }`로 거절 (`hardening_nested_wait.rs` engine 9 / host 2); 미선언·stranger·sibling·degrade는 cycle이 아님 — 추론 없음. root 간 cross-pool cycle은 여전히 범위 밖 (ADR 0003 D12 개정)
- [x] 한 runtime instance에서 old/new scheduler를 동시에 실행하지 않는다. shadow는 metadata decision replay만 하고 job을 재실행하지 않는다.
  → shadow scheduler 자체를 두지 않았다(N/A)

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).** 이 티켓의 통합은 H16-008/009와 분리 가능한 단계가
아니었으므로 같은 변경으로 수행했다.

- semantic capacity와 capability 점유가 **하나의** 권위다. host에는 counter가 없고
  engine에도 두 번째 원장이 없다.
- `fairness::select`의 후보는 semantic + physical 양쪽을 만족해야 한다. 물리적으로
  실행 불가능한 클래스를 골라 그 뒤에 두 번째 scheduling queue를 만들지 않는다.
- class 내부 strict FIFO를 보존했다. mixed-substrate HOL은 D08 결정대로 유지한다.
- host FIFO semaphore를 제거했으므로 도착 순서가 class fairness를 덮어쓰는 경로가 없다.
- promotion budget 소진 시 continuation이 보장된다(H16-006).

Regression: `hardening_intake_bounds.rs`의
`class_fairness_is_not_pre_empted_by_arrival_at_a_worker_gate`,
`a_class_blocked_on_its_own_quota_does_not_idle_a_shared_worker`,
`hardening_dispatch_resolution.rs`, `hardening_effect_retirement.rs`.

## 검증 / 완료 조건

- [x] `H16-010-A01` CPU full + IO free에서 eligible IO 진행; idle hoarding 제거
- [x] `H16-010-A02` class fairness가 external FIFO에 의해 역전되지 않음
- [x] `H16-010-A03` reserve 실패/선택 후 cancel이 phantom debt를 남기지 않음
- [x] `H16-010-A04` stale dispatch는 typed로 거절된다: sweep이 `DispatchReserved` lease를 회수한 뒤
  host의 `ExecutionLease::advance`는 `LeaseReclaimed`로 실패하고 작업을 시작하지 않는다
  (`a_lease_the_sweep_reclaimed_before_dispatch_refuses_to_advance_v1`, D14); claim vs reap 경주는
  production `Governor` 위의 loom 모델이 전수 interleaving으로 닫는다
  (`claim_and_reap_of_a_stale_promotion_fail_closed_both_ways`, D13). 별도 generation counter는
  두지 않았다 — lease 자체가 authorization이다.
- [x] `H16-010-A05` 공유 adapter에서 각 runtime이 자기 제출을 제한; 선언된 한계는 builder가 gate와
      대조한다 (D05 개정)
- [x] `H16-010-A06` 선언된 nested wait cycle의 typed reject: `awaited_child_of`로 선언된 child가
      자기 root만이 쥔 capacity에 막히면 queue 대신 `AdmissionVerdict::NestedWaitCycle { held_by_root }`
      (`an_awaited_child_blocked_only_by_its_own_root_is_refused_not_queued`,
      `a_parent_that_awaits_a_declared_child_on_its_own_slot_is_told_so_at_once`); 미선언 wait는 추론하지
      않는다 (`an_undeclared_child_queues_as_any_request_does`) — ADR 0003 D12 개정
- [x] `H16-010-A07` promotion K+1 backlog가 후속 event 없이 drain

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
cargo test -p taskmesh --test substrate_enforcement --test host_inferno --test e2e_chaos
cargo test -p taskmesh-engine --test fairness_fifo --test fairness_drr_deadline --test fairness_weighted
```

## 호환성 / 실패 모드

- engine과 host 둘 다 authoritative counter가 되는 이중원장 금지.
- global budget/fairness는 pool별 독립 shard로 바로 나눌 수 없다. sharding은 H16-022 측정 이후 별도 결정.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-19.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)

