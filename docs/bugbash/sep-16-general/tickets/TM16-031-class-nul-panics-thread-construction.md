# TM16-031 — Class 문자열의 NUL이 requested-stack thread 생성에서 typed error 대신 panic을 만든다

- Severity: P2
- Status: OPEN / host panic reproduced
- Lane: R — runtime worker construction
- Baseline: `9ae9547216c70458f54f37368a67661321060886`, 2026-09-16

## 근거

- `crates/taskmesh-contract/src/task.rs:17-20`: TaskClass 생성자는 arbitrary string을 허용한다.
- `crates/taskmesh/src/runtime.rs:703-704`: class 문자열을 OS thread name에 그대로 삽입한다.
- `crates/taskmesh/src/runtime.rs:73-82,153-160`: thread Builder spawn은 worker 내부 catch_unwind 밖에서 수행된다.
- `crates/taskmesh/src/runtime.rs:116,165`: spawn의 Result에 대한 map_err는 Rust thread-name validation panic을 처리하지 못한다.
- [Retained observation](evidence/repro/src/lib.rs): `requested_stack_class_with_nul_panics_before_spawn_error_mapping`.

## Trigger / 관찰

`TaskClass::new("class\0name")`을 class policy에 등록하면 Builder가 성공한다. blocking spec에 2MiB stack request를 넣고 실행하면 std thread construction에서 `thread name may not contain interior null bytes` panic이 발생한다. Tokio caller task의 JoinError::is_panic()이 true이며 `RunError::Governor`로 반환되지 않는다.

## 원인 / 영향 / 범위

semantic identifier를 OS-specific thread-name input으로 쓰면서 추가 제약을 검사/변환하지 않는다. worker panic mapping이 있어도 worker 시작 전 host setup panic은 빠진다. 같은 helper를 사용하는 requested-stack async에도 source상 동일 위험이 있다. async 경로의 독립 실행 재현은 아직 하지 않았다. 일반적인 Unicode class가 문제라는 주장은 아니다.

## 보완 계획

- OS thread name은 안전한 고정 prefix/opaque ID 또는 sanitized label로 생성한다. semantic class identity를 불필요하게 변경하지 않는다.
- class input을 제한하는 정책을 택한다면 Builder/deserialize→validation 경계에서 명시적으로 reject한다.
- thread setup 전체에서 accepted public input이 panic하지 않는지 검증하고 setup failure의 typed error/lease unwind를 보존한다.

## Acceptance / 회귀 검증

- NUL 입력을 typed reject하거나 안전하게 실행하며 caller task가 panic하지 않는다.
- blocking/requested-stack async, long/Unicode class, spawn OS failure를 각각 검증한다.
- rejected setup에서는 factory/job이 시작되지 않고 permit/gate가 회수된다.
