# TM16-001 — Substrate 대기열이 class queue/Reject 정책을 우회한다

- Severity: P1
- Status: OPEN
- Lane: runtime-topology
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

crates/taskmesh/src/runtime.rs:340-362,943-944; crates/taskmesh/src/executor/cancel.rs:30-44; docs/runtime-inventory-baseline.md:54-57

## Trigger / 관찰

`blocking_threads=1`, class `max_inflight=1`, `max_queue_depth=0`, `OverflowPolicy::Reject`; 한 작업이 슬롯을 점유한 동안 32개 `run_blocking`을 제출한다.

32개 요청이 거부되지 않고 semaphore에서 대기하며 snapshot의 class queued는 0이다. holder를 해제하면 모두 성공한다. CPU는 항상 topology gate가 있으므로 같은 구조가 기본 CPU 경로에도 적용된다.

## 원인 / 영향 / 범위

worker gate를 semantic admission 전에 기다리는 별도 backlog에는 depth 제한이 없다. timeout은 대기 시간만 제한하며 동시 waiter 수를 제한하지 않는다. permit accounting과 실제 pending 수가 분리되어 overload 관찰/제한이 실패한다.

동일 ingress 구조의 추가 증상 (별도 중복 티켓으로 세지 않음):

- unknown/disabled class 및 malformed spec도 포화 gate 뒤에서 기다린 후에야 terminal reject된다. 기본 unbounded submission은 worker가 영구 점유되면 unknown class조차 반환하지 않을 수 있다.
- semantic capacity를 기다리는 A가 physical slot을 이미 보유하여, 실제 blocking worker가 없어도 runnable B가 worker slot을 못 받는다. 다른 substrate에서 A의 class budget을 사용 중이면 이 조합이 자연스럽게 reachable하다.
- gate의 FIFO가 class fairness보다 먼저 중재하므로 대기 중 primary/WFQ/DRR/DeadlineAware 요청을 governor가 볼 수 없다. best-effort waiter가 먼저 gate에 들어가면 pending primary보다 먼저 실행할 수 있다. subagent가 idle-slot hoarding과 best-effort-first 각각 focused probe로 재현했으며 main의 retained probe는 queue-depth/Reject 우회를 재현한다.

## 보완 계획

worker governance와 semantic policy를 분리한 채 pending admission의 bounded reservation/ticket을 먼저 확보한다. physical waiter 개수/bytes와 per-class 상한을 명시하고 포화 시 typed reject를 반환한다. gate를 단순히 뒤로 옮겨 worker queue에 permit을 무제한 적재하는 수정은 피한다.

## Acceptance / 회귀 검증

hold 슬롯 + N 제출에서 end-to-end pending 상한, Reject의 즉시 거부, queued snapshot, cancel/timeout/drop 후 pending·gate·permit 회수, 여러 class fairness를 확인한다.
