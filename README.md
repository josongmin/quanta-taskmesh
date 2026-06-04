# taskmesh

`taskmesh`는 분석 시스템과 서비스 런타임을 위한 governed execution control-plane이다.

핵심 원칙:

1. semantic policy와 worker governance를 분리한다.
2. topology와 class policy를 분리한다.
3. capability pool만 연다. engine 전용 pool은 금지한다.
4. product-local taxonomy는 adapter layer가 owner한다.
5. fail-closed, deterministic reduce, inventory-first를 기본으로 한다.

패키지 (헥사고날 4-crate):

1. `taskmesh-contract`
   - public wire/domain contract + 모든 port trait (`Runtime`, `CpuExecutor`, `Clock`, `PermitWaker`)
2. `taskmesh-engine`
   - 순수 거버넌스 core: admission/fairness/memory/composite/inventory state machine
3. `taskmesh`
   - Tokio host adapter와 public facade (default CPU executor, cancel/deadline)
4. `taskmesh-rayon`
   - optional 공유 CPU executor 어댑터 (`CpuExecutor` port 구현)

의존 방향은 항상 안쪽(contract/engine)으로만 향한다. tokio와 rayon은 서로를 모르고,
엔진은 tokio를 모른다. 자세한 근거는 [ADR 0001](docs/adr/0001-hexagonal-feature-sliced-architecture.md).

빠른 예시:

```rust
use taskmesh::{Builder, ClassPolicy, ResourceBudget, Runtime, TaskClass, TaskSpec, TopologyConfig};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let runtime = Builder::new()
        .topology(
            TopologyConfig::new()
                .cpu_auto()
                .reserve_cores(1)
                .min_workers(1)
                .max_workers(8)
                .blocking_threads(8),
        )
        .resources(ResourceBudget::new().cpu_units(64).memory_units(256))
        .class_policy(
            TaskClass::new("retrieval"),
            ClassPolicy::new()
                .max_inflight(32)
                .max_queue_depth(128)
                .cpu_units(1)
                .memory_units(2),
        )
        .build()
        .expect("runtime must build");

    let spec = TaskSpec::blocking(TaskClass::new("retrieval")).operation("search:repo:123");
    let out: String = runtime
        .run_blocking(spec, || Ok::<_, std::convert::Infallible>("ok".to_string()))
        .await
        .expect("governor and task must succeed");

    assert_eq!(out, "ok");
}
```

## 외부 사용 가이드 (Rust)

외부 소비자는 `taskmesh` crate 하나만 의존한다. `taskmesh-contract`, `taskmesh-engine`,
`taskmesh-rayon`은 내부 구현이며 직접 의존하지 않는다.

### 1. 의존성 추가

```toml
[dependencies]
# 아직 crates.io 미배포 — path 또는 git 참조
taskmesh = { git = "https://example.com/quanta-taskmesh", features = ["rayon"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }

# 같은 워크스페이스 안에서라면
# taskmesh = { path = "crates/taskmesh", features = ["rayon"] }
```

feature 플래그:

1. `rayon`
   - shared CPU executor 어댑터를 켠다. `run_cpu` 경로의 실제 CPU 풀 구현.
   - 끄면 CPU 작업도 blocking 풀로 폴백한다.

### 2. 런타임 구성

`Builder`로 **worker governance(topology/resources)** 와 **semantic policy(class_policy)** 를
분리해서 선언한다. `build()`는 정책을 검증하며, 불가능한 예산이면 `GovernorError`로 fail-closed.

```rust
use taskmesh::{
    Builder, ClassPolicy, FairnessPolicy, ResourceBudget, TaskClass, TopologyConfig,
};

let runtime = Builder::new()
    // 워커 토폴로지: capability pool만 연다 (engine 전용 pool 금지)
    .topology(
        TopologyConfig::new()
            .cpu_auto()            // 또는 .cpu_fixed(n)
            .reserve_cores(1)
            .min_workers(1)
            .max_workers(8)
            .blocking_threads(8)
            .local_runtime_slots(1),
    )
    // 전역 자원 예산 (unit 단위)
    .resources(
        ResourceBudget::new()
            .cpu_units(64)
            .memory_units(256)
            .per_request_cpu_units(4)
            .per_request_memory_units(16),
    )
    // 클래스별 의미 정책 (taxonomy는 호출자가 소유)
    .class_policy(
        TaskClass::new("retrieval"),
        ClassPolicy::new()
            .max_inflight(32)
            .max_queue_depth(128)
            .cpu_units(1)
            .memory_units(2)
            .fairness(FairnessPolicy::DeficitRoundRobin { quantum: 4 }),
    )
    .build()
    .expect("policy must validate");
```

### 3. 작업 실행

`TaskSpec`으로 클래스/substrate hint를 기술하고, substrate에 맞는 `run_*`를 호출한다.
모든 `run_*`는 admission permit을 잡고 → 실행 → permit을 반납한다. `Runtime` trait이
스코프에 있어야 메서드가 보인다.

```rust
use taskmesh::{Runtime, TaskClass, TaskSpec};

let class = TaskClass::new("retrieval");

// async I/O 경로
let spec = TaskSpec::io(class.clone()).operation("fetch:doc:42");
let body: String = runtime
    .run_io(spec, async { Ok::<_, std::io::Error>(load().await?) })
    .await?;

// blocking 경로 (spawn_blocking 풀)
let spec = TaskSpec::blocking(class.clone()).operation("parse:blob");
let parsed: Doc = runtime
    .run_blocking(spec, || Ok::<_, ParseError>(parse_sync()))
    .await?;

// CPU 경로 (rayon feature 시 shared CPU pool)
let spec = TaskSpec::cpu(class.clone()).operation("rank:candidates");
let ranked: Vec<Hit> = runtime
    .run_cpu(spec, || Ok::<_, std::convert::Infallible>(rank()))
    .await?;

// non-Send 로컬 경로 (LocalSet)
let spec = TaskSpec::base(class, taskmesh::SubstrateHint::LocalRuntime).operation("local:job");
let out = runtime.run_local(spec, async { Ok::<_, MyError>(do_local().await) }).await?;
```

자식 작업은 root에 귀속시켜 attribution을 유지한다:

```rust
use taskmesh::TaskStage;

let child = TaskSpec::cpu(TaskClass::new("retrieval"))
    .child_of("fetch:doc:42", TaskStage::new("rank"))
    .operation("rank:shard:3");
```

### 4. 거부 처리 (fail-closed)

`run_*`는 `Result<T, RunError<E>>`를 돌려준다. 거버너 거부와 작업 에러를 구분한다.

```rust
use taskmesh::{AdmissionVerdict, GovernorError, RunError};

match runtime.run_blocking(spec, job).await {
    Ok(value) => { /* 성공 */ }

    // 거버너가 admission 단계에서 거부
    Err(RunError::Governor(GovernorError::Rejected(verdict))) => match verdict {
        AdmissionVerdict::UnknownClass { class } => { /* 미등록 클래스 — default-admit 안 함 */ }
        AdmissionVerdict::QueueFull { retry_after_ms }
        | AdmissionVerdict::CpuSaturated { retry_after_ms }
        | AdmissionVerdict::MemorySaturated { retry_after_ms }
        | AdmissionVerdict::SubstratePoolTimedOut { retry_after_ms } => {
            // retry_after_ms 만큼 backoff 후 재시도
            let _ = retry_after_ms;
        }
        AdmissionVerdict::SubstrateMismatch => { /* run path ↔ hint 불일치 */ }
        AdmissionVerdict::MalformedTask => { /* 0-stage / stage class 불일치 / reduce 누락 */ }
        other => { /* ClassDisabled, RuntimeUnavailable 등 */ let _ = other; }
    },

    // 작업 자체가 실패
    Err(RunError::Task(e)) => { /* 사용자 에러 e */ let _ = e; }

    Err(other) => { let _ = other; }
}
```

`verdict.retry_after_ms()`로 backpressure 힌트를 바로 꺼낼 수 있다.

### 5. 관측 (inventory-first)

`snapshot()`은 클래스별 inflight/queue/held 자원과 substrate 인벤토리를 결정적으로 반환한다.

```rust
let snap = runtime.snapshot();
for (class, c) in &snap.classes {
    println!("{class}: inflight={} queued={} cpu_held={}", c.inflight, c.queued, c.cpu_units_held);
}
for s in &snap.substrates {
    println!("substrate {} kind={:?} pool={:?}", s.name, s.kind, s.capability_pool);
}
```

### 클래스 정책 옵션 요약

| 메서드 / 필드 | 의미 |
|---|---|
| `max_inflight` | 동시 실행 상한 |
| `max_queue_depth` | 대기 큐 깊이 (초과 시 `overflow_policy`) |
| `fairness` | `Fifo` / `WeightedFairQueue` / `DeficitRoundRobin` / `DeadlineAware` / `BestEffortScavenger` |
| `retry_after_policy` | `None` / `FixedMs(ms)` / `Adaptive` — 거부 시 backoff 힌트 |
| `overflow_policy` | `Reject` / `QueueWithinDepth` / `DropBestEffort`(현재 `Reject`와 동치) |
| `memory_overcommit_policy` | `Reject` / `Queue` / `DegradeToLight { fallback_class }` |
| `memory_release_policy` | `OnTaskCompletion` / `OnStageBoundary` / `LeakDetecting`(leak sweep 대상) |
| `cancellation_policy` | `PreSubmitOnly` / `Cooperative` / `CooperativeWithDeadline` |

## 집행 의미 (enforcement semantics)

선언된 contract는 런타임에서 실제로 집행된다 (선언만 하는 inert knob 아님):

1. **substrate 분류는 권위적이다.** 각 `run_*`는 spec의 substrate hint와 일치해야 한다.
   `run_io`=`AsyncIo`, `run_blocking`=`BlockingPool`/`LargeStackCapability`/`BackgroundOnly`,
   `run_cpu`=`SharedCpuExecutor`, `run_local`=`LocalRuntime`. 불일치는 `SubstrateMismatch`로 reject.
2. **topology slot은 실제 capability-pool 상한이다.** `blocking_threads`/`large_stack_slots`/
   `local_runtime_slots`/`maintenance_workers` 및 topology-sized CPU 풀은 해당 substrate 동시성을
   제한한다. `0 = 무제한`. 풀이 가득 차면 `SubstratePoolTimedOut`로 backpressure(거버너 입장 큐 대기
   `PermitAcquireTimedOut`와 구분). `Blocking`/`LargeStack`/`Background`는 blocking executor를 공유하되
   capability pool은 별도다.
3. **fan-out reduce는 admission에서 강제된다.** reduce policy 없는 fan-out stage는 `MalformedTask`.
   0-stage spec과 stage별 class 불일치도 `MalformedTask`로 reject.
4. **fairness는 tier 단위 discipline이다.** 한 tier(best-effort 여부) 안의 클래스는 같은 discipline을
   써야 하며, 혼합 시 `build()`가 거부한다. (weight/quantum/slack 등 파라미터는 클래스별로 달라도 됨.)
   `DeficitRoundRobin`은 deficit 커서로 quantum에 비례해 라운드로빈한다.
5. **cancellation은 정책을 따른다.** `Cooperative`/`CooperativeWithDeadline` 클래스는
   `run_io`/`run_local`/`run_cpu` 모두에서 토큰으로 mid-run 취소(→`GovernorError::Cancelled`)된다
   (`run_cpu`는 stalled `CpuExecutor`에서도 탈출). `CooperativeWithDeadline`는 추가로
   `SubmitOptions::deadline`을 발효(→`GovernorError::DeadlineExceeded`); plain `Cooperative`는 무시.
6. **`LeakDetecting` 클래스만 leak sweep으로 회수**되며, downward `reconcile_memory`는 예산을 풀고
   대기 작업을 promote한다.
7. **정책 검증은 fail-closed다.** 불가능한 budget·mixed-tier fairness·잘못된 memory scaling 등은
   `Builder::build`/`Governor::new`에서 거부되며 런타임 admit로 미루지 않는다.
8. **child→root 귀속:** `child_of(root).operation(name)`는 root를 유지한다 (operation은 root를
   재설정하지 않음).
8. `checkpoint_policy`는 host가 hook point에서 inspect하는 **메타데이터**다(엔진 강제 아님).

문서:

1. [RFC](docs/rfcs/0001-governed-runtime.md)
2. [Library Spec](docs/taskmesh-library-spec.md)
3. [External Interface](docs/taskmesh-external-interface.md)
