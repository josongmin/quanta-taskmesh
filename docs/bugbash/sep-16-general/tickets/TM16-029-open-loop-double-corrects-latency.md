# TM16-029 — 이미 open-loop로 생성한 요청 latency에 omission backfill을 다시 적용한다

- Severity: P2
- Status: OPEN / deterministic observation reproduced
- Lane: B — benchmarks
- Baseline: `9ae9547216c70458f54f37368a67661321060886`, 2026-09-16

## 근거

- `crates/taskmesh-bench/src/loadgen.rs:41-44`: 모든 measured wait에 `record_correct`를 적용한다.
- `crates/taskmesh-bench/src/loadgen.rs:135-175`: simulator는 predefined arrivals를 admission completion과 무관하게 전부 offered로 처리한다.
- `crates/taskmesh-bench/src/loadgen.rs:225-230`: 각 queued ticket의 실제 wait도 다시 backfill한다.
- `crates/taskmesh-bench/examples/loadgen_probe.rs:41-49`: 이 histogram quantile을 admission-wait headline으로 출력한다.
- [Retained observation](evidence/repro-bench/src/lib.rs): `open_loop_waits_are_recorded_more_than_once`.

## Trigger / 관찰

arrival 2개(0ns, 1ns), inflight=1, service=10000ns, correction interval=1000ns:

- actual waits: 0ns, 9999ns; offered/completed=2.
- histogram samples=10, reported p50=4999ns.
- 8개 synthetic sample은 누락된 실제 요청을 복구한 것이 아니다. simulator는 예정된 2개 arrival를 이미 모두 처리했다.

## 원인 / 영향 / 범위

closed-loop 측정의 미발행 요청 보정과 omission 없는 open-loop discrete-event 측정을 혼합한다. wait가 긴 요청일수록 synthetic sample로 더 큰 가중치를 받고, quantile은 실제 admitted request wait 분포가 아니다. count conservation/dropped=0는 histogram 표본 모집단을 검사하지 않는다. TM16-018의 benchmark non-admission 생략과는 다른 측정 오류다.

## 보완 계획

- open-loop simulator의 primary histogram은 실제 admitted wait마다 raw record를 한 번 수행한다.
- closed-loop/실제 미발행 cadence 측정용 correction recorder는 별도 mode/type으로 분리한다.
- 가상 fixed-cadence 분포를 추가로 보여주려면 synthetic population/count를 명시하고 actual request quantile과 분리한다. Poisson/trace mean interval을 고정 cadence authority로 취급하지 않는다.

## Acceptance / 회귀 검증

- raw histogram len이 completed/started와 일치한다. rejected/leftover population은 별도 집계한다.
- 위 2개 요청은 raw samples=2이며 independently recorded histogram과 quantile이 일치한다.
- 실제 omission을 만든 closed-loop fixture에서만 correction 효과를 검증한다.
- workload rate/queue depth를 바꾸어도 actual quantile의 모집단이 synthetic count로 바뀌지 않는다.
