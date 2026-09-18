# H16-003 — 단일 ticket lifecycle과 terminal claim

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P0 (원본 bug severity 변경 아님)
- 책임 역할: E — Engine owner (실제 assignee 미지정)
- 선행 완료: [H16-002](H16-002-validated-policy-topology.md)
- 원본 finding: [TM16-010](../../../bugbash/sep-16-general/tickets/TM16-010-leak-sweep-leaves-dead-claim.md)
- 배타적 write lease: `contract`, `engine`, `host`; [적용 순서](EXECUTION.md) 준수

## 목적

ticket/permit/root guard를 하나의 terminal transition에서 정리하고 dead claim·영구 await를 제거한다.

## 변경 범위

- 기존: [crates/taskmesh-engine/src/engine/state.rs](../../../../crates/taskmesh-engine/src/engine/state.rs)
- 기존: [crates/taskmesh-engine/src/engine/governor.rs](../../../../crates/taskmesh-engine/src/engine/governor.rs)
- 기존: [crates/taskmesh-engine/src/shared/mod.rs](../../../../crates/taskmesh-engine/src/shared/mod.rs)
- 기존: [crates/taskmesh-engine/src/features/admission/mod.rs](../../../../crates/taskmesh-engine/src/features/admission/mod.rs)
- 기존: [crates/taskmesh-engine/src/features/memory/mod.rs](../../../../crates/taskmesh-engine/src/features/memory/mod.rs)
- 기존: [crates/taskmesh/src/runtime.rs](../../../../crates/taskmesh/src/runtime.rs)
- 제안 경로: `crates/taskmesh-engine/src/engine/lifecycle.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음
- 제안 경로: `crates/taskmesh-engine/tests/hardening_lifecycle.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [x] Pending/DispatchReserved/Accepted/Running/CleanupPending/Terminal과 caller-response status를 분리한 request record를 정의한다. Accepted는 adapter가 인수했지만 아직 Started가 없는 phase다. legacy direct Governor admit은 외부 host가 소유하는 permit임을 명시한다.
- [x] ClaimOutcome(Ready/Pending/Terminal/Invalid)을 도입한다. bare Option compatibility wrapper의 한계를 문서화하고 host await_promotion은 terminal outcome을 반드시 처리한다.
  → bare Option wrapper는 두지 않았다(breaking, `ClaimOutcome`만); host는 네 outcome 모두 처리
- [x] release/abandon/reap/spawn-failure가 공통 invalidate path를 사용하도록 옮긴다. permit index, ticket index, recursion guard, root/class/global delta를 함께 commit한다.
- [x] 취소와 claim의 linearization winner를 정한다. queued→reserved→cancel→late worker start race에서 시작 승인과 refund가 둘 다 성공할 수 없게 한다.
- [x] terminal 상태는 waiter-owned completion cell 또는 bounded retention으로 전달한다. orphaned terminal cell, abandoned handle, ID exhaustion을 포함하고 무한 tombstone map을 금지한다.
  → ID 공간은 u64 monotonic(재사용 없음); exhaustion은 실용적으로 도달 불가로 두었다
- [x] run_local future/closure 자체는 Send kernel로 이동하지 않는다. kernel에는 immutable metadata/handle만 두고 !Send payload는 caller-local owner에 유지한다.
- [x] runtime-owned lease와 직접 Governor/manual permit의 release/reap 권한을 구분한다. 공개 제어 API로 살아 있는 runtime task의 capacity를 조기 환급할 수 없게 ownership capability를 검증한다.
  → 처음엔 token 없이 검출만 했다(post-dispatch double release debug assert). 마무리 검증에서 방지로 바꿈: `DispatchReserved`를 벗어나는 첫 `advance_phase`가 `LeaseToken`을 mint하고(`AdvanceOutcome::Leased`), 이후 `release(id)`는 `HeldByLease { phase }`로 거절되며 `release_leased(token)`만 permit을 끝낸다 — `hardening_lease_token.rs`, `an_ext_release_cannot_free_a_running_jobs_slot` (ADR 0003 D14 개정)

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- 단일 ticket index(`Queued`/`Granted`/`Terminal`)와 `ClaimOutcome`
  (`Ready`/`Pending`/`Terminal`/`Invalid`)를 도입했다. "아직"과 "끝났다"가 더 이상 같은
  값이 아니다.
- `unwind`가 permit과 ticket을 같은 transition에서 무효화하고, 아직 claim되지 않은
  waiter에게 terminal 사유를 전달한다.
- terminal 보존은 `MAX_TERMINAL_TICKETS`로 bounded다. 한도를 넘으면 가장 오래된 기록이
  밀려나고 그 ticket은 `Invalid`로 읽힌다 — 사유만 사라지고 여전히 terminal이다.
- host `await_promotion`은 네 outcome을 모두 처리하며 terminal/invalid를 typed
  `GovernorError`로 반환한다.
- `run_local`의 non-`Send` payload는 caller task에 그대로 남는다.

Regression: `crates/taskmesh-engine/tests/hardening_lifecycle.rs` (7),
`crates/taskmesh/src/runtime/claim_acquisition_tests.rs` (9).

## 검증 / 완료 조건

- [x] `H16-003-A01` promote→reap→claim에서 dead permit 없음; waiter는 terminal로 종료
- [x] `H16-003-A02` claim/abandon/reap/release 순열에서 ownership 1회
- [x] `H16-003-A03` child recursion/root index가 record projection과 일치 — queued child abandon,
      promoted child reclaim 각각 slot·attribution 반환 (`hardening_child_scope.rs`)
- [x] `H16-003-A04` late/duplicate/foreign handle이 다른 실행을 release하지 못함 — `release`는
      `ReleaseOutcome::UnknownPermit`으로 보고(조용한 no-op 아님); 다른 governor의 동일 id 포함
- [x] `H16-003-A05` terminal handle 반복 생성에도 저장 상한 유지
- [x] `H16-003-A06` non-Send `run_local` 경로 유지
- [x] `H16-003-A07` Accepted↔Started barrier: holding adapter로 caller가 Accepted 단계에서 떠나도
      gauge(`accepted=1, running=0`)·credit이 유지되고 adapter가 실행하면 한 번만 refund됨
      (`a_caller_that_leaves_while_the_job_is_accepted_but_not_started_keeps_it_charged`).
      dispatch 전에 sweep이 lease를 회수하면 `advance`가 `LeaseReclaimed`로 작업을 거부한다.

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
cargo test -p taskmesh-engine --test leak_sweep --test permit_lifecycle --test composite_root_attribution --test recursive_rejection
cargo test -p taskmesh --test runtime_cancel_timeout --test runtime_local
```

## 호환성 / 실패 모드

- 군데군데 map.remove 추가로 끝내지 않는다. missing mapping과 pending 상태를 같은 None으로 취급하면 hang이 남는다.
- 세대 ID는 재사용이 있을 때 필요하며 u64 wrap을 자동 허용하지 않는다.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-18.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)
