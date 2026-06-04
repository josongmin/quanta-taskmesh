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

고급 규칙:

1. `.stage(...)`는 adapter/runtime owner용; `.reduce_stage(...)`는 fan-out + 결정적 reduce용
2. `child_of(...)`는 composite permit attribution용 (root에 귀속)
3. unknown class는 `AdmissionVerdict::UnknownClass`로 reject
4. CPU executor 교체: `Builder::cpu_executor(Arc::new(taskmesh_rayon::RayonCpuExecutor::from_topology(&topo)))`
5. cancel/timeout: `run_*_with(spec, SubmitOptions::unbounded().with_cancel(token).with_acquire_timeout(d), ...)`
   - pre-submit cancel는 모든 클래스에서 honored. **mid-run 협조 취소**는 `cancellation_policy`가
     `Cooperative`/`CooperativeWithDeadline`인 클래스의 `run_io`에서만 동작(→`GovernorError::Cancelled`).
     동기 `run_blocking`/`run_cpu`는 pre-submit only.
6. substrate hint는 run path와 일치해야 한다(불일치 → `MalformedTask`). topology slot은 실제 동시성 상한(`0`=무제한).

타입 규칙:

1. governor rejection과 task failure는 `RunError::{Governor, Task}`로 분리되며 다시 flatten되지 않는다.
2. `spawn_blocking`/CPU worker join 실패는 task error가 아니라 governor-side error다.
