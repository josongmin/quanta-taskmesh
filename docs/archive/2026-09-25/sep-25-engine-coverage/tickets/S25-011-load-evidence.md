# S25-011 — simulator와 실제 host 부하 증거

- 상태: PLANNED. 우선: P1. 선행: [S25-005](S25-005-executor-authority.md), [S25-006](S25-006-deadline-and-custody.md), [S25-008](S25-008-admission-capacity.md). 소유: bench owner; host fixture는 host owner.
- 주 담당 시나리오: H17, H28. 추가 측정 계약 A03은 53개 미충족 ID의 주 담당 수에 포함하지 않는다.

## 목적

simulator admission-wait 통계와 실제 host 응답 지연의 모집단을 분리하고, 부하 중 사라진 요청과 응답 뒤 남은 worker custody를 독립 원장으로 판정한다. 현재 `crates/taskmesh-bench/src/loadgen.rs`는 open-loop discrete-event simulator이고 단일 class overload에서 `lat.len()==started()`를 이미 검사한다. `tests/hellgate.rs`의 multi-seed는 단일 class이며 `crates/taskmesh/tests/e2e_chaos.rs`는 고정 batch라 실제 host open-loop 지연 증거가 아니다. `bench-smoke`는 harness 실행만 확인한다.

## 변경 파일

| 구분 | 경로 | 변경 목적 |
|---|---|---|
| 기존 simulator | `crates/taskmesh-bench/src/loadgen.rs`, `src/workload.rs` | 다중 class trace, virtual arrival/queue/start/complete/reject event ledger와 `LatencyRecorder::raw()` 모집단 대조. |
| 기존 simulator fixture | `crates/taskmesh-bench/tests/hellgate.rs`, `tests/inferno.rs` | multi-class/multi-seed, overload/undersaturation/queue-bound 조합. |
| 신규 host fixture | `crates/taskmesh/tests/host_open_loop.rs` | public facade로 실제 open-loop 제출, per-request offered/terminal/unanswered/custody 원장. |
| 기존 host fixture 참고 | `crates/taskmesh/tests/e2e_chaos.rs`, `hardening_deadline_custody.rs` | bounded chaos와 응답 뒤 worker custody helper 재사용 여부 확인. 고정 batch를 open-loop 증거로 재표기하지 않음. |
| 조건부 측정 도구 | `crates/taskmesh-bench/benches/host_edge_paths.rs`, `tools/bench-gate.sh`, `tools/bench/perf-gate.json` | 성능 회귀 수치가 실제로 필요하고 quiet-host baseline이 생긴 뒤 별도 rail로 적용. 기능 fixture 때문에 임의 p99 threshold를 추가하지 않음. |
| 증거 문서 | `docs/taskmesh-library-spec.md`, `docs/taskmesh-external-interface.md` 및 이 계획의 검증 기록 | simulator/host metric 정의와 sample denominator, source·environment identity. |

## 구현 순서

1. simulator event trace는 입력 `ValidatedArrivals`의 class/ordinal/time을 고정해 arrival→admit/queue/reject→start→complete를 각각 기록한다. `SimResult::is_conserved()`와 같은 계산을 복제한 helper 대신 trace에서 독립 집계한다. raw latency는 started 요청의 **admission wait** 한 sample씩이고 rejected/leftover는 별도 집계한다. omission-corrected synthetic sample과 혼합하지 않는다.
2. 다중 class·다중 seed trace를 overload/undersaturation에 적용한다. 동일 seed replay의 event sequence, queue high-water, final snapshot을 비교한다. 포화 이후 p99의 무조건 단조성은 요구하지 않는다.
3. host fixture는 제출을 completion에 의해 pace하지 않는 외부 clock/schedule 기반 bounded producer를 사용한다. 각 요청에 고유 ID·class·path·scheduled/actual offer/response timestamp를 기록한다. 동시에 미응답 요청의 deadline/horizon을 정하고 producer 자체의 지연·드롭도 offered/failed-to-offer로 구분한다. 작업 수와 queue depth는 유한하게 제한한다.
4. host의 terminal response를 Ok, task error, rejected, cancelled, deadline, worker failure로 분류한다. 응답 시점에 끝나지 않은 worker 수/role을 별도 active counter로 기록해 snapshot charge와 대조한다. worker 종료·drain 후에는 독립 active/permit 원장이 0이어야 한다. 응답 latency와 worker completion/custody duration은 다른 분모로 기록한다.
5. functional population fixture를 CI의 `test` selection에 넣고, 수치 성능 주장은 고정 machine·quiet-host·source/feature/input·baseline을 갖춘 별도 bench rail에서만 한다. hosted runner variance를 절대 p99 hard gate로 오인하지 않는다.

## DoD

- [ ] `S25-011-H17`: 다중 class, 여러 seed, overload/undersaturation마다 입력 event 원장에서 `offered = completed + rejected + leftover_queued`, `completed = admitted_immediately + queued_then_promoted`, `raw recorded_calls = started = histogram len`, synthetic=0, queue high-water≤configured depth를 독립 산출한다. 동일 seed trace 재생 결과가 동일하다. 기존 단일 class sample fixture를 중복 증거로 세지 않는다.
- [ ] `S25-011-H28`: 실제 public `run_*` facade의 bounded open-loop에서 unique offered ID = terminal response ID + 명시적 horizon의 unanswered ID. terminal 종류별 합과 class/path별 response latency 모집단이 일치한다. 응답 뒤 남은 worker custody는 외부 active ledger와 snapshot으로 대조하고 child 종료 뒤 drain/charge 0을 확인한다. simulator admission-wait 수치를 host end-to-end latency로 표시하지 않는다.
- [ ] `S25-011-A03`: 성능 비교를 수행할 때는 동일 source digest, toolchain, feature set, workload trace/seed, warmup, quiet-host machine, baseline과 sample denominator를 기록한다. baseline 없는 신규 host 수치나 `bench-smoke` PASS를 p99 개선/회귀 판정으로 쓰지 않는다.

## 계획된 검증

구현 시 owner-local: `cargo test --locked -p taskmesh-bench --test hellgate`, `cargo test --locked -p taskmesh-bench --test inferno`, `cargo test --locked -p taskmesh --test host_open_loop`, `cargo test --locked -p taskmesh --test e2e_chaos`. deterministic population fixture는 `just test`/CI가 선택하도록 S25-012에 요청한다. 실측 성능은 별도 `just bench`/`just bench-gate`의 baseline 계약에 따르며, 이 계획 작성에서는 어떤 테스트·bench도 실행하지 않는다.

## 인계·중단 조건

S25-005의 공유 executor authority, S25-006의 response/custody, S25-008의 capacity 경계가 결정된 뒤 host 측정 기대값을 고정한다. S25-012에는 fixture 선택과 receipt 필드(source/seed/feature/denominator)를 넘긴다. producer가 completion-paced이거나 unanswered를 누락하거나 latency 분모가 다르면 측정 판정을 중단한다. 정량 성능 목표는 baseline 없이 새로 발명하지 않는다.
