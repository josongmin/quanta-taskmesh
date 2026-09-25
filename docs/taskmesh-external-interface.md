# Taskmesh External Interface

```rust
use taskmesh::{Builder, CancellationPolicy, ClassPolicy, ResourceBudget, Runtime, TaskClass, TaskSpec, TopologyConfig};
```

기본 사용법:

```rust
let runtime = Builder::new()
    .topology(TopologyConfig::new().cpu_auto().blocking_threads(8))
    .resources(ResourceBudget::new().cpu_units(64).memory_units(256))
    .class_policy(
        TaskClass::new("retrieval"),
        ClassPolicy::new()
            .max_inflight(32)
            .max_queue_depth(128)
            // 아래 5번의 mid-run cancel·deadline은 이 정책이 있어야 발효된다.
            .cancellation_policy(CancellationPolicy::CooperativeWithDeadline),
    )
    .build()?;

let spec = TaskSpec::blocking(TaskClass::new("retrieval")).operation("search:repo:123");
let out = runtime
    .run_blocking(spec, || Ok::<_, std::convert::Infallible>("ok"))
    .await?;
```

호출 규칙:

1. async IO -> `run_io`
2. blocking syscall/FFI -> `run_blocking`
3. CPU-heavy compute -> `run_cpu`
4. `!Send`/thread-affinity/current-thread 예외 -> `run_local`
5. 명시적 large-stack async root -> `run_async_with_requested_stack`

고급 규칙:

1. `TaskSpec`의 첫 stage만 한 번의 `run_*` 호출에서 실행된다. `.stage(...)`는 후속 작업을
   선언하지만 host가 자동 순회하거나 용량을 선점하지 않는다. 후속 작업은 caller/adapter가
   별도 `run_*` 호출로 제출하고 새 permit을 받아야 한다. `.reduce_stage(...)`는 fan-out과
   결정적 reduce **정책을 선언**한다. validation은 정책의 존재와 형태를 검사할 뿐, branch를
   생성하거나 개수를 제한하거나 결과를 합치지 않는다. 그 실행과 reducer는 caller/adapter 소유다.
   direct `Governor::{admit, admit_waitable, admit_validated}`는 모든 선언 stage hint의
   **서로 다른 capability pool**을 permit 하나에 예약한다. 명시적 resolved admission 메서드는
   caller가 제공한 requirement를 사용한다. Tokio host `run_*`는 실제 첫 dispatch가 점유하는
   role pool과 physical domain만 예약한다. 같은 multi-stage spec도 진입점에 따라 예약 범위가 다르다.
2. `child_of(root_operation_id, parent_operation_id, parent_stage)`는 정확한 immediate parent와
   root attribution을 선언한다. parent가 결과를 기다리면 같은 3개 인자의
   `awaited_child_of(...)`를 사용한다. 연속된 `parent_awaits` 선언의 exact ancestor chain이
   전부 쥔 capacity의 cycle은 거절된다. enqueue/grant에서 관찰된 live parent permit generation을
   고정하므로 release 뒤 같은 operation name을 재사용해도 기존 ancestry를 바꾸지 않는다.
   결과는 `AdmissionVerdict::NestedWaitCycle { held_by_root: HeldCapacity }`이다.
   stranger/sibling blocker는 false cycle이 아니며, queued request는 promotion 때 재판정된다.
   기존 2인자 child builder는 source incompatible이다. legacy child JSON에
   `parent_operation_id`가 없으면 모호하므로 decode를 거부한다.
3. unknown class는 `AdmissionVerdict::UnknownClass`로 reject
4. CPU executor 교체: `Builder::cpu_executor(Arc::new(taskmesh::ext::RayonCpuExecutor::try_from_topology(&topo)?))`
   (`features = ["rayon"]`; 직접 `taskmesh-rayon` 의존 불필요). `try_new`/`try_from_topology`는
   zero workers, invalid topology, pool construction 실패를 typed `RayonBuildError`로 반환한다.
   구 `new`/`from_topology`는 deprecated panic 경로다. adapter의 worker count·physical domain·
   nonblocking submit 선언이 실제 executor와 다르면 `Builder::build()`가 typed reject한다.
5. cancel/timeout: `run_*_with(spec, SubmitOptions::unbounded().with_cancel(token).with_acquire_timeout(d).with_deadline(d), ...)`
   - pre-submit cancel는 모든 클래스에서 honored.
   - **mid-run 협조 취소**는 `cancellation_policy`가 `Cooperative`/`CooperativeWithDeadline`인 클래스에서
     `run_io`/`run_local`/`run_cpu`/`run_blocking` 모두 동작(→`GovernorError::Cancelled`; 동기 경로에서는
     caller의 대기만 끝나고 시작된 작업은 charged 상태로 끝까지 실행된다). `run_cpu`는 토큰이 stalled
     `CpuExecutor`에서도 caller wait를 탈출시키되, queued/running worker closure가 종료되거나 drop될 때까지
     permit/gate를 보유한다.
   - **run deadline**(`with_deadline`)은 `CooperativeWithDeadline` 클래스에서만 발효(→`GovernorError::DeadlineExceeded`).
     다른 정책의 클래스에 `deadline`을 주면 제출 시점에 `GovernorError::DeadlineUnsupported { class, policy }`로
     거절된다 — 집행할 수 없는 상한을 조용히 버리고 `Ok`를 돌려주지 않는다.
   - worker 실패는 `WorkerPanicked { context }`, `WorkerUnavailable { context, detail }`(worker를 만들지 못함;
     작업 미시작), `JobAbandoned { context }`(adapter가 closure를 실행하지 않음)로 구분된다.
     `LeaseReclaimed { permit_id }`는 dispatch 전에 sweep이 lease를 회수한 경우다(작업 미시작).
   - **absolute deadline**(`with_absolute_deadline`)은 같은 클래스에서 substrate wait, governor admission,
     execution 전체를 하나의 `Instant`로 제한한다. 상대 run budget으로 변환하거나 단계별로 재시작하지 않는다.
     cooperative poll 경계를 소유하는 `run_io`, `run_local`, requested-stack async에서만 허용되며,
     동기 blocking/CPU 경로는 permit 조기 반환을 막기 위해 fail-closed로 거부한다.
   - `acquire_timeout`은 admission lock 대기를 포함한 **모든** 획득 대기를 하나의 checked budget으로
     bound한다. 만료 후 즉시 승인된 permit은 unwind되고 `PermitAcquireTimedOut`이 반환된다.
     capability pool 때문에 큐에 들어갔다 만료된 요청은 `SubstratePoolTimedOut`으로 원인을 구분해 보고한다.
     `Duration::ZERO`는 상대 acquire budget의 "대기하지 말고 시도"다. 이미 만료된 absolute
     CompleteBy는 ZERO로 가려지지 않는다. 경합 시 우선순위는 cancel → absolute deadline
     → relative timeout이며 equality는 expired다. permit handoff 직전에 같은 arbiter를 재검사하고
     거절된 permit은 work closure 생성 없이 반환한다.
   - `RunFor`는 worker 자신의 시작 시각 기준이다(inline executor 포함). 완료 시각이 budget을 넘긴 결과는
     `Ok`가 아니라 `DeadlineExceeded`다. `run_blocking`의 `RunFor`는 caller 대기만 제한하며 시작된 동기
     작업을 중단하지 않는다.
   - **응답과 custody는 다른 사건이다.** caller future drop·deadline·cancel은 caller의 대기를 끝낼 뿐이며,
     동기 blocking/requested-stack/CPU worker는 실제 종료 시점(소유 runtime teardown 포함)까지 permit과
     capability slot을 보유한다. 그동안 snapshot에는 `running`/`cleanup_pending`으로 남는다. 정상 완료
     시에는 결과와 함께 custody가 이동하므로 caller가 값을 관측하는 시점에 용량은 이미 반환되어 있다.
   - 큐에서 대기하던 요청이 claim 전에 끝나면(leak sweep 회수 등) `GovernorError::TicketClaimTerminated
     { ticket, reason }`로 사유가 보존된다. 알 수 없는 ticket은 `InvalidTicketClaim`이며 둘 다 대기가 아니다.
6. substrate hint는 run path와 일치해야 한다(불일치 → `SubstrateMismatch`). requested-stack 크기는
   모든 sync/async 경로에서 admission 전에 단일 validator로 판정된다. topology slot은 실제 동시성
   상한(`0`=무제한)이며 capability 점유는 class inflight·resource budget과 **같은** admission 결정이다:
   admission 앞에 별도 대기 큐가 없으므로, 풀이 가득 차면 비-queueing 클래스는 즉시 `SubstrateSaturated`로
   shed되고 queueing 클래스는 자기 `max_queue_depth` 안에서 기다린다. `stack_size_bytes`가 있는
   blocking-family 제출은 hint와 무관하게 `large_stack` pool을 소비한다. topology는 `Builder::build`에서
   검증되며(inverted worker window·과대 slot count → `GovernorError::InvalidTopology`) panic하지 않는다.
   SEP-21 host는 `physical.shared_blocking`/`physical.cpu`/`physical.dedicated`에 유한한
   physical domain limit을 설치한다. CPU fallback·blocking·maintenance의 실제 실행도 공유
   domain 상한을 넘지 않으며, snapshot occupancy는 같은 engine admission authority에서 나온다.
7. spec 형태가 구조적으로 깨지면(0-stage, stage별 class 불일치, reduce 누락 fan-out) 입장 자체가 `MalformedTask`로 reject.
8. 불가능한 budget·mixed-tier fairness·잘못된 memory scaling 등은 `Builder::build`에서 fail-closed로 reject (런타임 admit로 미룸 없음).
9. `run_async_with_requested_stack[_with]`는 `SubstrateHint::LargeStackCapability`와
   `TaskSpec::stack_size_bytes(...)`를 모두 요구한다. host가 requested-stack OS thread와 owned
   current-thread Tokio runtime을 만들고 그 안에서 future factory를 호출한다. root future는 `!Send`일 수
   있고 `tokio::spawn` child도 caller runtime으로 이탈하지 않는다. caller future drop은 worker root와
   runtime-owned child를 함께 종료한다.
10. shutdown은 `drain(timeout)` 뒤 handle drop이다 (D17). `TokioRuntime::drain`은 engine의 admission을
    **닫고**(one-way; 이후 유효한 제출은 host 경로와 direct `governor()` 경로 모두
    `Rejected(RuntimeUnavailable)`로 거절 — queue·계상·total 변화 없음; malformed spec이나
    잘못된 explicit capability는 그 검증 오류가 먼저 반환될 수 있다) 모든 클래스의 `inflight == 0 &&
    queued == 0`을 engine gauge에서 기다린다. 이미 queue·admit된 작업은 취소하지 않는다. timeout이 먼저
    오면 `Err(NotDrained { classes: {class → {inflight, queued}}, elapsed })`이며 runtime은 draining으로
    남는다 — 이 보고는 종료 영수증이 아니다: 시작된 blocking 작업은 abort할 수 없으므로(D10) 그 worker는
    여전히 charged다. `is_draining()`은 engine의 `admission_closed()`를 그대로 읽는다. teardown은 여전히
    handle drop뿐이다.

두 단계 실행 예시 (`runtime`은 위 Builder로 생성):

```rust
use taskmesh::{SubstrateHint, TaskStage};

let io_spec = TaskSpec::io(TaskClass::new("retrieval"))
    .operation("fetch:repo:123")
    .stage(TaskStage::new("rank"), SubstrateHint::SharedCpuExecutor);
let bytes = runtime
    .run_io(io_spec, async { Ok::<_, std::convert::Infallible>(vec![1_u8, 2, 3]) })
    .await?;

// 위 stage 선언은 CPU 작업을 실행하거나 예약하지 않는다.
let cpu_spec = TaskSpec::cpu(TaskClass::new("retrieval")).operation("rank:repo:123");
let ranked = runtime
    .run_cpu(cpu_spec, move || Ok::<_, std::convert::Infallible>(bytes.len()))
    .await?;
```

첫 `run_io`는 클래스·자원 budget만 계상한다 (`AsyncIo`에는 별도 capability pool이 없다).
두 번째 `run_cpu`는 새 permit에서 CPU role pool과 실제 executor의 physical domain을
예약한다. 동일한 `io_spec`을 direct `Governor::admit`에 넘기면 선언된 CPU stage hint의
pool을 첫 permit에서도 예약한다.

타입 규칙:

1. governor rejection과 task failure는 `RunError::{Governor, Task}`로 분리되며 다시 flatten되지 않는다.
2. `spawn_blocking`/CPU worker join 실패는 task error가 아니라 governor-side error다.
3. `Snapshot`은 `schema_version = 2`다. held 자원은 `u128`이며 JSON에서는 decimal string으로 직렬화된다.
   phase gauge(`dispatch_reserved`/`accepted`/`running`/`cleanup_pending`)는 `inflight`를 정확히 분할하고,
   `admitted_total == inflight + terminated_total`이다 (`Snapshot::conservation_violation`).
4. `Governor::claim`은 `ClaimOutcome`, `release`는 `ReleaseOutcome::{Released, UnknownPermit, HeldByLease { phase }}`
   (`#[must_use]`), `release_stage_memory`는 `StageReleaseOutcome`, implicit `reconcile_memory`와
   explicit `reconcile_memory_at` 모두 `ReconcileOutcome`을 반환한다. `MeasurementSequence`가
   `u64::MAX`에 닿으면 `EpochExhausted`이고 해당 permit에서는 재시도 불가다. permit release는 여전히
   필요하다. 거부·stale·unknown은 값으로 보고되며 0이나 `None`이나 `()`로 접히지
   않는다. `claim`은 lease 활동으로 간주되어 leak sweep의 staleness 시계를 다시 시작한다.
   `advance_phase`는 `AdvanceOutcome`을 반환한다: `DispatchReserved`를 벗어나는 첫 전진이
   `Leased(LeaseToken)`으로 custody를 넘기고, 그 뒤 dispatch된 permit은 `release_leased(token)`으로만
   끝난다 — `release(id)`는 `HeldByLease`로 거절되고 아무것도 바꾸지 않는다. token은 `Clone`이 아니며
   `#[must_use]`다 (버리면 그 permit은 sweep도 회수하지 않는 영구 charged 상태).
5. `CpuExecutor::capabilities()`(default 구현 제공)로 adapter가 보장하는 것을 선언한다. host는 선언
   이상을 가정하지 않으며, 공유 pool의 ambient 작업을 제한한다고 주장하지 않는다. 선언은 load-bearing이다:
   `declared_workers`가 topology가 resolve한 physical domain worker 수와 다르면 `Builder::build`가
   `TopologyError::ExecutorWorkerCountMismatch`로 거절한다. 검증된 선언은 build 시 동결되며 dispatch,
   debug, `TokioRuntime::executor_capabilities()`가 같은 snapshot을 사용한다. 기본 Tokio blocking/CPU
   dispatch는 permit 발급 전에 active Tokio context를 검사하고 없으면 typed `WorkerUnavailable`을 반환한다.
   `RayonCpuExecutor`는 pool의 실제 thread 수와 exclusivity(`with_pool`은 shared)를 선언한다.
   `BlockingPoolCpuExecutor`는 resolved Taskmesh shared-domain 제출 한도·shared·non-blocking
   submit을 선언하며 ambient Tokio pool 전체 크기를 주장하지 않는다.
6. 클래스 내부는 도착 순서다(D08): runnable한 head가 promotion pass를 기다리는 동안 새 도착은
   `CapacityBlock::QueuedBehind`로 queue된다. 유일한 추월은 head가 다른 capability pool에 막힌 경우다.
7. 동시성 모델 검사(`just loom`/`just shuttle`)는 production `Governor`를 checker의 mutex/atomics 위에
   컴파일해 돌린다(engine `src/sync.rs`, feature `loom`/`shuttle` + 같은 이름의 `--cfg`). replica가 아니다.
