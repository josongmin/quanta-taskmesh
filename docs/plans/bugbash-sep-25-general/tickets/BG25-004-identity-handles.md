# BG25-004 — identity·parent plan·owner-bound handle

- 구현 상태: IMPLEMENTED
- 증명 상태: STATIC_MAPPED
- 외부 상태: NOT_APPLICABLE — qualification pending
- 우선순위: P1
- 선행: BG25-001 D4–D5
- 소유: contract/API owner → engine owner → consumer owner

## 목적

동일 root-operation 재사용, parent-stage membership 책임, cross-Governor raw ID 충돌을 명시하고 최종적으로 authority-bound handles를 제공한다.

## 근거

- `PermitId`/`Ticket`은 private authority와 local sequence를 가진 opaque handle이다.
- transition API는 전체 handle로 local map을 조회한다.
- `identity_authority.rs`가 같은 local sequence의 foreign handle을 상대 Governor API에 직접 제출한다.
- engine은 다른 요청의 full parent plan registry를 보유하지 않는다.

## 변경 파일

- `crates/taskmesh-engine/src/{shared/mod,engine/governor,engine/state}.rs`
- public re-export와 host use-site: `crates/taskmesh-engine/src/lib.rs`, `crates/taskmesh/src/runtime.rs`
- tests: `hardening_child_scope.rs`, `hardening_lease_token.rs`, `identity_authority.rs`
- public docs and migration notes

## 작업 계획

1. 두 Governor가 같은 permit/ticket local sequence를 발급하는 control을 고정했다.
2. foreign release/advance/claim/abandon이 typed negative outcome, state/callback 0임을 고정했다.
3. Governor authority + local sequence를 private하게 가진 handle과 telemetry sequence를 분리했다.
4. parent membership은 외부 planner owner와 fixture를 지정한다.
5. public facade와 MSRV consumer를 migration한다.

## DoD

- `BG25-004-B21`: class/stage/queued 상태와 무관하게 같은 live `(root, operation)`은 side-effect 0으로 거절되고 terminal 뒤 재사용된다.
- `BG25-004-B22`: 현재 non-membership behavior가 고정되고 목표 planner는 Governor 호출 전에 없는 stage를 거절한다.
- `BG25-004-B28`: 네 raw API의 충돌이 재현되고 목표 handle은 foreign state/callback 0의 typed reject를 보장한다.
- `BG25-004-H18`: public import 경로에서 raw/validated wire, host/direct multi-stage reservation, default/Rayon owned/shared 선언을 확인한다.

## 검증

- Engine identity tests, host public-surface tests, Rayon feature tests, consumer-MSRV.
- `cargo test --locked -p taskmesh-engine --test identity_authority`
- Counter exhaustion/wrap은 `IdentityExhausted`이며 ID 재사용이 아님을 세 counter 각각 검증한다.

## 현재 재현 범위 (2026-09-25)

- `cross_governor_ids`의 4개 control이 두 Governor의 raw ID 충돌과 상대 Governor에서의 `release`/`advance_phase`/`claim`/`abandon` 로컬 효과를 각각 재현한다. Focused test 4/4와 해당 target Clippy가 통과했다.
- 이 테스트는 **현행 결함 재현**이다. foreign 입력을 거절하는 목표 동작, opaque owner-bound handle, public facade migration은 아직 구현되지 않았다.
- D5의 breaking API/semver 선택은 미결이다. 결정 전에는 raw aliases/outcomes를 교체하거나 재현 테스트를 성공 증거로 승격하지 않는다.

## 인계 및 중단 조건

- Breaking handle/outcome change는 BG25-001의 semver 결정 전 적용하지 않는다.
- 새 handle을 숫자/문자열로 재구성 가능하게 만들지 않는다.
