# BG25-007 — stage 선언과 root/child lifetime

- 상태: PLANNED
- 우선순위: P1
- 선행: BG25-001, BG25-006
- 소유: host/runtime owner, API docs owner

## 목적

Taskmesh가 실행하는 bootstrap root와 caller가 소유하는 later stage, reducer, detached child의 lifetime 경계를 fixture와 공개 계약으로 고정한다.

## 근거

- 한 `run_*`는 첫 dispatch만 실행한다.
- `run_io`/`run_local` lease는 caller-owned root가 끝나면 반환된다.
- `LocalSet::run_until(root)`은 미await `spawn_local` child completion을 약속하지 않는다.
- ambient `tokio::spawn` child는 Governor custody 밖이다.

## 변경 파일

- 문서: `docs/taskmesh-library-spec.md`, `docs/taskmesh-external-interface.md`
- 필요 시 `crates/taskmesh/src/runtime.rs` rustdoc/API
- 신규 host fixture 후보 `crates/taskmesh/tests/hardening_root_child_scope.rs`
- existing `e2e_scenarios.rs`, `runtime_local.rs`, `hardening_dispatch_resolution.rs`

## 작업 계획

1. sequential/fan-out declarations와 실제 closure 실행 횟수를 독립 counter로 비교한다.
2. IO/local caller panic/drop과 raw spawned child lifetime을 분리한다.
3. `spawn_local` and ambient Tokio child의 completion/drop/drain 의미를 고정한다.
4. Structured child tracking이 제품 요구라면 별도 API/RFC로 분리한다.

## DoD

- `BG25-007-A11`: later stages/reducer는 자동 실행·예약되지 않고 caller의 별도 submission만 새 permit을 얻는다.
- `BG25-007-H21`: root panic/drop은 root lease를 반환하지만 raw detached child를 WorkerPanicked나 governed custody로 위장하지 않는다.
- `BG25-007-H35`: unawaited local/ambient child의 drop·side effect·drain 의미가 각각 fixture와 문서에 일치한다.

## 검증

- Host root-child fixture, local-runtime fixture, public docs examples.
- Detached work를 기다리는 sleep 기반 테스트 대신 explicit channel을 사용한다.

## 현재 증거 범위 (2026-09-25)

- `hardening_root_child_scope` 5개 focused case와 target Clippy가 통과했다. 후속 stage/reducer의 비실행과 별도 caller submission, ambient Tokio child의 root/drain 외부 생존, local child의 LocalSet 종료 시 drop, root panic/abort의 root lease 반환을 channel로 고정한다.
- 두 공개 계약 문서에 `run_io` ambient child와 `run_local` unawaited child의 상이한 lifetime을 명시했다. requested-stack owned-runtime child custody는 별도 기존 계약으로 유지한다.
- 이 결과는 임의 detached child를 Taskmesh가 추적한다는 증거가 아니다. BG25-012의 selector·exact-source CI-profile 증거 전에는 전체 완료로 승격하지 않는다.

## 인계 및 중단 조건

- 임의 child 추적을 기존 `run_*`에 암묵적으로 추가하지 않는다.
- structured concurrency가 필요하면 본 티켓을 닫고 별도 public API 설계로 확장한다.
