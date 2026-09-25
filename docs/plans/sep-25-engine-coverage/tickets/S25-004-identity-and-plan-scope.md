# S25-004 — identity·parent plan·Governor handle

- 상태: PLANNED; 우선: P1; 선행: [S25-001](S25-001-contract-boundaries.md); 소유: contract/API owner → engine owner → consumer owner.
- 주 담당 시나리오: B21, B22, B28, H18.
- 근거: `crates/taskmesh-engine/src/shared/mod.rs`의 `PermitId`/`Ticket` u64, `governor.rs`의 direct API, `task.rs`의 child identity, `docs/taskmesh-external-interface.md`의 host/direct 예약 차이.

## 목적

identity를 `(root_operation_id, operation)`의 live 요청, parent generation, Governor instance로 분해한다. B28의 숫자 충돌을 재현한 뒤 governor-bound opaque handle로 이행하는 API 설계를 확정한다. 기존 raw API를 유지하는 기간에는 same-Governor scope를 명시하며 foreign 안전성으로 승격하지 않는다. B22의 cross-plan membership은 planner 책임으로 문서화하고, engine에 registry를 추가하지 않는 선택의 결과를 fixture로 고정한다.

현재 engine은 live `(root, operation)` 인덱스와 queued request를 확인하고 child가 enqueue/grant에서 관찰한 parent permit generation을 고정한다. 하지만 parent의 **선언된 plan stage 집합**을 보관하는 registry는 없다. `PermitId`/`Ticket`은 `shared/mod.rs`의 `u64` 별칭이며 각 Governor counter가 1부터 시작한다. `LeaseToken`은 별도 nonce로 보호된다. 세 identity를 같은 보장으로 합치지 않는다.

## 변경 파일

| 구분 | 경로 | 작업 |
|---|---|---|
| 기존 근거·수정 | `crates/taskmesh-engine/src/shared/mod.rs`, `crates/taskmesh-engine/src/engine/{governor,state}.rs` | raw ID 발급/lookup/전이; 승인된 경우 opaque handle·owner 검사·wrap 실패 처리 |
| 조건부 수정 | `crates/taskmesh-engine/src/features/{admission/pending,composite/mod}.rs`, `crates/taskmesh-engine/src/lib.rs` | handle 타입 전파와 public re-export. parent generation 로직 보존 |
| 조건부 수정 | `crates/taskmesh/src/{runtime,lib}.rs`, `crates/taskmesh-contract/src/verdict.rs` | host guard와 `taskmesh::ext` API, 오류의 숫자 표시만 telemetry로 분리. 실제 변경은 S25-001 semver 후 |
| 기존 fixture | `crates/taskmesh-engine/tests/{hardening_child_scope,hardening_lease_token,recursive_rejection,composite_root_attribution}.rs` | B21/B22의 current behavior 및 lease-token control |
| 신규 fixture 제안 | `crates/taskmesh-engine/tests/cross_governor_ids.rs` | B28 permit/ticket 숫자 충돌과 모든 direct API foreign/local 분기 |
| 기존/신규 consumer fixture | `crates/taskmesh/tests/{hardening_consumer_surface,hardening_dispatch_resolution,runtime_cpu_executor}.rs`, 신규 `crates/taskmesh/tests/public_surface_matrix.rs` | host/direct multi-stage, default/Rayon owned/shared 공개 API 선택. 신규 파일은 필요 시 |
| 문서 | `docs/taskmesh-library-spec.md`, `docs/taskmesh-external-interface.md` | B22 planner 책임, ID scope/migration, 예약 범위 |
| 외부 소유 | product planner/consumer repository | parent plan membership 및 wire alias 실제 통합 receipt |

## 구현 순서

1. **B21 현재 identity.** 같은 `(root_operation_id, operation)`을 class/stage를 바꿔 중복 제출하고, 첫 요청이 `Queued`, `Granted`, `Running`인 경우를 분리한다. 다른 root의 같은 operation, terminal 후 재사용은 정상 control로 둔다. duplicate rejection에서 snapshot/queue/waker/worker side effect 0을 독립 event ledger로 확인한다.
2. **B22 계획 범위.** live parent를 가리키지만 parent가 선언하지 않은 유효 `parent_stage`를 child로 제출한다. 현재 validator가 문자열만 검사하고 engine이 membership을 보장하지 않는 사실을 fixture로 고정한다. S25-001이 권고를 채택하면 **실제 외부 planner**가 parent generation/plan view를 보유하고 child 생성 전에 membership 검사한다. engine은 해당 registry를 암묵적으로 만들지 않는다.
3. **B28 재현 먼저.** 두 Governor를 별도 생성해 local permit/ticket 숫자가 각각 1로 충돌하는 control을 만든다. foreign raw 값을 다른 Governor의 `release`, `advance_phase`, `claim`, `abandon`에 넣었을 때 local 상태에 작용할 수 있는 현행 위험을 API별 별도 fixture로 기록한다. 관측 후에만 목표 API를 설계한다.
4. **B28 목표 API.** governor instance nonce/authority와 local sequence를 private 필드로 가진 opaque permit/ticket handle을 발급한다. 모든 lookup/transition은 소유권을 먼저 확인하고 foreign은 typed reject 또는 기존 outcome의 명시적 foreign variant로 끝내며 state·callback 0 변화. `abandon`의 현재 `()` 및 public `PermitId`/`Ticket` alias 변경은 breaking이므로 S25-001에서 version/유예를 결정한다. display용 숫자와 authority handle을 별도 타입으로 둔다. counter wrap은 재사용 허용이 아니라 typed exhaustion이다. 기존 `LeaseToken`의 nonce와 중복된 권위 체계를 만들지 않게 구조를 검토한다.
5. **H18 소비자 matrix.** `taskmesh` facade만 import하는 fixture에서 raw/validated wire, legacy source alias, direct `Governor::admit`의 모든 선언 stage capability 예약과 host `run_*`의 첫 dispatch 예약을 비교한다. default CPU, Rayon owned, Rayon `with_pool` shared는 실제 feature 선택과 선언을 각각 확인한다. `just test-rayon`은 현재 lib와 단일 host test만 선택하므로 신규 integration target selector를 S25-012에 넘긴다.

## DoD

- [ ] `S25-004-B21` class/stage/queue 상태를 바꿔도 동일 live root-operation 중복은 admission side effect 0으로 거절되고 release/terminal 후 재사용된다.
- [ ] `S25-004-B22` 유효한 문자열이지만 parent plan에 없는 stage가 현재 engine에서는 membership reject가 아님을 관측한다. 외부 planner의 membership 검사 계약 및 통합 owner가 지정된다.
- [ ] `S25-004-B28` 두 Governor가 같은 raw 숫자를 발급했을 때 release/advance/claim/abandon의 실제 local 충돌을 분리해 재현한다. 목표 opaque handle에는 foreign reject와 local 정상 control이 있으며 semver/소비자 migration이 있다.
- [ ] `S25-004-H18` 외부 소비자가 raw/validated wire, old alias, host/direct multi-stage 예약 범위, default/Rayon owned/shared 선언을 관측한다. 내부 helper만 호출하는 fixture는 불합격이다.

handle 타입 변경이 public API breaking이면 S25-001의 결정에 따라 버전을 분리한다. owner-local contract+engine+host → `test-rayon`, consumer MSRV, CI.

권고 migration은 (1) 충돌 재현과 same-Governor 범위 문서화, (2) instance nonce를 private하게 보유하는 새 opaque permit/ticket handle과 foreign typed reject, (3) raw 숫자 API의 deprecation 또는 명시적 breaking 제거 순서다. 숫자 display/telemetry ID와 권한 handle은 별도 타입으로 둔다. lease token의 기존 전역 nonce와 소유권 관계를 검토하고 두 authority를 중복 생성하지 않는다.

| Acceptance | 독립 oracle |
|---|---|
| `S25-004-B21` | live/queued 중복은 exact `RecursiveAdmission`, 두 번째의 total/queued/inflight/waker 변화 0. 다른 root는 통과하고 terminal 이후 동일 key 재사용 가능. |
| `S25-004-B22` | 현재 엔진의 non-membership reject 부재를 현행 behavior로 판정. 목표 외부 planner는 없는 stage를 **governor 호출 전에** 거절하며, planner의 source/owner/fixture를 지정. |
| `S25-004-B28` | 현재 foreign raw 숫자 충돌을 release/advance/claim/abandon별로 재현. 목표 handle은 네 API 모두 foreign state 변화 0 및 typed 결과, local 정상 control; token lease 경로 유지. 단순 다른 instance 상태 독립성만 보는 기존 `hardening_child_scope` 테스트는 충돌 증거가 아님. |
| `S25-004-H18` | host/direct 다중 stage 예약 차이, default/Rayon owned/shared executor 선언, wire alias를 public import 경로에서 관측. Rayon test가 실제 CI selector에 들어감. |

## 계획된 검증

owner-local 후보: `cargo test --locked -p taskmesh-engine --test hardening_child_scope --test hardening_lease_token --test cross_governor_ids`, `cargo test --locked -p taskmesh --test hardening_consumer_surface --test hardening_dispatch_resolution --test public_surface_matrix`, `cargo test --locked -p taskmesh --features rayon --test public_surface_matrix`, `just consumer-msrv`. 신규 target 이름은 **제안**이며 생성 전 명령은 실행 불가하다. `just test-rayon` selector 변경과 clean-source `just verify-macos-ci`는 S25-012가 소유한다. 현재 문서 단계에서는 테스트를 실행하지 않는다.

## 인계·중단 조건

contract/API owner가 B28의 semver 및 handle/outcome 설계를 승인한 뒤 engine owner가 shared/governor/state를 **직렬** 변경하고 host/consumer owner가 public facade를 이행한다. `governor.rs`는 S25-003/008/009와 겹치므로 동시 쓰지 않는다. 외부 planner가 확인되지 않으면 B22의 current-engine fixture만 닫고 planner 통합 DoD는 OPEN이다. 기존 raw API를 계속 제공하는 동안 그 API를 foreign-safe라고 표시하지 않는다. 새로운 handle이 wire 또는 FFI에서 재구성 가능하면 authority 보호가 무력화되므로 설계를 다시 검토한다.
