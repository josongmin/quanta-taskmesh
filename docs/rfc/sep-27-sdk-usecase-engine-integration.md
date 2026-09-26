# SEP-27: SDK use case 매핑과 engine 통합 설계

- 상태: **Proposed — 구현 계획; 실행·배포 증거 아님**
- 상위 결정: [SDK DX와 확장성 RFC](sep-27-sdk-dx-extensibility.md)
- Taskmesh 소스: `8a4f3a6e58403c8aede2355a658a8884831a0745`
- 외부 관찰: Semantica `ee4d72c6cb6a61182639e922b878405f24c27960`
- 관찰 범위: Taskmesh owner source/fixtures, Semantica `quanta-runtime/src/governance`와 named consumer test.
  Semantica 관찰 경로에는 미커밋 변경이 없었다. 전체 저장소 또는 배포 바이너리의 clean/activation을 뜻하지 않는다.
- 이 문서의 새로운 메서드 이름은 제안이다. 현재 메서드는 별도 표시한다.

## 1. 이전 원칙과 필드 보존

첫 이전 대상은 소비자 adapter의 호출 경계다. product class registry, identity 발급,
error projection, 기존 실행 컨텍스트를 그대로 둔 채 실제 host 진입점만 바꾼다.
class·operation 두 필드로 기존 spec를 재구성하면 lineage나 provenance가 사라질 수 있다.

| 기존 정보 | 새 facade에서의 보존 위치 | owner / 실패 조건 |
|---|---|---|
| class | builder 및 보존된 TaskSpec | contract 문법 검사, engine 등록 정책 검사; unknown class 거부 |
| operation / root_operation_id | spec 그대로 | consumer가 호출 identity 발급; SDK가 임의 재발급하지 않음 |
| source / reason | spec bridge; 신규 builder의 typed setter | 제품 provenance 변환은 consumer adapter 소유 |
| scope / parent operation / parent stage / parent_awaits | spec bridge 또는 기존 child builder | planner는 부모 stage membership, engine은 live ancestry/wait cycle 소유 |
| stage descriptor / reduce policy | spec bridge 그대로 | metadata 검증 유지; 실행된 workflow로 승격하지 않음 |
| stack_size_bytes | blocking bridge 또는 별도 stack-async builder | host에서 크기와 dispatch 검증; CPU/IO로 옮기며 무시하지 않음 |
| cancellation token / acquire timeout / deadline | SubmitOptions 또는 builder의 private controls | token은 동작 중 공유하고 absolute deadline을 재시작하지 않음 |
| closure/future와 외부 context wrapper | 종단 run 인자 | async가 된다고 blocking 작업을 caller executor에서 실행하지 않음 |

`operation`은 단순 표시 label이 아니다. 동일한 semantic 작업의 동시 root 호출에는
별도 live identity가 필요하다. child는 부모의 root ID를 계승하고 정확한 immediate
parent ID를 보존한다. generic SDK에 제품별 ID 포맷·hash 정책을 넣지 않는다.

## 2. 기존 use case → 새 API → backend 매핑

`*_spec(spec)`는 제안하는 metadata 보존 bridge다. 기존 `run_*_with`도 계속 지원한다.
bridge는 새로운 실행 경로나 재검증 생략 권한을 만들지 않는다.

| ID | 기존 use case / 관찰 anchor | 권장 이전 경로 | backend / 유지할 동작 |
|---|---|---|---|
| U01 | `run_io` / `run_io_with`; `tools/consumer-msrv::everyday_ports` | 새 root는 `io(class, invocation)`; 기존 계획은 `io_spec(spec)` | CallerFuture, borrowed Send future 지원, caller task에서 poll |
| U02 | `run_cpu_with`; default CPU 또는 Rayon | `cpu` / `cpu_spec`, `run_for`, `run(job)` | frozen CpuExecutor; role cpu + adapter physical domain; CompleteBy 거부 |
| U03 | blocking SDK query/index/session 작업 | `blocking_spec(spec).run(job)`부터 이전 | BlockingPool; Send + static closure/result; 기존 error projection 유지 |
| U04 | BackgroundOnly warmup | 초기에는 `blocking_spec(spec)` | maintenance role + physical.shared_blocking; 일반 blocking spec로 재작성 금지 |
| U05 | blocking spec + stack size | `blocking_spec(spec)` 또는 `blocking(...).stack_size_bytes(...)` | DedicatedStackThread; large_stack + physical.dedicated; caller timeout 이후에도 lease 유지 |
| U06 | `run_async_with_requested_stack_with` | `stack_async_spec(spec)` / 신규 `stack_async(class, invocation, bytes)` | DedicatedStackRuntime; factory만 worker로 전송, future는 그 worker에서 생성; teardown custody 유지 |
| U07 | `run_local_with` / non-Send payload | `local` / `local_spec` | local role, caller affinity와 기존 LocalSet/context 조건; IO로 강제 변환 금지 |
| U08 | child / awaited child | 초기에는 기존 `TaskSpec::child_of`·`awaited_child_of` + spec bridge | 동일 ancestry/wait-cycle 검사; parent plan membership은 planner 소유 |
| U09 | multi-stage / fan-out / reduce 선언 | spec bridge 그대로; 실제 다음 작업은 별도 submission | 첫 실행만 host dispatch; 선언 전체를 자동 실행하지 않음 |
| U10 | 직접 permit/ticket을 관리하는 embedder | 명시적인 declared-plan admission alias; resolved 경로 유지 | claim/abandon/release/leased release 및 waker lifecycle 보존 |
| U11 | memory measurement / stage release | 기존 `ext` API 유지 | reconcile sequence, ledger, 실제 memory release; builder에 raw permit 노출하지 않음 |
| U12 | untrusted JSON task/config | `parse_task_spec` / `parse_runtime_config` 유지 | bytes 검사 → validated plan/config → 기존 host 진입; raw Serde로 교체 금지 |
| U13 | shutdown / snapshot / clone | runtime handle 그대로; config만 내부 공유 | admission close → queued/inflight settlement → drain; clone은 동일 Governor |
| U14 | custom CPU executor / Rayon | Builder의 기존 executor 설치 경계 | descriptor validation/freeze, declared finite capacity, physical domain 및 context 필요성 |
| U15 | absolute deadline을 전달하는 synchronous consumer | additive builder 이전과 분리해 아래 D01 해결 | 현재 CompleteBy 거부를 유지; 필요한 response-only 계약을 별도 구현 |

U10–U14는 새 일상 DSL로 억지로 감싸지 않는다. 기존 low-level 계약을 유지하는 것도
명시적인 mapping이다. 임의의 future/closure를 섞은 `submit(Work)` enum으로 type erasure를
확대하지 않는다.

검토한 fixture anchors: [consumer](../../tools/consumer-msrv/src/main.rs),
[dispatch](../../crates/taskmesh/tests/hardening_dispatch_resolution.rs),
[root/child](../../crates/taskmesh/tests/hardening_root_child_scope.rs),
[memory](../../crates/taskmesh-engine/tests/hardening_memory_epochs.rs),
[ingress](../../crates/taskmesh/tests/strict_ingress.rs).
이 anchor 목록은 검증 실행 결과가 아니다.

## 3. Semantica 소비자 이전 계획

관찰 저장소는 sibling `semantica-codegraph-v2`다. 아래 경로의 공통 prefix는
`packages/analysis/quanta-v2/crates/quanta-runtime/`이다. 현재 observation과 과거 실행은
[external adoption ledger](../plans/bugbash-sep-25-general/tickets/EXTERNAL-ADOPTION.md)의
서로 다른 증거로 취급한다.

| consumer owner | 관찰한 현재 동작 | 이전 작업 |
|---|---|---|
| `src/governance/plan_builder.rs::distinct_root_invocation_v1` | sequence를 붙이고 긴 label은 hash해 서로 다른 root identity 생성 | identity 발급을 유지. 새 facade의 operation 인자에 원래 label만 전달하는 치환 금지 |
| `plan_builder.rs`의 public task builders | `source=PublicSdk/Warmup`, `reason=DerivedFromRequestKind` | 전체 spec bridge 사용; 제품 mapping을 Taskmesh contract로 이동하지 않음 |
| `plan_builder.rs::warmup_background_task_spec_v1` | BackgroundOnly spec | U04 유지; 일반 `.blocking(class, id)`로 role을 바꾸지 않음 |
| `runtime_adapter.rs::runtime_v1` | OnceLock의 `Result<TokioRuntime, GovernorError>`를 호출마다 clone | Taskmesh config Arc 공유가 이 경로의 registry 복사를 줄임. bootstrap 오류 보존 |
| `runtime_adapter.rs::run_by_hint_v1` | hint별 IO/blocking/CPU 분기; 다른 hint 거부 | 초기에 그대로 유지. caller별 실제 async/CPU 성격을 조사한 뒤 typed 진입점으로 좁힘 |
| `runtime_adapter.rs::run_async_with_requested_stack_v1` | optional absolute deadline와 Send factory 전달 | stack_async bridge 사용; future factory를 caller에서 미리 실행하지 않음 |
| `mod.rs`의 context wrapper / `public_projection.rs` | product execution context 전달과 typed/product 오류 변환 | 호출 순서, error kind, retry hint 유지; SDK에 제품 enum 추가하지 않음 |

### 3.1 이전 전에 해결할 충돌

**D01 — blocking absolute deadline의 실제 소비자 수요와 현재 지원 범위 충돌.**
`mod.rs::run_execute_query_task_before_v1` → `runtime_adapter.rs::run_blocking_before_v1`은
`with_absolute_deadline`을 synchronous blocking 경로에 전달한다. public class 등록은
기본 `PreSubmitOnly`를 유지한다. 현재 Taskmesh 소스에서 이 조합은 class preflight의
`DeadlineUnsupported`로 거부되고, class를 deadline 지원으로 바꿔도 sync dispatch의
CompleteBy 거부가 남는다. consumer의 `absolute_ide_query_deadline_rejects_before_worker_then_allows_successor_v1`
등은 DeadlineExceeded 및 미래 deadline 성공을 기대한다.

이는 두 소스를 함께 사용했을 때의 계약 충돌이라는 정적 결론이다. 이번 작업에서 그
consumer 테스트를 실행하지 않았으며 배포된 버그나 실행 실패를 새로 관찰했다고 주장하지 않는다.
해결은 §7의 response-only 계약을 도입하거나, 실제 cooperative async 작업으로 전환하는
경우에 한한다. class 정책만 바꾸거나 blocking closure를 `async { job() }`로 감싸는 것은
cooperative 실행 전환이 아니다.

**D02 — 옛 별도 substrate gate를 전제한 topology sizing 설명.**
`runtime_adapter.rs` 상단은 class queue 앞에 별도 substrate gate가 있다고 설명하고
`inflight + queue + 1`로 worker 수를 정한다. 현재 Taskmesh는 role/physical requirements를
하나의 Governor admission에서 처리한다. 이전 시 해당 설명을 고치고 class queue와
physical saturation을 별도로 검증한다. 숫자를 즉시 줄이지 않고 실제 workload와 pool
공유 관계로 다시 산정한다.

**D03 — sync job을 AsyncIo hint로 실행하는 branch.**
`run_by_hint_v1`의 IO arm은 `async move { job() }`다. 동기 job의 poll이 오래 걸리면
협력적 deadline 응답성도 제한된다. 실제 caller inventory로 blocking 가능성을 확인해
분류를 결정한다. 이 branch가 있다는 사실만으로 모든 IO 사용이 결함이라고 판단하지 않는다.

### 3.2 단계적 consumer cutover

1. Taskmesh additive 변경을 고정하고 기존 consumer 경로를 먼저 실행한다.
2. Semantica adapter의 U02/U03 등 단순 경로부터 spec bridge로 바꾼다. 제품 public 함수 signature는 유지한다.
3. 원래/새 경로를 같은 입력으로 **각각 격리 실행**해 identity, verdict, pool accounting, output을 비교한다.
   side effect가 있는 작업을 production에서 이중 제출하는 shadow 실행은 금지한다.
4. background, requested-stack, root concurrency, queue-full, error projection을 비교한다.
5. D01은 별도 변경과 oracle로 닫은 뒤 deadline wrapper를 이전한다.
6. 두 저장소 source SHA, manifest resolution, feature/target/toolchain과 로그를 함께 기록한다.
   외부 activation 승인과 배포 topology는 consumer owner 증거로 남긴다.

## 4. 새 facade의 구현 형태

제안 파일은 `crates/taskmesh/src/submission.rs`다. 아래 type은 설계용 의사코드다.

```text
SubmissionData<'rt> { runtime: &'rt TokioRuntime, spec: TaskSpec, options: SubmitOptions }
CpuSubmission<'rt>          { data: SubmissionData<'rt> }
BlockingSubmission<'rt>     { data: SubmissionData<'rt> }
IoSubmission<'rt>           { data: SubmissionData<'rt> }
LocalSubmission<'rt>        { data: SubmissionData<'rt> }
StackAsyncSubmission<'rt>   { data: SubmissionData<'rt> }

TokioRuntime::cpu(class, invocation) -> CpuSubmission
TokioRuntime::cpu_spec(spec)         -> CpuSubmission
CpuSubmission::run(self, job)        -> Future<Result<T, RunError<E>>>
```

각 타입은 private 필드와 consuming setters를 가진다. copy/paste한 validator나 런타임
kind enum에 의존하는 범용 public builder를 만들지 않는다. 공통 setter 구현은 내부 helper로
공유하고, 지원하지 않는 메서드는 해당 타입에 제공하지 않는다.

| builder | 공통 controls | 추가 controls / 종단 입력 |
|---|---|---|
| CPU | cancel, acquire_timeout, run_for | Send + static job/result; complete_by·stack 설정 없음 |
| Blocking | 동일 | stack_size_bytes; Send + static job/result |
| IO | 동일 + complete_by | 기존 borrowed Send future 조건 유지 |
| Local | 동일 + complete_by | 기존 non-Send 및 static 조건 유지 |
| StackAsync | 동일 + complete_by, 필수 stack | Send + static factory; 생성된 future는 non-Send 가능, static |

`*_spec`는 spec를 이동해서 보관하고 필드를 다시 쓰지 않는다. constructor에 실수로
다른 hint가 들어가면 기존 `run_*_with` preflight의 mismatch를 그대로 반환한다.
새 단순 생성자의 source/reason 기본값은 현재 TaskSpec와 같고 typed setter로 변경한다.
첫 버전의 복잡한 child/stage 선언은 spec bridge로 전달해 API 크기를 제한한다.

첫 구현은 종단 run에서 기존 inherent `run_*_with`로 직접 위임한다. 기존 메서드를
builder 경유로 다시 작성해 mutual recursion을 만들지 않는다. 새 경로의 validation과
acquire timing은 poll 시점으로 문서화한다. 기존 blocking 메서드는 호출 시 plan을 만드는
부분이 있으므로, method invocation과 first poll의 차이를 숨기지 않는다. 어느 경우든
poll 전 permit·ticket·worker 생성은 없어야 한다.

`ValidatedTaskPlan`은 shape validation을 증명한다. executor/class/deadline을 이미 검증한
실행 권한으로 취급하지 않는다. strict ingress 결과는 `into_spec()`로 기존 경로에 연결할
수 있고, 중복 shape 검증 최적화는 측정 후 하나의 shared internal path로만 한다.

## 5. Backend owner와 필요한 구현

| 작업 | owner 파일 / 재사용 symbol | 실제 변경 | engine 상태 변경 |
|---|---|---|---|
| 검증 생성자 | contract `task.rs`, `validation.rs::validate_identifier` | try_new, facade error re-export; Stage standalone 오류 위치는 기존 enum과 호환되는 모델 검토 | 없음 |
| typed submission | host `submission.rs`(신규), `lib.rs`, `runtime.rs` | controls/spec 조립과 run 위임 | 없음 |
| 시간 alias | host `executor/cancel.rs` | 기존 RunFor/CompleteBy에 연결; deadline 정책 지원 검사 유지 | 없음 |
| 설정 공유 | host `runtime.rs::TokioRuntime`, `builder.rs` | config를 Arc로 보관; `config() -> &RuntimeConfig`, Debug, Clone 의미 보존 | 없음 |
| direct 명명 | engine `engine/governor.rs` | declared-plan alias가 기존 admit/validated/waitable로 위임 | 신규 admission queue 없음 |
| resolved embedding | engine `shared/mod.rs`, `Governor::admit_validated_requirements` | facade에서 필요한 타입 노출; 동일 authority requirement 검증 | 기존 상태 기계 재사용 |
| dispatch resolution | host `execution_plan.rs::ValidatedDispatchPlan::preflight` | 신규 builder도 동일 경로 사용 | 기존 role+physical 원자 예약 유지 |
| lifecycle | host `runtime.rs::{TicketGuard, ExecutionLease, run_detached_job, await_detached}`, `runtime/drain.rs` | 새 controls의 취소·종료 wiring; 소유권 복제 금지 | 기존 advance/release/release_leased 유지 |
| observability | host feature 및 emission helper(신규) | transition 뒤 bounded typed event; disabled 비용 최소화 | 기본안은 없음 |
| response-only deadline | §7 host controls/arbiter/worker envelope | absolute caller deadline 지원과 start/response fence | phase·ledger 변경 없이 가능한지 owner proof로 확인 |
| executable flow | 조건 충족 시 facade composition module | bounded coordinator와 실제 reducer | 현재 admission을 호출; workflow state를 engine에 추가하지 않음 |

engine의 주 연결점은 [governor.rs](../../crates/taskmesh-engine/src/engine/governor.rs),
[shared authority types](../../crates/taskmesh-engine/src/shared/mod.rs),
[state/effects](../../crates/taskmesh-engine/src/engine/state.rs)다. 새 기능마다 admission/fairness
정책을 다시 구현하지 않는다. 여러 runtime이 같은 physical executor를 공유하는 별도
capacity-authority 기능은 이 계획의 자동 부수 기능이 아니다.

## 6. Admission부터 종료까지의 통합 계약

```text
spec + controls
  -> ValidatedTaskPlan
  -> ValidatedDispatchPlan::preflight
  -> acquire_execution_lease (arbiter / runtime context)
  -> Governor::admit_validated_requirements
       Rejected -> typed error; worker 없음
       Queued   -> TicketGuard -> claim 또는 abandon
       Admitted -> ExecutionLease::Reserved
  -> dispatch / advance_phase -> LeaseToken custody
  -> 작업·정리 -> release_leased -> settlement wake -> drain의 authoritative 재검사
```

role과 physical 요구량은 preflight에서 같은 policy authority로 해석한다.

| 실제 실행 | role | physical |
|---|---|---|
| caller IO | hint의 기존 ungated 의미 | 없음 |
| local | local_runtime | 없음 |
| pooled blocking | blocking | physical.shared_blocking |
| BackgroundOnly, stack 없음 | maintenance | physical.shared_blocking |
| CPU | cpu | 설치 시 frozen executor domain (기본 shared blocking 또는 Rayon cpu 등) |
| requested-stack sync/async | large_stack | physical.dedicated |

LargeStackCapability hint만 있고 stack 요청이 없는 기존 blocking 호출은 large_stack role과
shared blocking physical domain을 사용한다. hint 이름만으로 dedicated dispatch를 추정하지 않는다.

| 종료 시점 | owner가 해야 할 일 | 관측 oracle |
|---|---|---|
| poll 이전 builder/future drop | payload 해제만 수행 | admission counter·queue·pool 변화 없음 |
| queued drop / 취소 | TicketGuard abandon; promotion 경합도 처리 | 남은 ticket/미청구 permit 없음 |
| admit 뒤 dispatch 전 실패 | Reserved lease 반환 | 작업 미실행, capacity 원복 |
| detached worker 시작 뒤 caller 종료 | worker가 lease 유지 | caller terminal과 inflight가 동시에 존재 가능 |
| 정상 completion | 기존 release fence 뒤 결과 전달 | Ok 관측 시 해당 lease 반환 |
| requested-stack async terminal | caller 응답 뒤 teardown 가능 | teardown 중 custody 유지; 정상 성공은 teardown/release 뒤 |
| drain | close admission, snapshot 전 Notified 준비, settlement 뒤 재검사 | 이후 admission 거부; 기존 queued/inflight만 완료 |

queued/admitted event를 내기 전에 guard를 설치한다. 진단 subscriber panic이 release나
settlement를 건너뛰지 않도록 emission을 격리한다. worker drop/unwind 안에서 진단 때문에
두 번째 panic을 내면 안 된다. correctness-critical waker의 기존 panic discipline과
optional diagnostic failure 정책을 구분한다.

host tracing은 정확한 engine promotion 시각까지 안다고 주장하지 않는다. 예를 들어
`lease_acquired`는 host가 claim한 시점이다. 모든 direct-engine transition event가 실제로
필요해지면 별도 observer port와 `TransitionEffects` 확장을 검토한다. 그 경우 typed data를
lock 안에서 수집하고 callback은 lock 밖에서 실행하며, bounded event 수·순서·유실 계약을
정의한다. optional tracing이 `SettlementWaker`를 대체하지 않는다.

## 7. 신규 host 기능 후보: synchronous response deadline

**D01의 권장 후속안은 `response_by(Instant)`라는 별도 caller-response 계약이다.**
이 기능은 additive SDK 기초 작업과 구분한다. 현재 `CompleteBy` sync 거부와 기존 정책을
그대로 유지한 상태에서 설계·구현·consumer 전환을 하나의 별도 변경으로 검증한다.

### 7.1 의미와 API 경계

- 의미: admission/dispatch 대기 및 caller 결과 수신을 하나의 absolute boundary로 제한한다.
- 이미 시작한 sync 작업은 계속 실행할 수 있고 lease도 유지한다. 강제 종료 보장이 아니다.
- CPU/blocking builder에만 먼저 제공하고, 필요하면 private 필드의 새 host controls type으로 기존 spec 경로에 연결한다.
- 기존 `SubmitOptions`에 public 필드를 추가하거나 exhaustive `SubmissionDeadline`에 variant를 추가하지 않는다.
- 초기 버전은 class의 `CooperativeWithDeadline` opt-in을 요구한다. consumer public query class의 정책 변경은 별도 migration으로 기록한다.
- builder 내부 deadline mode는 한 종류다. `run_for`/`response_by`를 연속 설정하면 마지막 값이 선택된다.
  future public raw controls도 conflicting authority를 허용하지 않도록 설계한다.
- caller가 already-expired boundary를 주면 worker를 만들지 않고 typed deadline 결과를 반환한다.

### 7.2 구현 연결

1. private host controls를 기존 options로부터 정규화하는 공통 함수를 둔다. 기존 호출의 검증 우선순위와 동작을 유지한다.
2. `AcquisitionArbiter`가 response boundary도 기다림의 상한으로 받도록 일반화한다.
   cancel → absolute boundary → relative acquire timeout의 precedence를 명시한다.
   이를 CompleteBy로 가장해 sync 지원 검사를 우회하지 않는다.
3. `acquire_execution_lease`의 최종 deadline 재검사로 늦은 grant를 미실행 상태에서 반환한다.
   executor 내부 대기는 별도 host queue를 만들지 않고 기존 dispatch envelope가 담당한다.
4. `run_detached_job`에서 user closure를 호출하기 직전에 boundary와 caller-interest를 검사한다.
   이미 지나간 요청은 부작용을 시작하지 않고 lease를 정리한다. clock check와 함수 호출 사이의
   OS preemption까지 hard real-time으로 막는다는 보장은 하지 않는다.
5. `await_detached`는 worker-start 통지를 기다리지 않고 absolute timer를 즉시 감시한다.
   timeout이면 result receiver를 종료하고 worker가 custody를 정리하도록 기존 ownership을 유지한다.
6. 성공/error 수신 시 caller 응답 경계를 다시 검사한다. worker가 일찍 끝났더라도 늦게 관측한
   성공을 boundary 이후 전달하지 않는다. 정상 결과의 release fence도 보존한다.
7. worker envelope·진단·panic 처리 전 경로에서 exact-once release를 확인한다. Governor의
   capacity counter를 deadline 처리 코드에서 직접 수정하지 않는다.

outer `tokio::time::timeout`만 붙이는 형태는 consumer 임시 response wait 제한으로는
쓸 수 있지만, executor에 제출된 미시작 job의 시작 방지까지 자동 보장하지 않는다.
따라서 위 통합 계약을 갖춘 SDK 기능과 동등한 해결로 기록하지 않는다.

### 7.3 필수 oracle

expired-before-submit, queue→promotion 경합, admit→executor-start 경합, executor 내부
장기 대기, started worker 중 timeout, timer와 completion 동시 readiness, caller drop,
worker panic, drain timeout 뒤 실제 settlement를 각각 확인한다. 관측값은 caller error,
user closure start 횟수, outstanding capacity, 후속 admission 가능 여부다.
지원하지 않는 기존 CompleteBy sync 요청의 negative oracle도 유지한다.

## 8. 통합 순서와 변경 단위

| 변경 단위 | 선행 조건 | 통합 결과 / 검증 anchor |
|---|---|---|
| S1 typed surface | 현재 facade 반환 타입 목록 | try_new + error exports; consumer가 signature와 variant를 facade로 이름 붙임 |
| S2 config sharing | config 읽기/생성 call site 조사 | Arc config; config getter와 runtime clone의 Governor/drain 동일성 |
| S3 submission facade | S1 | submission.rs + spec bridge; U01–U09의 old/new 행동 비교 |
| S4 direct naming | direct embedder inventory | 기존 admission delegate alias; U10–U11 변함없음 |
| S5 host observation | lifecycle owner inventory | guard 설치 뒤 emission; feature off/on과 drop/panic custody |
| S6 deadline extension | D01 수요·의미 확정, S3 | §7 구현 및 consumer policy/wrapper 이전; 기존 sync CompleteBy 거부 유지 |
| S7 consumer cutover | S1–S4, deadline 사용은 S6 | 고정된 Taskmesh/Semantica pair의 실제 adapter 비교 |
| 후속 flow | 두 pipeline와 자원/결과 계약 | host composition과 bounded fan-out; engine workflow 상태 없음 |

S2와 S4는 소유 파일이 겹치지 않는 범위에서 독립 작업할 수 있다. `runtime.rs`의 S2/S3/S5/S6
통합은 순차로 수행한다. semantic commit 단위로 source/consumer 영향과 rollback을 기록한다.
이 표는 작업 분할 계획이며 이번 문서 작업에서 sub-agent나 구현 작업을 시작했다는 뜻이 아니다.

## 9. Acceptance와 증거 범위

| 검증 | 기존 owner anchor | 추가 확인 |
|---|---|---|
| facade / MSRV / features | `tools/consumer-msrv`, `hardening_consumer_surface.rs` | 새 builder, 반환 오류 타입, Send spawn / local non-Send, default/rayon와 tracing 조합 |
| dispatch | `hardening_dispatch_resolution.rs`, `hardening_role_pool_isolation.rs` | U04–U06의 role/physical 변화 없음, mismatch가 worker 이전 거부 |
| deadline | `deadline_cancel.rs`, `hardening_deadline_custody.rs` | 지원표와 S6 acceptance; deadline policy 거부를 성공으로 오인하지 않음 |
| lifecycle | `hardening_drain.rs`, `runtime_cancel_leak.rs` | builder/drop, promotion 경합, direct release wake, clone 공유 |
| identity / metadata | `hardening_root_child_scope.rs`, engine `identity_authority.rs` | 모든 spec 필드 보존, 동시 root identity, child root/parent 유지 |
| input | `strict_ingress.rs` | JSON 경로가 raw DTO로 후퇴하지 않음; private builder가 ingress 검증 대체하지 않음 |
| external consumer | Semantica governance adapter / `taskmesh_query_async_support_contract` 및 deadline owner tests | 기존 두 smoke 사례만으로 D01 이전 완료 처리 금지 |
| cost | clone/submit/snapshot owner benchmark | class 수별 clone allocation, old/new submission allocation·latency, tracing off 비용 |

관련 테스트 파일은 [host tests](../../crates/taskmesh/tests)와
[engine tests](../../crates/taskmesh-engine/tests)에 있다. fixture 존재는 PASS 증거가 아니다.
문서 편집 단계에서는 링크·소스 symbol·상태 일관성만 확인한다. 구현 시 owner checks와
외부 consumer compile/run 후, 소스가 고정되면 ordinary CI를 한 번 통합한다. mutation과
release qualification은 별도 현재 요청으로 승인된 경우에만 수행한다.

완료 기록은 `change / Taskmesh SHA / consumer SHA 또는 N/A / feature-target-toolchain /
실행 명령·종료 상태 / 로그 위치 / 남은 activation`을 포함한다. 사용자 작업이 섞인
checkout의 부분 검증, 과거 receipt, 단순 정적 mapping을 최종 qualification으로 승격하지 않는다.

## 10. Rollback과 미결정 경계

- S1–S3 rollback: 같은 원본 spec/options/job를 기존 run 메서드로 전달한다. identity를 재발급하지 않는다.
- S4 rollback: alias 사용을 이전 이름으로 되돌린다. reservation semantics 변경이 없어야 한다.
- S5 rollback: tracing feature를 끈다. correctness waker나 accounting은 기능 상태에 의존하지 않는다.
- S6 rollback: caller가 새 response deadline을 요구하면 명시적으로 미지원 처리한다. unsupported
  CompleteBy로 바꾸거나 deadline을 누락한 채 계속 실행하는 fallback은 금지한다.
- 새 executor 종류, runtime 간 공유 capacity authority, engine parent-plan registry, hot policy reload,
  durable workflow/retry는 별도 요구와 ADR 없이는 추가하지 않는다.
- consumer class 비용·실제 물리 pool·요청 크기는 측정 대상이다. migration 편의를 위해
  unknown class default-admit, unbounded 대기열, global duplicate-ID 우회 등을 도입하지 않는다.
