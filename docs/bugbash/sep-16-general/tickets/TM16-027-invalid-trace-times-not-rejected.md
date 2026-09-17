# TM16-027 — Trace parser가 invalid time을 허용해 replay를 왜곡하거나 panic한다

- Severity: P2
- Status: OPEN / debug and release observation retained
- Lane: B — benchmarks
- Baseline: `9ae9547216c70458f54f37368a67661321060886`, 2026-09-16

## 근거

- `crates/taskmesh-bench/src/workload.rs:240-266`: timestamp는 f64 parse 성공만 확인한다. finite/nonnegative/ordered 조건이 없다.
- `crates/taskmesh-bench/src/loadgen.rs:135-159`: f64 timestamp를 `as u64`로 변환하고 ManualClock을 입력 순서대로 설정한다.
- `crates/taskmesh-bench/src/loadgen.rs:166,230`: completion time 덧셈이 unchecked다.
- [Retained observation](evidence/repro-bench/src/lib.rs): `trace_parser_accepts_invalid_event_times`.

## Trigger / 관찰

- `NaN,retrieval`, `-1,retrieval`, `2,retrieval\n1,retrieval`을 모두 Ok로 parse한다. simulate도 conserved result를 반환한다.
- `inf,retrieval`도 Ok다. ns cast가 u64::MAX로 saturate되어 service_ns=10 덧셈에서 debug panic이 발생한다. release에서는 completion time이 wrap된 뒤 conserved result를 반환한다.

## 원인 / 영향 / 범위

문법적으로 parse 가능한 float와 유효한 event schedule을 혼동한다. NaN/음수는 zero-time으로 바뀌고 역순 trace는 virtual time을 후퇴시킨다. 무한대/큰 finite time은 overflow를 만든다. 요청 수 conservation과 dropped=0만으로 이 오류를 검출하지 못한다. 실제 host runtime 결함이 아닌 benchmark/replay proof 입력 검증 결함이다.

## 보완 계획

- finite, nonnegative, nondecreasing time과 ns representability를 검사하고 offending line을 반환한다.
- 같은 시각의 arrivals는 허용할 수 있지만 역순 입력을 자동 정렬해 원본 오류를 숨기지 않는다.
- simulate의 public Arrival 입력도 검증하고 time conversion/`t + service_ns`/tail scheduling에 checked arithmetic을 적용한다.

## Acceptance / 회귀 검증

- NaN/±inf/음수/역순/너무 큰 finite time을 debug/release 모두 typed input error로 거부한다.
- valid CSV exact roundtrip, 동시 arrivals, 정상 trace replay를 보존한다.
- 직접 생성한 invalid Arrival도 simulate entry에서 거부된다. overflow가 panic 또는 wrapping green이 되어서는 안 된다.
