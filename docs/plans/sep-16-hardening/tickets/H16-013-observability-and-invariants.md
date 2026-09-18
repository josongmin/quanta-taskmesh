# H16-013 — Authoritative snapshot·진단·보존식

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P1 (원본 bug severity 변경 아님)
- 책임 역할: E — Engine owner (실제 assignee 미지정)
- 선행 완료: [H16-010](H16-010-dispatch-integration.md)
- 원본 finding: 통합·계약·품질 작업; 독립 신규 결함 수에 가산하지 않음
- 배타적 write lease: `engine`, `host`, `contract`; [적용 순서](EXECUTION.md) 준수

## 목적

snapshot·진단이 실제 lifecycle phase와 resource owner를 일관되게 보여주게 한다.

## 변경 범위

- 기존: [crates/taskmesh-contract/src/snapshot.rs](../../../../crates/taskmesh-contract/src/snapshot.rs)
- 기존: [crates/taskmesh-engine/src/engine/governor.rs](../../../../crates/taskmesh-engine/src/engine/governor.rs)
- 기존: [crates/taskmesh-engine/src/engine/state.rs](../../../../crates/taskmesh-engine/src/engine/state.rs)
- 기존: [crates/taskmesh/src/runtime.rs](../../../../crates/taskmesh/src/runtime.rs)
- 제안 경로: `crates/taskmesh-engine/tests/hardening_snapshot_projection.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [x] Pending/DispatchReserved/Accepted/Running/CleanupPending/Terminal의 배타적 ownership phase와 별도 ResultHeld retention, class/global/capability usage를 authoritative state의 projection으로 제공한다. Accepted는 adapter 내부 start 대기를 숨기지 않는 gauge다.
- [x] caller completed와 execution terminated를 별도 counter로 둔다. deadline response가 live-worker gauge를 0으로 만들지 않는다.
- [x] admitted_total=sum(live ownership phases)+terminal_total, started_total=running+started_cleanup+terminated_after_start처럼 같은 scope의 보존식을 정의한다. adapter Accepted gauge와 누적 admission count를 구분하고 never-started terminal을 started 식에 넣지 않는다. ResultHeld는 execution phase와 직교하므로 live execution에 중복합산하지 않는다. retired terminal record도 누적 counter에는 보존하고 reserved/active units 중복합산을 금지한다.
- [x] queue wait, dispatch delay, run duration, cleanup duration, lock hold/selection visits를 구분한다. timeout phase·resolved class·adapter failure code를 safe context로 노출한다.
  → queue wait/block 원인은 `pending_view`/`pending_block_reason`, selection visits는 `drr_ring_visits`(test-util), typed error에 phase/class context; lock hold time metric은 두지 않음
- [x] high-cardinality operation/root ID는 metric label로 사용하지 않는다. bounded diagnostic ring 또는 sampling을 사용하며 diagnostics drop을 별도 count로 표시한다.
  → high-cardinality label 없음; diagnostics ring 없음(N/A, EXCEPTIONS H16-013-A03)
- [x] legacy snapshot 의미 변경은 D06 버전 계약을 따른다. snapshot/read callback은 사용자 코드를 lock 안에서 실행하지 않는다.

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- `ExecutionPhase`(DispatchReserved/Accepted/Running/CleanupPending)를 도입했다. phase는
  monotonic하고 `inflight`를 정확히 분할한다. caller의 응답은 phase를 바꾸지 않는다.
- 누적 counter를 분리했다: `admitted_total`, `started_total`, `terminated_total`.
  보존식 `admitted_total == inflight + terminated_total`과 phase 분할을
  `ClassSnapshot::conservation_violation()` / `Snapshot::conservation_violation()`가
  검사한다.
- capability 점유를 `(in_use, limit)`으로 노출한다.
- `pending_view` / `pending_block_reason`이 queue wait와 blocking 원인을 보고하므로
  timeout이 "class quota"와 "worker pool"을 구별해 말한다.
- `permit_ledgers()`가 독립 oracle용 원자료를 제공한다.
- high-cardinality label은 도입하지 않았다. metric은 class·pool 단위이며 root/operation
  ID를 label로 쓰지 않는다.
- `diagnostics_dropped` 필드는 **제거**했다. 버퍼링하는 diagnostics가 없어 항상 0인
  필드는 계약이 아니라 죽은 wire surface다.

Regression: `crates/taskmesh-engine/tests/hardening_snapshot_projection.rs` (6).

## 검증 / 완료 조건

- [x] `H16-013-A01` 각 transition 후 snapshot과 독립 record projection 동일
- [x] `H16-013-A02` hidden outstanding count 없음 (Accepted가 별도 gauge)
- [ ] `H16-013-A03` diagnostics ring을 두지 않았으므로 해당 없음 — 넘칠 버퍼가 없다.
  → 예외 대장: [EXCEPTIONS.md](EXCEPTIONS.md)
- [x] `H16-013-A04` 장기 운용 후 metric cardinality 및 terminal storage bounded
- [x] `H16-013-A05` wide resource serialization 왕복 무손실
- [x] `H16-013-A06` Accepted→Started 사이 cancel/shutdown에서 gauge·credit 유지
      (`hardening_executor_protocol.rs::a_caller_that_leaves_while_the_job_is_accepted_but_not_started_keeps_it_charged`)

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
cargo test -p taskmesh-engine --test composite_root_attribution --test prop_invariants
cargo test -p taskmesh --test e2e_proof
```

## 호환성 / 실패 모드

- telemetry는 admission의 두 번째 원장이 아니다.
- 진단 메시지 drop과 필수 completion event drop을 동일하게 취급하지 않는다.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-18.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)
