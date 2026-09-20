# SEP21-H01 — Pre-admission validated dispatch plan

- 상태: LOCALLY_VERIFIED
- 우선순위: P1
- 포함 finding: TM21-007
- 선행: C01
- write lane: `host`
- 후속 소비자: H02, H03

## 목적

dispatch에 필요한 consumer input을 governor 호출 전에 typed form으로 검증한다. invalid stack
요청을 admit한 뒤 원복하는 구조를 제거하고 H02/H03이 공유할 최종 preflight boundary를 만든다.

## RCA

- `ResolvedExecutionPlan`이 substrate를 고르지만 stack presence/range/`usize` 변환은 worker
  helper가 나중에 수행한다.
- sync/async requested-stack path가 raw `TaskSpec`을 각각 다시 읽는다.
- validation failure가 queue/fairness/resource state를 이미 변경한 뒤 발생한다.

## 확정 근거

- blocking path의 admission-before-stack-validation:
  `crates/taskmesh/src/runtime.rs:393-414,453-475`.
- async requested-stack의 admission-before-range-validation:
  `crates/taskmesh/src/runtime.rs:527-575`.
- 16 GiB/`usize` validator가 dispatch helper에만 존재:
  `crates/taskmesh/src/runtime.rs:1006-1037`.

## 목표 구조와 불변식

- `ValidatedDispatchPlan`이 substrate, registered capability, validated stack size, deadline/cancel
  compatibility를 admission 전에 freeze한다.
- `ValidatedStackSize(usize)`는 공통 validator만 생성한다.
- worker/dispatch code는 raw `TaskSpec::stack_size_bytes`를 읽거나 재검증하지 않는다.
- validation precedence는 plan shape → dispatch semantics → admission 순서다.

## 작업 플랜

1. `crates/taskmesh/src/execution_plan.rs`
   - `ValidatedStackSize`, `ValidatedDispatchPlan`과 공통 preflight validator를 추가한다.
   - 16 GiB upper bound와 `usize::try_from`을 여기서 처리한다.
2. `crates/taskmesh/src/runtime.rs`
   - `requested_stack_size_bytes_v1` 후검증을 제거한다.
   - sync blocking, background, large-stack async가 plan의 typed 값만 사용한다.
3. `crates/taskmesh/src/lib.rs`
   - 기존 public maximum constant가 있다면 source-compatible export를 유지한다.
4. C01 validator와 오류 precedence를 맞추고 closure evidence에 public migration/documentation
   delta를 남긴다. shared external interface/library spec/ADR/CHANGELOG 반영은 R01이 통합한다.

## 테스트 플랜

- `crates/taskmesh/tests/hardening_executor_protocol.rs`
  - missing/zero/max/max+1/`usize` conversion matrix와 exact error.
- `hardening_dispatch_resolution.rs`
  - 모든 blocking-family hint가 같은 validator를 사용.
- held large-stack slot 뒤 invalid waiter가 즉시 반환하고 queue/ticket/permit/capability/counter가
  전혀 변하지 않는 contention fixture.
- intentional mutant: validator를 acquire 뒤로 이동하면 snapshot oracle이 실패.

## DoD

- `SEP21-H01-A01`: invalid 요청은 worker/thread/closure 0, engine state delta 0이다.
- `SEP21-H01-A02`: sync/async/requested-stack 모든 경로에 validator 구현이 하나뿐이다.
- `SEP21-H01-A03`: maximum-valid preflight와 실제 OS spawn failure는 다른 typed outcome이다.
- `SEP21-H01-A04`: 32/64-bit conversion 정책이 compile fixture 또는 injectable seam으로 검증된다.
- `SEP21-H01-A05`: H02/H03이 raw stack 필드에 접근하지 않고 final plan을 소비한다.

## 금지되는 임시방편

- clamp, OS spawn을 validator로 사용, path별 validator 복제.
- admit 후 즉시 release를 “사전 검증”으로 간주.
- host semaphore 추가.

## 검증 명령 후보

```sh
cargo test --locked -p taskmesh --test hardening_executor_protocol --test hardening_dispatch_resolution
cargo test --locked -p taskmesh --test substrate_enforcement
```

## Closure evidence (2026-09-21)

- source: uncommitted `host` lane atop `main@0fb1874`
- owner: `ValidatedDispatchPlan::preflight` validates C01 shape, dispatch semantics, requested
  stack, registered capability requirements, and cancellation/deadline compatibility before
  `Governor::admit_validated_requirements`.
- stack authority: only `ValidatedStackSize::validate` applies nonzero, 16 GiB, and target-width
  conversion. Worker paths consume `ValidatedStackSize::get` and never re-read raw stack bytes.

| Acceptance | Local evidence |
| --- | --- |
| SEP21-H01-A01 | `invalid_stack_preflight_has_zero_governor_and_worker_side_effects` holds a contended slot and proves exact snapshot equality plus closure count 0 for zero/over-limit sync and async requests. |
| SEP21-H01-A02 | Blocking, background, dedicated blocking, and requested-stack async all consume the same `ValidatedDispatchPlan`; the old runtime post-validator was removed. |
| SEP21-H01-A03 | Preflight policy failures remain `PolicyViolation`; valid preflight followed by OS spawn failure remains `WorkerUnavailable` in executor protocol fixtures. |
| SEP21-H01-A04 | `stack_validator_has_an_injectable_32_bit_conversion_boundary` proves the exact 32-bit `usize` edge without requiring a 32-bit host. |
| SEP21-H01-A05 | H02 `AcquisitionArbiter` and H03 capability/domain resolution receive only the frozen plan; neither reads `TaskSpec::stack_size_bytes`. |

Validation: taskmesh default 160/160 PASS; taskmesh+rayon 160/160 PASS; contract 57/57 PASS;
default/rayon/taskmesh-rayon clippy `-D warnings` PASS; consumer MSRV default+rayon PASS on Rust
1.81. Workspace/release qualification remains R01.
