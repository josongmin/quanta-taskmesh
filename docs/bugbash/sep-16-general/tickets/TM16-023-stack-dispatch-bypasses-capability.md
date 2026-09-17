# TM16-023 — Blocking stack request가 실제 dedicated worker의 large-stack cap을 우회한다

- Severity: P1
- Status: OPEN
- Lane: runtime-topology
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)
- Evidence: evidence/repro/src/lib.rs

## 근거

crates/taskmesh/src/runtime.rs:480-500,134-165,348-354; crates/taskmesh-contract/src/task.rs:199-205

## Trigger / 관찰

large_stack_slots=1, blocking_threads=0에서 TaskSpec::blocking(c).stack_size_bytes(2MiB) 작업 2개를 동시에 제출한다.

두 requested-stack OS worker가 동시에 실행되며 max concurrent=2다. retained probe 재현. gate는 original BlockingPool hint로 획득되어 configured large_stack_slots를 사용하지 않는다. BackgroundOnly에서도 같은 dedicated dispatch 분기가 reachable하다.

## 원인 / 영향 / 범위

actual executor 선택은 stack_size 필드가 바꾸지만 physical capability selection은 original hint만 따른다. LargeStackCapability cap을 선언해도 accepted alternate hint가 실제 requested-stack execution에 사용될 수 있다.

## 보완 계획

dedicated requested-stack dispatch에는 LargeStackCapability hint를 필수로 하거나 actual dispatch가 요구하는 capability gate를 함께 획득한다. blocking/background stack 요청 API의 backward compatibility와 worker governance 의미를 문서/타입에서 고정한다.

## Acceptance / 회귀 검증

blocking/large-stack/background 힌트 각각 stack request matrix, cap=1 concurrency, mismatch 사전 reject, acquire timeout/cancel 및 실제 worker 종료 후 gate 회수를 검증한다.

