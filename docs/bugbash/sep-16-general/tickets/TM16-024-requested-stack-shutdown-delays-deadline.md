# TM16-024 — Requested-stack owned runtime teardown이 deadline 응답을 blocking child 종료까지 지연한다

- Severity: P1
- Status: OPEN
- Lane: runtime-topology
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)
- Evidence: evidence/repro/src/lib.rs

## 근거

crates/taskmesh/src/runtime.rs:82-114; docs/taskmesh-library-spec.md:86-101; docs/taskmesh-external-interface.md:47-50

## Trigger / 관찰

requested-stack async root가 owned runtime에서 150ms spawn_blocking child를 만들고 pending으로 yield한다. CompleteBy=20ms를 제출한다.

root는 DeadlineExceeded가 되지만 caller result는 owned runtime drop이 blocking child 종료를 기다린 뒤 100ms 이상 지나 반환된다. retained probe 재현. long non-yielding root와 달리 root 자체는 계속 cooperative/yielding이다.

## 원인 / 영향 / 범위

runtime은 catch_unwind 내부 local로 소유되어 tx.send 이전 scope teardown에서 drop된다. Tokio runtime shutdown은 running blocking task를 끝낼 수 없어 drop이 기다린다. 영구 blocking child이면 deadline/cancellation caller completion도 영구 지연 가능하다.

## 보완 계획

지원하는 child execution 범위를 제한하거나 논리적 deadline 결과 전달과 실제 cleanup worker ownership을 분리한다. cleanup 중 ExecutionLease는 계속 worker/cleanup owner가 보유해야 한다. shutdown_timeout을 써서 ungoverned child를 남긴 채 permit을 반납하는 수정은 금지한다.

## Acceptance / 회귀 검증

yielding root + blocking child의 deadline/cancel/caller-drop, cleanup중 inflight/gate 유지, child 종료후회수, 정상 성공 completion fence와 동기화한다. TM16-015의 release fence는 actual cleanup completed 성공 경로와 이 pending-cleanup terminal 경로를 구분해 적용한다.
