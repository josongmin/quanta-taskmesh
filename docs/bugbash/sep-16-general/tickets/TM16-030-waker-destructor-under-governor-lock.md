# TM16-030 — Queued waker의 소멸자가 governor mutex 아래 실행되어 재진입 deadlock을 만든다

- Severity: P2
- Status: OPEN / isolated-process deadlock reproduced
- Lane: E — engine lifecycle / driven-port lock discipline
- Baseline: `9ae9547216c70458f54f37368a67661321060886`, 2026-09-16

## 근거

- `crates/taskmesh-engine/src/engine/governor.rs:122-143`: state mutex를 보유한 채 queued request를 제거하고 지역 `req`를 drop한다.
- `crates/taskmesh-engine/src/engine/state.rs:45`: PendingRequest는 `Option<Arc<dyn PermitWaker>>`를 소유한다.
- `crates/taskmesh-engine/src/engine/governor.rs:157-161`: 명시적인 wake callback은 재진입/차단 방지를 위해 lock 밖에서 호출한다. 소멸자 호출에는 이 원칙이 적용되지 않는다.
- [Retained observation](evidence/repro/src/lib.rs): `queued_waker_drop_reenters_governor_under_lock`.

## Trigger / 관찰

1. max_inflight=1인 class의 holder를 admit하고 두 번째 작업을 custom waker와 함께 queue한다.
2. waker의 마지막 strong reference는 queue가 소유한다. waker의 Drop은 Weak<Governor>를 upgrade하여 snapshot을 호출한다.
3. `abandon(ticket)`이 request/waker를 mutex 아래 drop한다. snapshot이 동일 non-reentrant mutex를 다시 취득하려 하여 멈춘다.

Probe는 별도 subprocess에서 destructor 진입 marker를 확인한다. 2초 동안 종료하지 않은 child를 kill/wait하여 회수한다. 감사 종료 후 hung child를 남기지 않는다.

## 원인 / 영향 / 범위

외부 driven-port의 user code는 `wake()`뿐 아니라 마지막 Arc의 destructor에서도 실행된다. queued cancellation이 전체 governor의 admit/release/snapshot을 막을 수 있다. default TokioPermitWaker에서 deadlock을 관찰한 것은 아니며, public engine embedding/custom port 범위다. panic 격리 여부와는 다른 lock-ownership 결함이다.

## 보완 계획

- queue/recursion state mutation을 먼저 완료하고 제거된 request/외부 Arc의 최종 drop을 lock 밖으로 지연한다.
- direct admission/rejection, promotion 및 shutdown에서 마지막 port reference를 drop하는 위치도 감사한다.
- destructor를 금지하는 미문서화된 전제를 추가하여 해결했다고 판단하지 않는다. callback 호출과 callback object retirement의 lock discipline을 함께 정한다.

## Acceptance / 회귀 검증

- 동일 reentrant destructor에서 abandon이 bounded하게 반환하고 snapshot/다음 admit이 정상 동작한다.
- reentrant wake와 reentrant Drop을 각각 검증한다. blocking destructor 동안 다른 governor operation은 진행 가능해야 한다.
- child recursion guard 제거, promoted-ticket abandonment와 resource accounting의 정합성을 보존한다.
