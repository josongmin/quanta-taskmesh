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

## 검증 표면 (proof surface)

macOS 기본 피드백 진입점은 `just dev`이다. core workspace(Rust benchmark harness와
generated doc fixture 제외),
production `lib/bin` Clippy, 저비용 architecture/gate owner tests와 실제 정적 정책만 실행한다.
Semgrep은 실제 소스를 한 번 스캔하고, test/example/bench Clippy 및 43-case synthetic rule-pack
회귀팩은 CI 프로필에 남긴다. dependency checks와 feature/doc/MSRV matrix는
`just verify-macos-ci`에 둔다. curated/generated mutation, sanitizer/fuzzer,
model exploration, coverage, Linux IAI는 명시적 `nightly` 프로필
(`just verify-macos-nightly`)에서만 실행한다. `just release`는 `ci`와 `nightly`를 합친
cross-platform required set과 정확히 같은 집합으로 expand되어야 하며 `just gates-inventory`가 이를
강제한다. 증명은 세 층으로 겹친다:

두 macOS 진입점의 Cargo compile concurrency는 기본 4이며 `TASKMESH_BUILD_JOBS`로, test
concurrency는 `TASKMESH_TEST_JOBS`로 조정한다. nextest가 없을 때의 Cargo fallback도 같은 test
thread 상한을 사용한다.

| 층 | 무엇을 | 무엇으로 |
|---|---|---|
| 모델 검사 | production `Governor`의 모든 interleaving(작은 상태) / 무작위 schedule, D08 promotion-gap 규칙 포함 | `just modelcheck` (Loom 5 / Shuttle 7 + replay) — engine `src/sync.rs` seam, replica 아님. `just loom`/`just shuttle`은 focused debug용 |
| 실행 가능한 명세 | admission/promotion 상태기계 ≡ ~150줄 reference model (매 op 뒤 verdict·gauge·ticket·ledger 동치) | `cargo test -p taskmesh-engine --test differential_model` (proptest, `just test`에 포함) |
| 실제 메모리 시스템 | parking_lot·Tokio·OS thread 위의 data race | `just tsan` (nightly `-Zbuild-std -Zsanitizer=thread`) |
| coverage-guided 탐색 | production `Governor`의 모든 public transition을 무작위 순서로 몰며 매 step 뒤 invariant(conservation·상한·budget·handle 일치)와 마지막 quiescence; policy/topology/builder front door; JSON wire format | `just fuzz` (nightly + cargo-fuzz, `fuzz/`; target은 `just fuzz-check`로 stable에서 항상 컴파일) |
| 테스트가 실제로 실패할 수 있는가 | 고친 결함 하나를 다시 넣으면 named test가 named reason으로 죽는가 | `just mutants-critical` (curated single-edit inventory: 102 defect probes + 1 control; cargo-mutants score 아님) |
| 객관 지표 | 실행된 production code (threshold 아님) | `just coverage-report` (lines/regions/functions/instantiations; branch·MCDC 미수집 시 `NOT_COLLECTED`) |
| 소비자 계약 | Rust 1.81에서 default·rayon 표면 컴파일 | `just consumer-msrv` |
| 문서가 컴파일되는가 | 이 README·`docs/taskmesh-external-interface.md`·`CHANGELOG.md`의 모든 ```rust 블록이 *그대로* facade에 대해 type-check (build.rs가 추출; hidden line 없음; fence attribute `body`/`arms`/`builder`로 scaffold 선택, `rust,ignore`는 CHANGELOG의 `// 0.1.0` 인용에만 허용) | `cargo test -p taskmesh-doc-examples` (`just test`에 포함), `just doctest` (workspace rustdoc examples) |
| 성능 | admit→release allocs/op(=현재 기준 8.0, 무여유), Linux instruction count | `just bench-gate`, `just bench-iai` |

clean checkout의 exact `HEAD`/tree/digest 전후가 같은 macOS CI receipt는
`just verify-macos-ci`가 `target/verification/macos-gates.json`에 남긴다. Linux 전용 `bench-iai`는
nightly receipt에서 `SKIPPED_PLATFORM`으로 기록된다. `just install-hooks`는 이
CI receipt가 현재 push SHA와 일치하지 않으면 branch push를 거부한다(`qualify-local` release receipt도
허용). nightly는 별도 `just verify-macos-nightly`로 실행하며 일반 CI/push 증거에 필요하지 않다.
전체 cross-platform/release 자격은 mutation을 포함한 local Linux `just qualify-local`, release 판정은
`just release-local`로 수행한다. GitHub workflow 세 개는
호환성 기록으로만 남기고 repository 설정에서 비활성화한다. `docs/release-checklist.md` 참조.
전체 gate runner는 첫 non-PASS에서 기본 fail-fast하며, 남은 applicable gate를
`NOT_RUN`/`blocked_by`로 기록해 분모를 보존한다. 실패 뒤의 전체 진단이 필요한 경우에만
`tools/gates/run.py ... --keep-going`을 사용한다.
정식 release collector의 기본 선택과 실행 순서는 `tools/gates/required.json` 단일 권위이며,
cheap/static blocker 뒤에 고비용 proof producer를 배치한다. 명시적 `--tier`는 진단 subset이다.
`--required`/`--all`과 정식 qualification collector는 dirty source를 실행 전에 거부하므로,
자격 취득이 불가능한 상태에서 mutation/fuzz 등 고비용 작업을 시작하지 않는다.
dirty checkout 전체를 의도적으로 진단해야 할 때만
`--allow-dirty-source --keep-going`을 함께 사용한다. 이 override로 생성된 receipt는 source
상태 때문에 qualification으로 승격되지 않는다. `mutants-generated`는 제품 회귀 테스트가 아니라
현재 테스트망이 compiler-generated 변경 전체를 잡는지 감사하는 final-candidate proof다. 수천 개
변이를 격리 실행하므로 일상 수정이나 focused 재검증에서는 실행하지 않는다.
구현·증명·운영을 함께 재감사할 때는
[Taskmesh SOTA Audit Checklist](docs/taskmesh-sota-audit-checklist.md)의 `M/R/D` 판정과
증거 ledger를 사용한다.

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

**버전 / 호환성.** 네 crate는 하나의 workspace 버전을 공유하며 현재 manifest는 `0.3.0`이다.
SEP-21 변경은 선택된 minor-version 후보지만 human version approval, exact-source release
qualification과 publication이 끝나지 않은 미출시 상태다. `0.2.0` 배포 증거나 `0.3.0`
publication으로 취급하지 않는다.
`0.x`에서는 minor bump(`0.1 → 0.2`)에 breaking change가 포함될 수 있고, 그 전부는
[CHANGELOG.md](CHANGELOG.md)의 *Breaking changes and migration* 절에 before/after 코드와
함께 열거된다 (`Governor::claim → ClaimOutcome`, `Governor::release → ReleaseOutcome`,
`Snapshot` wire schema 2, `DeadlineUnsupported`, `SubstrateSaturated`,
`ExecutorCapabilities` 등). 각 변경의 계약과 근거는
[ADR 0003](docs/adr/0003-sep-16-hardening-contracts.md)(D01–D17)에 있다. 소비자가 의존하는
public surface는 `taskmesh` (+ `taskmesh::ext`)이며, release 후보마다 immutable baseline SHA에
대한 4-crate `just semver-release`, 수동 API/wire/behavior 판정, MSRV(1.81) 소비자 fixture
(`just consumer-msrv`, [tools/consumer-msrv](tools/consumer-msrv/src/main.rs))로 검증한다.
선택적으로 수동 실행한 hosted `QUALIFIED`도 별도의 release receipt가 아니며, 최종 SHA에 묶인 human
API/wire/behavior adjudication과 release receipt는 별도로 필요하다 —
[docs/release-checklist.md](docs/release-checklist.md) 참조.

### 2. 런타임 구성

`Builder`로 **worker governance(topology/resources)** 와 **semantic policy(class_policy)** 를
분리해서 선언한다. `build()`는 정책을 검증하며, 불가능한 예산이면 `GovernorError`로 fail-closed.

```rust
use taskmesh::{
    Builder, CancellationPolicy, ClassPolicy, FairnessPolicy, ResourceBudget, TaskClass,
    TopologyConfig,
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
            .fairness(FairnessPolicy::DeficitRoundRobin { quantum: 4 })
            // §6의 mid-run 취소·데드라인은 이 정책이 있어야 발효된다. 기본값
            // `PreSubmitOnly`인 클래스에 deadline을 주면 `DeadlineUnsupported`로 거절된다.
            .cancellation_policy(CancellationPolicy::CooperativeWithDeadline),
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
let spec = TaskSpec::local(class).operation("local:job");
let out = runtime.run_local(spec, async { Ok::<_, MyError>(do_local().await) }).await?;
```

> `run_*`에서 보이는 `Runtime` trait은 `taskmesh::Runtime`(top-level)이다. 메서드를
> 쓰려면 `use taskmesh::Runtime;`이 스코프에 있어야 한다.

자식 작업은 root에 귀속시켜 attribution을 유지한다:

```rust
use taskmesh::TaskStage;

let child = TaskSpec::cpu(TaskClass::new("retrieval"))
    .child_of("fetch:doc:42", "fetch:doc:42", TaskStage::new("rank"))
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
        | AdmissionVerdict::SubstrateSaturated { retry_after_ms }   // 즉시 shed (비-queueing 클래스)
        | AdmissionVerdict::SubstratePoolTimedOut { retry_after_ms } // bounded 대기 후 만료
        | AdmissionVerdict::PermitAcquireTimedOut { retry_after_ms } => {
            // retry_after_ms 만큼 backoff 후 재시도
            let _ = retry_after_ms;
        }
        AdmissionVerdict::SubstrateMismatch => { /* run path ↔ hint 불일치 */ }
        AdmissionVerdict::MalformedTask => { /* 0-stage / stage class 불일치 / reduce 누락 */ }
        other => { /* ClassDisabled, RuntimeUnavailable 등 */ let _ = other; }
    },

    // 작업 자체가 실패
    // 큐에서 대기하던 ticket이 claim 전에 끝났다 (leak sweep 회수 등). 사유가 보존된다.
    Err(RunError::Governor(GovernorError::TicketClaimTerminated { reason, .. })) => { let _ = reason; }

    // 작업 자체가 실패
    Err(RunError::Task(e)) => { /* 사용자 에러 e */ let _ = e; }

    Err(other) => { let _ = other; }
}
```

`verdict.retry_after_ms()`로 backpressure 힌트를 바로 꺼낼 수 있다.

### 5. 관측 (inventory-first)

`snapshot()`은 클래스별 inflight/queue/held 자원, ownership phase gauge, 누적 counter,
capability-pool 점유, substrate 인벤토리를 결정적으로 반환한다 (`schema_version = 2`).

```rust
let snap = runtime.snapshot();
assert_eq!(snap.conservation_violation(), None); // inflight == phase 합, admitted == inflight + terminated
for (class, c) in &snap.classes {
    println!(
        "{class}: inflight={} (reserved={} accepted={} running={} cleanup={}) queued={} cpu_held={}",
        c.inflight, c.dispatch_reserved, c.accepted, c.running, c.cleanup_pending, c.queued, c.cpu_units_held,
    );
}
for (pool, usage) in &snap.capabilities {
    println!("pool {pool}: {}/{}", usage.in_use, usage.limit); // limit 0 = 무제한
}
for s in &snap.substrates {
    println!("substrate {} kind={:?} pool={:?}", s.name, s.kind, s.capability_pool);
}
```

**`queued`와 outstanding의 구분.** `queued`는 클래스 admission queue에서 기다리는 요청 수다.
admission 앞에 별도의 무제한 대기 공간은 없으므로, runtime이 보유한 outstanding 요청은 정확히
`queued + inflight`다. held 자원(`cpu_units_held`/`memory_units_held`)은 `u128`이며 JSON에서는
정수 정확도를 잃지 않도록 decimal string으로 직렬화된다.

### 6. 취소 · 타임아웃 · 데드라인 (`SubmitOptions`)

기본 `run_*`는 무제한 대기다. 취소/타임아웃/데드라인이 필요하면 `run_*_with(spec,
SubmitOptions, …)` 변형을 쓴다.

```rust
use std::time::Duration;
use taskmesh::{AdmissionVerdict, CancellationToken, GovernorError, RunError, SubmitOptions, TaskClass, TaskSpec};

let token = CancellationToken::new();
let opts = SubmitOptions::unbounded()
    .with_cancel(token.clone())                       // 제출 전/실행 중 협조 취소
    .with_acquire_timeout(Duration::from_millis(50))  // admission 큐 대기 상한
    .with_deadline(Duration::from_secs(2));           // 실행 데드라인

let spec = TaskSpec::io(TaskClass::new("retrieval")).operation("fetch");
let out = runtime.run_io_with(spec, opts, async { Ok::<_, MyError>(fetch().await) }).await;

match out {
    Err(RunError::Governor(GovernorError::Rejected(AdmissionVerdict::CancelledBeforeSubmit))) => {
        /* 제출 전 취소 */
    }
    Err(RunError::Governor(GovernorError::Cancelled)) => { /* mid-run 협조 취소 */ }
    Err(RunError::Governor(GovernorError::DeadlineExceeded)) => { /* 실행 데드라인 초과 */ }
    Err(RunError::Governor(GovernorError::Rejected(
        AdmissionVerdict::PermitAcquireTimedOut { .. },
    ))) => { /* 큐 대기 타임아웃 */ }
    Err(RunError::Governor(GovernorError::DeadlineUnsupported { class, policy })) => {
        /* 구성 오류: `class`의 `policy`는 데드라인을 집행할 수 없다 (제출 전 거절, 작업 미시작) */
    }
    _ => {}
}
```

규칙: pre-submit 취소(`CancelledBeforeSubmit`)는 모든 클래스에서 발효된다. **mid-run
협조 취소**(`Cancelled`)는 `Cooperative`/`CooperativeWithDeadline` 클래스에서 동작한다 —
`run_io`/`run_local`/`run_cpu`/`run_blocking` 모두에서 caller의 대기를 끝내며, 동기 worker
(`run_cpu`/`run_blocking`)의 시작된 작업은 중단되지 않고 charged 상태로 끝까지 실행된다.
**데드라인**(`DeadlineExceeded`)은 `CooperativeWithDeadline` 클래스에서만 동작한다. 데드라인을 집행할 수
없는 클래스에 `deadline`을 주면 조용히 버려지는 대신 제출 시점에
`GovernorError::DeadlineUnsupported { class, policy }`로 거절된다 — 요청한 상한 없이 끝까지
실행하고 `Ok`를 돌려주는 것은 이 host가 만들지 않는 종류의 실패다.
취소/타임아웃으로 future가 drop돼도 permit과 capability slot은 항상 반납된다(누수 없음).

worker 쪽 실패는 각각의 이름으로 온다: `WorkerPanicked { context }`(작업 또는 adapter의
`spawn`이 panic; 부작용 미상), `WorkerUnavailable { context, detail }`(thread/runtime을 만들지
못함; 작업은 시작되지 않았다), `JobAbandoned { context }`(adapter가 closure를 실행하지 않고
버렸거나 worker가 사라짐). `LeaseReclaimed { permit_id }`는 dispatch 전에 leak sweep이 lease를
회수한 경우로, 작업은 시작되지 않는다.

**응답과 custody는 다른 사건이다.** deadline/취소 응답은 caller의 대기를 끝내지만, 동기 worker
(blocking·CPU·requested-stack)는 실제로 종료할 때까지 lease를 계속 보유한다 — 그동안 snapshot의
해당 요청은 `running`/`cleanup_pending`으로 남고 용량은 반환되지 않는다. 정상 완료 시에는 custody가
결과와 함께 이동하므로 caller가 값을 볼 수 있는 시점에 용량은 이미 반환되어 있다. blocking 경로의
`RunFor`는 **caller의 대기**를 제한할 뿐 시작된 동기 작업을 중단하지 않는다.

`acquire_timeout`은 admission lock 대기를 포함한 모든 획득 대기를 포함한다. 만료 후 도착한
permit은 — 즉시 승인이든, 큐 대기 중 timeout과 경합한 promotion이든 — 시작되지 않고 unwind되며
`PermitAcquireTimedOut`(capability pool 대기였다면 `SubstratePoolTimedOut`)이 반환된다.
`Duration::ZERO`는 "대기하지 말고 시도" 의미를 유지한다.

**종료(drain).** 프로세스를 내리기 전에는 `drain`으로 새 작업을 막고 이미 받은 작업이 끝나기를 기다린다.
`drain`은 engine 안에서 admission을 닫으므로(one-way) 이후 모든 제출은 admission 전에
`Rejected(RuntimeUnavailable)`로 거절되고, queue·계상·total은 움직이지 않는다. 이미 queue·admit된
작업은 취소하지 않는다. 기다림의 기준은 caller의 응답이 아니라 engine의 custody gauge
(`inflight == 0 && queued == 0`, 모든 클래스)다.

```rust
use std::time::Duration;

match runtime.drain(Duration::from_secs(30)).await {
    Ok(report) => {
        // 모든 클래스가 비었다: handle을 drop하면 남는 것이 없다.
        let _ = report.elapsed;
    }
    Err(not_drained) => {
        // 아직 charged된 작업이 있다 — 시작된 blocking 작업은 abort할 수 없다.
        // runtime은 draining 상태로 남고, 다시 drain하면 같은 대기를 이어간다.
        for (class, left) in &not_drained.classes {
            eprintln!("{class}: inflight={} queued={}", left.inflight, left.queued);
        }
    }
}
```

teardown은 여전히 runtime handle drop뿐이다.

### 7. 고급 통합 (`taskmesh::ext`)

일상적 SDK 사용엔 필요 없다. 직접 임베딩·커스텀 어댑터·심층 거버넌스가 필요할 때만:

```rust
use taskmesh::ext::{Governor, Provenance};

// (a) 거버너 직접 접근 — 메모리 reconcile / leak sweep / substrate 인벤토리
let gov: &Governor = runtime.governor();
let report = gov.reap_leaks();                       // LeakDetecting 클래스의 stale·미시작 permit 회수
// report.retained_active: stale이지만 실행 중이라 회수하지 않은 permit 수 (stale ≠ dead)
// gov.reconcile_memory(permit_id, measured_bytes) -> ReconcileOutcome
// EpochExhausted면 같은 permit에서 재시도하지 않고 release해야 한다.
// gov.release_stage_memory(permit_id, units) -> StageReleaseOutcome (OnTaskCompletion 클래스는 PolicyForbids)
// gov.claim(ticket) -> ClaimOutcome::{Ready, Pending, Terminal(reason), Invalid}

// (b) classification provenance — "왜 이 클래스였나"가 submit 이후에도 audit 가능
// let prov: Option<Provenance> = gov.permit_provenance(permit_id); // source/reason
let _ = report;
```

`features = ["rayon"]`만 켜면 `run_cpu`의 기본 executor가 공유 rayon 풀로 **자동
와이어링**된다(추가 코드 불필요). 풀을 직접 만들어 주입하려면 같은 feature 아래
`taskmesh::ext::RayonCpuExecutor`를 쓴다 — `taskmesh-rayon`을 직접 의존할 필요는 없다:

```rust
// Cargo.toml: taskmesh = { …, features = ["rayon"] }
use std::sync::Arc;
use taskmesh::ext::RayonCpuExecutor;

let topo = taskmesh::TopologyConfig::new().cpu_auto().reserve_cores(1);
let runtime = taskmesh::Builder::new()
    .topology(topo.clone())
    .cpu_executor(Arc::new(RayonCpuExecutor::try_from_topology(&topo)?))
    .class_policy(taskmesh::TaskClass::new("rank"), taskmesh::ClassPolicy::new().cpu_units(1))
    .build()?;
```

주의: `RayonCpuExecutor::with_pool(pool)`은 그 pool의 실제 thread 수와 shared ownership을
선언한다. executor가 physical domain, nonblocking submit, actual worker count를 명확히
선언하지 못하거나 topology와 불일치하면 `build()`에서 typed reject된다.
직접 `Governor`를 구동하는 embedder는 `advance_phase(permit, Running)`/`CleanupPending`을
자기가 선언해야 한다: leak sweep(`DEFAULT_LEAK_STALE_MS` = 60 000 ms)은 `DispatchReserved`인
permit만 회수하므로, 전진시키지 않은 permit은 실행 중에도 회수 대상이다. `DispatchReserved`를
벗어나는 **첫** `advance_phase`는 `AdvanceOutcome::Leased(LeaseToken)`으로 그 permit의 유일한
lease를 돌려준다 — 그 뒤로는 `release_leased(token)`만 permit을 끝내고, `release(permit_id)`는
`ReleaseOutcome::HeldByLease { phase }`로 거절된다(아무것도 바꾸지 않음). 실행 중인 남의 작업
용량을 번호 하나 잘못 넘겨 환급하는 실수를 구조로 막는다; token은 `Clone`이 아니고 `#[must_use]`다.

> `ext`에는 `Governor`, `PolicySet`, `AdmissionDecision`, `Provenance`,
> `RootAttribution`, `Clock`/`SystemClock`/`ManualClock`, `CpuExecutor`,
> `PermitWaker`, `TokioPermitWaker`, `BlockingPoolCpuExecutor` (+ `RayonCpuExecutor` with
> `rayon`), `RequestKey`, `builtin_records`,
> `BUILTIN_SUBSTRATES`, `DEFAULT_LEAK_STALE_MS`, `ClaimOutcome`, `ReleaseOutcome`,
> `StageReleaseOutcome`, `ReconcileOutcome`, `PermitLedgerView`, `CapacityBlock` 등이 있다.
> `Governor::release`는 `ReleaseOutcome`(`#[must_use]`)을 돌려준다 — `UnknownPermit`은 double
> release이거나 sweep에 회수된 lease이며 조용한 no-op이 아니다; `HeldByLease { phase }`는 dispatch된
> permit을 id로 놓으려 한 것으로, 그 permit은 `LeaseToken`의 것이다. `RequestKey`는 더 이상
> 공개 입력이 아니다 — admission key는 `root_operation_id`에서 권위적으로 파생된다.
> 계약의 근거는 [ADR 0003](docs/adr/0003-sep-16-hardening-contracts.md)에 있다.

### 클래스 정책 옵션 요약

| 메서드 / 필드 | 의미 |
|---|---|
| `max_inflight` | 동시 실행 상한 |
| `max_queue_depth` | 대기 큐 깊이 (초과 시 `overflow_policy`) |
| `fairness` | `Fifo` / `WeightedFairQueue` / `DeficitRoundRobin` / `DeadlineAware` / `BestEffortScavenger` |
| `retry_after_policy` | `None` / `FixedMs(ms)` / `Adaptive` — 거부 시 backoff 힌트 |
| `overflow_policy` | `Reject` / `QueueWithinDepth` / `DropBestEffort`(현재 `Reject`와 동치) |
| `memory_overcommit_policy` | `Reject` / `Queue` / `DegradeToLight { fallback_class }` |
| `memory_release_policy` | `OnTaskCompletion` / `OnStageBoundary` / `LeakDetecting`(leak sweep 대상). **강제 계약**: `OnTaskCompletion`은 stage release를 `PolicyForbids`로 거절 |
| `cancellation_policy` | `PreSubmitOnly` / `Cooperative` / `CooperativeWithDeadline` |

## 집행 의미 (enforcement semantics)

선언된 contract는 런타임에서 실제로 집행된다 (선언만 하는 inert knob 아님):

1. **substrate 분류는 권위적이다.** 각 `run_*`는 spec의 substrate hint와 일치해야 한다.
   `run_io`=`AsyncIo`, `run_blocking`=`BlockingPool`/`LargeStackCapability`/`BackgroundOnly`,
   `run_cpu`=`SharedCpuExecutor`, `run_local`=`LocalRuntime`. 불일치는 `SubstrateMismatch`로 reject.
2. **topology slot은 실제 capability-pool 상한이며, admission과 같은 결정이다.** `blocking_threads`/
   `large_stack_slots`/`local_runtime_slots`/`maintenance_workers` 및 topology-sized CPU 풀은 해당
   substrate 동시성을 제한한다. `0 = 무제한`. capability 점유는 class inflight·resource budget과
   **하나의** admission transition에서 결정되므로 admission 앞에 별도 대기 큐가 없다: 풀이 가득 차면
   비-queueing 클래스는 즉시 `SubstrateSaturated`로 shed되고, queueing 클래스는 자기 `max_queue_depth`
   안에서 기다리다 만료 시 `SubstratePoolTimedOut`(거버너 큐 대기 `PermitAcquireTimedOut`와 구분).
   `stack_size_bytes`가 있는 blocking-family 제출은 hint와 무관하게 `large_stack` pool을 소비한다.
   `Blocking`/`LargeStack`/`Background`는 blocking executor를 공유하되 capability pool은 별도다.
3. **fan-out reduce는 admission에서 강제된다.** reduce policy 없는 fan-out stage는 `MalformedTask`.
   0-stage spec과 stage별 class 불일치도 `MalformedTask`로 reject.
4. **fairness는 tier 단위 discipline이다.** 한 tier(best-effort 여부) 안의 클래스는 같은 discipline을
   써야 하며, 혼합 시 `build()`가 거부한다. (weight/quantum/slack 등 파라미터는 클래스별로 달라도 됨.)
   `DeficitRoundRobin`은 deficit 커서로 quantum에 비례해 라운드로빈한다 (ring은 산술적으로 순회하며
   queue가 drain되면 credit을 reset한다). `WeightedFairQueue`는 `1..=u32::MAX` 전 범위에서 비례를
   유지하며 weight 0은 `build()`가 거부한다. 취소된 요청은 service debt를 남기지 않는다.
   클래스 내부는 도착 순서다: 같은 클래스의 runnable한 head가 promotion pass를 기다리는 동안 새
   도착은 그 앞의 capacity를 가져가지 못하고 뒤에 선다(`CapacityBlock::QueuedBehind`); head가
   *다른* capability pool에 막혀 있을 때만 추월한다. 한 번의 promotion pass는 `PROMOTION_BUDGET`(64)
   개까지 grant하고 lock을 놓은 뒤 이어가며, 그 경계에서 fairness 비용이 어긋나지 않는다.
5. **cancellation은 정책을 따른다.** `Cooperative`/`CooperativeWithDeadline` 클래스는
   `run_io`/`run_local`/`run_cpu`/`run_blocking` 모두에서 토큰으로 mid-run 취소(→`GovernorError::Cancelled`)된다
   (`run_cpu`는 stalled `CpuExecutor`에서도 탈출; `run_cpu`/`run_blocking`의 시작된 동기 작업은
   중단되지 않고 charged 상태로 끝까지 실행된다 — caller의 대기만 끝난다). `CooperativeWithDeadline`는 추가로
   `SubmitOptions::deadline`을 발효(→`GovernorError::DeadlineExceeded`); 다른 정책의 클래스에 deadline을
   주면 `DeadlineUnsupported`로 거절된다(무시하지 않는다).
   `RunFor`는 worker 자신의 시작 시각을 기준으로 하며(inline executor 포함) 완료 시각이 budget을
   넘긴 결과는 `Ok`로 보고되지 않는다. `run_blocking`의 `RunFor`는 caller의 대기만 제한한다.
   worker 실패는 `WorkerPanicked`/`WorkerUnavailable`/`JobAbandoned`로 구분된다.
6. **`LeakDetecting` 클래스만 leak sweep으로 회수**되며, 그중에서도 executor에 도달하지 않은
   (`DispatchReserved`) permit만 회수한다 — stale은 dead가 아니다. 실행 중인 stale permit은
   `retained_active`로 보고되고 charged 상태로 남는다. stage release·reconcile은 lease를 touch한다.
   downward `reconcile_memory`는 예산을 풀고 대기 작업을 promote한다. lease timestamp는 monotonic
   commit watermark로 clamp되므로 늦게 도착한 clock 샘플이 lease를 과거로 되돌리지 못한다.
7. **정책 검증은 fail-closed다.** 불가능한 budget·mixed-tier fairness·잘못된 memory scaling·disabled
   클래스로의 `DegradeToLight`·스스로 다시 degrade하는 fallback으로의 `DegradeToLight`(degrade는 한
   hop이며 체인·순환은 거부) 등은 `Builder::build`/`Governor::new`에서 거부되며 런타임 admit로
   미루지 않는다. `CpuExecutor`가 `declared_workers`를 선언하면 topology가 resolve한 `cpu` gate보다
   작을 수 없다(`TopologyError::ExecutorDeclaresFewerWorkers`) — gate와 pool은 한 답에서 나온다.
   `runtime.executor_capabilities()`가 adapter의 선언(worker 수·exclusive 여부·non-blocking submit)을
   노출한다.
8. **child→root 귀속:** `child_of(root, immediate_parent_operation, parent_stage)`는 root와
   직접 parent identity를 함께 보존한다. `operation(name)`은 root를 재설정하지 않는다.
   동일 root의 중복 active operation은 typed reject되며, parent lookup은 문자열 stage scan이 아닌
   exact operation identity를 사용한다. **선언된 nested wait (SEP-21 E04):** parent가 child를 기다리면
   `awaited_child_of(root, immediate_parent_operation, parent_stage)`로 선언한다. 직접 parent permit이
   필요한 capacity를 전부 보유한 irreversible blocker라면 `NestedWaitCycle`로 즉시 거절한다.
   sibling·stranger가 해제할 수 있는 blocker는 정상 bounded queue이며, 기다리는 동안 새 cycle이
   형성되면 promotion 때 terminalize한다. 미선언 wait는 추론하지 않는다.
9. `checkpoint_policy`는 host가 hook point에서 inspect하는 **메타데이터**다(엔진 강제 아님).

문서:

1. [RFC](docs/rfcs/0001-governed-runtime.md)
2. [Library Spec](docs/taskmesh-library-spec.md)
3. [External Interface](docs/taskmesh-external-interface.md)
4. [Accepted ADR index](docs/adr/README.md)
5. [Completed work archive](docs/archive/2026-09-25/README.md)
6. [Open BG25 follow-ups](docs/plans/bugbash-sep-25-general/tickets/OPEN-FOLLOWUPS.md)
