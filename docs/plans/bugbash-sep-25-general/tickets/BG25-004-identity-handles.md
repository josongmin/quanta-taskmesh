# BG25-004 — identity·parent plan·owner-bound handle

- 구현 상태: IMPLEMENTED
- 증명 상태: STATIC_MAPPED
- 외부 상태: NOT_APPLICABLE
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
4. parent membership의 검증 책임은 외부 planner에 남는다. 실제 consumer owner와 fixture는 아직 연결되지 않았다.
5. public facade와 MSRV consumer를 opaque handle로 이전했다.

## DoD

- `BG25-004-B21`: class/stage/queued 상태와 무관하게 같은 live `(root, operation)`은 side-effect 0으로 거절되고 terminal 뒤 재사용된다.
- `BG25-004-B22`: 현재 non-membership behavior가 고정되고 목표 planner는 Governor 호출 전에 없는 stage를 거절한다.
- `BG25-004-B28`: 네 raw API의 충돌이 재현되고 목표 handle은 foreign state/callback 0의 typed reject를 보장한다.
- `BG25-004-H18`: public import 경로에서 raw/validated wire, host/direct multi-stage reservation, default/Rayon owned/shared 선언을 확인한다.

## 검증

- Engine identity tests, host public-surface tests, Rayon feature tests, consumer-MSRV.
- `cargo test --locked -p taskmesh-engine --test identity_authority`
- Counter exhaustion/wrap은 `IdentityExhausted`이며 ID 재사용이 아님을 세 counter 각각 검증한다.

## 현재 증거 범위

- `cross_governor_ids`는 이전 raw local-sequence 충돌의 회귀 control이고 `identity_authority::foreign_handles_never_alias_same_sequence_local_state`는 현재 foreign handle의 네 transition 거절을 검증한다.
- `PermitId`/`Ticket`은 private Governor authority를 가진 opaque handle이며 public facade, fuzz harness, consumer-MSRV fixture가 새 결과형을 사용한다. 실행 결과는 최종 committed HEAD의 BG25-012 receipt에서 판정한다.
- D5 라이브러리 선택은 [DECISIONS](DECISIONS.md)에 accepted다. 이 티켓은 라이브러리 handle 이전을 담당한다. 외부 consumer의 API/semver migration 승인과 parent-plan membership 검증은 BG25-001/003의 OPEN 경계로 남는다.
- Header의 `NOT_APPLICABLE`은 이 티켓이 외부 채택을 소유하지 않는다는 뜻이다. 외부 작업 자체는 BG25-001/003에서 OPEN으로 추적한다.
- B21은 admitted holder의 stage 차이와 함께 `duplicate_identity_is_global_across_classes_and_includes_queued_requests`로 class 차이, queued identity, 종료 후 재사용까지 검증한다.
- H18은 Rayon CPU soak/drain, physical-domain authority, direct/host multistage reservation, public wire roundtrip과 legacy alias를 `test-rayon` exact selectors로 연결한다. 별도 `consumer-msrv` gate가 default+rayon 외부 fixture를 declared MSRV에서 실행하므로 최종 호환성 판정은 두 gate가 모두 PASS일 때만 성립한다.

## 인계 및 중단 조건

- Breaking handle/outcome change는 BG25-001의 semver 결정 전 적용하지 않는다.
- 새 handle을 숫자/문자열로 재구성 가능하게 만들지 않는다.
