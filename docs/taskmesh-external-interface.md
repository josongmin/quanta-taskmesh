# Taskmesh External Interface

```rust
use taskmesh::{Builder, ClassPolicy, ResourceBudget, Runtime, TaskClass, TaskSpec, TopologyConfig};
```

기본 사용법:

```rust
let runtime = Builder::new()
    .topology(TopologyConfig::new().cpu_auto().blocking_threads(8))
    .resources(ResourceBudget::new().cpu_units(64).memory_units(256))
    .class_policy(
        TaskClass::new("retrieval"),
        ClassPolicy::new().max_inflight(32).max_queue_depth(128),
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
4. CPU executor 교체: `Builder::cpu_executor(Arc::new(taskmesh_rayon::RayonCpuExecutor::from_topology(&topo)))`
5. cancel/timeout: `run_*_with(spec, SubmitOptions::unbounded().with_cancel(token).with_acquire_timeout(d).with_deadline(d), ...)`
   - pre-submit cancel는 모든 클래스에서 honored.
   - **mid-run 협조 취소**는 `cancellation_policy`가 `Cooperative`/`CooperativeWithDeadline`인 클래스에서
     `run_io`/`run_local`/`run_cpu` 모두 동작(→`GovernorError::Cancelled`). `run_cpu`는 토큰이 stalled
     `CpuExecutor`에서도 탈출시킨다(permit/gate는 정확히 한 번 release).
   - **run deadline**(`with_deadline`)은 `CooperativeWithDeadline` 클래스에서만 발효(→`GovernorError::DeadlineExceeded`);
     plain `Cooperative`는 `deadline`을 무시한다.
   - **absolute deadline**(`with_absolute_deadline`)은 같은 클래스에서 substrate wait, governor admission,
     execution 전체를 하나의 `Instant`로 제한한다. 상대 run budget으로 변환하거나 단계별로 재시작하지 않는다.
     cooperative poll 경계를 소유하는 `run_io`, `run_local`, requested-stack async에서만 허용되며,
     동기 blocking/CPU 경로는 permit 조기 반환을 막기 위해 fail-closed로 거부한다.
   - `acquire_timeout`은 governor 입장 큐 대기(→`PermitAcquireTimedOut`)와 substrate capability-pool 슬롯 대기
     (→`SubstratePoolTimedOut`) 둘 다를 bound한다.
6. substrate hint는 run path와 일치해야 한다(불일치 → `SubstrateMismatch`). topology slot은 실제 동시성 상한(`0`=무제한);
   슬롯이 가득 차면 `SubstratePoolTimedOut`로 backpressure.
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
