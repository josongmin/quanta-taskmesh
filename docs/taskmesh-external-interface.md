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

1. `.stage(...)`는 adapter/runtime owner용; `.reduce_stage(...)`는 fan-out + 결정적 reduce용
2. `child_of(...)`는 composite permit attribution용 (root에 귀속)
3. unknown class는 `AdmissionVerdict::UnknownClass`로 reject
4. CPU executor 교체: `Builder::cpu_executor(Arc::new(taskmesh::ext::RayonCpuExecutor::from_topology(&topo)))`
   (`features = ["rayon"]`; 직접 `taskmesh-rayon` 의존 불필요). adapter가 `declared_workers`를 topology의
   `cpu` gate보다 작게 선언하면 `build()`가 `ExecutorDeclaresFewerWorkers`로 거절한다.
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
     `Duration::ZERO`는 "대기하지 말고 시도"다.
   - `RunFor`는 worker 자신의 시작 시각 기준이다(inline executor 포함). 완료 시각이 budget을 넘긴 결과는
     `Ok`가 아니라 `DeadlineExceeded`다. `run_blocking`의 `RunFor`는 caller 대기만 제한하며 시작된 동기
     작업을 중단하지 않는다.
   - **응답과 custody는 다른 사건이다.** caller future drop·deadline·cancel은 caller의 대기를 끝낼 뿐이며,
     동기 blocking/requested-stack/CPU worker는 실제 종료 시점(소유 runtime teardown 포함)까지 permit과
     capability slot을 보유한다. 그동안 snapshot에는 `running`/`cleanup_pending`으로 남는다. 정상 완료
     시에는 결과와 함께 custody가 이동하므로 caller가 값을 관측하는 시점에 용량은 이미 반환되어 있다.
   - 큐에서 대기하던 요청이 claim 전에 끝나면(leak sweep 회수 등) `GovernorError::TicketClaimTerminated
     { ticket, reason }`로 사유가 보존된다. 알 수 없는 ticket은 `InvalidTicketClaim`이며 둘 다 대기가 아니다.
6. substrate hint는 run path와 일치해야 한다(불일치 → `SubstrateMismatch`). topology slot은 실제 동시성
   상한(`0`=무제한)이며 capability 점유는 class inflight·resource budget과 **같은** admission 결정이다:
   admission 앞에 별도 대기 큐가 없으므로, 풀이 가득 차면 비-queueing 클래스는 즉시 `SubstrateSaturated`로
   shed되고 queueing 클래스는 자기 `max_queue_depth` 안에서 기다린다. `stack_size_bytes`가 있는
   blocking-family 제출은 hint와 무관하게 `large_stack` pool을 소비한다. topology는 `Builder::build`에서
   검증되며(inverted worker window·과대 slot count → `GovernorError::InvalidTopology`) panic하지 않는다.
7. spec 형태가 구조적으로 깨지면(0-stage, stage별 class 불일치, reduce 누락 fan-out) 입장 자체가 `MalformedTask`로 reject.
8. 불가능한 budget·mixed-tier fairness·잘못된 memory scaling 등은 `Builder::build`에서 fail-closed로 reject (런타임 admit로 미룸 없음).
9. `run_async_with_requested_stack[_with]`는 `SubstrateHint::LargeStackCapability`와
   `TaskSpec::stack_size_bytes(...)`를 모두 요구한다. host가 requested-stack OS thread와 owned
   current-thread Tokio runtime을 만들고 그 안에서 future factory를 호출한다. root future는 `!Send`일 수
   있고 `tokio::spawn` child도 caller runtime으로 이탈하지 않는다. caller future drop은 worker root와
   runtime-owned child를 함께 종료한다.

타입 규칙:

1. governor rejection과 task failure는 `RunError::{Governor, Task}`로 분리되며 다시 flatten되지 않는다.
2. `spawn_blocking`/CPU worker join 실패는 task error가 아니라 governor-side error다.
3. `Snapshot`은 `schema_version = 2`다. held 자원은 `u128`이며 JSON에서는 decimal string으로 직렬화된다.
   phase gauge(`dispatch_reserved`/`accepted`/`running`/`cleanup_pending`)는 `inflight`를 정확히 분할하고,
   `admitted_total == inflight + terminated_total`이다 (`Snapshot::conservation_violation`).
4. `Governor::claim`은 `ClaimOutcome`, `release`는 `ReleaseOutcome::{Released, UnknownPermit}`
   (`#[must_use]`), `release_stage_memory`는 `StageReleaseOutcome`, `reconcile_memory_at`은
   `ReconcileOutcome`을 반환한다. 거부·stale·unknown은 값으로 보고되며 0이나 `None`이나 `()`로 접히지
   않는다. `claim`은 lease 활동으로 간주되어 leak sweep의 staleness 시계를 다시 시작한다.
5. `CpuExecutor::capabilities()`(default 구현 제공)로 adapter가 보장하는 것을 선언한다. host는 선언
   이상을 가정하지 않으며, 공유 pool의 ambient 작업을 제한한다고 주장하지 않는다. 선언은 load-bearing이다:
   `declared_workers`가 topology의 `cpu` gate보다 작으면 `Builder::build`가
   `TopologyError::ExecutorDeclaresFewerWorkers`로 거절하고, `TokioRuntime::executor_capabilities()`가
   선언을 노출한다. `RayonCpuExecutor`는 pool의 실제 thread 수와 exclusivity(`with_pool`은 shared)를,
   `BlockingPoolCpuExecutor`는 worker 수 unknown·shared·non-blocking submit을 선언한다.
6. 클래스 내부는 도착 순서다(D08): runnable한 head가 promotion pass를 기다리는 동안 새 도착은
   `CapacityBlock::QueuedBehind`로 queue된다. 유일한 추월은 head가 다른 capability pool에 막힌 경우다.
7. 동시성 모델 검사(`just loom`/`just shuttle`)는 production `Governor`를 checker의 mutex/atomics 위에
   컴파일해 돌린다(engine `src/sync.rs`, feature `loom`/`shuttle` + 같은 이름의 `--cfg`). replica가 아니다.
