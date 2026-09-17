# TM16-032 — 동기 admission lock 대기가 acquire_timeout을 넘겨도 작업을 시작한다

- Severity: P2
- Status: OPEN / controlled-contention host observation reproduced
- Lane: R — runtime acquisition controls, E interface coordination
- Baseline: `9ae9547216c70458f54f37368a67661321060886`, 2026-09-16

## 근거

- `crates/taskmesh/src/executor/cancel.rs:31-38`: acquire timeout은 governor/physical slot 취득의 최대 대기 budget으로 문서화되어 있다.
- `crates/taskmesh/src/runtime.rs:325-363`: 한 acquisition deadline을 계산하고 gate 및 governor acquisition에 전달한다.
- `crates/taskmesh/src/runtime.rs:231-232`: synchronous `admit_waitable`의 immediate Admitted 결과는 deadline 재검사 없이 성공한다.
- `crates/taskmesh-engine/src/engine/governor.rs:109-112`: admission은 parking_lot mutex 취득 동안 caller thread를 차단한다.
- [Retained observation](evidence/repro/src/lib.rs): `acquire_timeout_does_not_cover_synchronous_admission_lock_wait`.

## Trigger / 관찰

custom waker Drop/barrier로 governor mutex를 100ms 보유하는 상황을 만든다. capacity가 비어 있는 다른 class에 IO 작업을 acquire_timeout=5ms로 제출하면 admission lock에서 기다린 뒤 즉시 Admitted를 받는다. 호출은 Ok이며 실제 job 시작 시각은 제출 후 80ms 이상이다.

TM16-030의 destructor 경로를 contention fixture로 사용했다. 자연적인 host contention 빈도는 측정하지 않았다. 긴 DRR selection(TM16-012) 등 동기 mutex 지연도 같은 unchecked-return 경로에 도달한다는 것은 source evidence다.

## 원인 / 영향 / 범위

deadline select는 physical gate/semantic queue의 async wait에만 적용되고 synchronous admission의 lock wait에는 적용되지 않는다. 이미 만료된 acquisition budget으로 받은 immediate permit을 성공 처리하여 작업 side effect를 시작한다. queue timeout/promotion 경합의 last-chance claim과 다른 경로다. CompleteBy execution deadline의 late-result 처리와도 구분한다.

## 보완 계획

- expired acquisition budget 이후에는 immediate admission을 포함하여 작업을 시작하지 않도록 결과 경계에서 deadline을 재검사하고 permit을 unwind한다.
- timed/try admission 또는 nonblocking host bridge로 동기 lock wait의 bound를 정한다. 단순 사후 검사만으로 response latency까지 bound했다고 주장하지 않는다.
- ZERO timeout은 uncontended immediate acquisition을 허용하는 기존 의미를 보존하고, timer/preemption 한계를 문서화한다.

## Acceptance / 회귀 검증

- controlled contention에서 budget 초과 시 typed PermitAcquireTimedOut를 반환하고 job이 시작되지 않는다.
- 응답 latency 상한은 별도로 검증한다. actual release/promote batch와 long DRR contention도 포함한다.
- ZERO/free capacity, finite/free capacity, queued timeout, cancel/absolute deadline 및 expiry 직전 성공을 구분하여 검증한다.
