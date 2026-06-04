# 0001. Feature-sliced 워크스페이스와 헥사고날 거버넌스 엔진

- 상태: Accepted
- 날짜: 2026-06-04
- 결정자: Song Min

## Context

`taskmesh`는 분석 시스템과 서비스 런타임을 위한 governed execution
control-plane이다. 현재는 compileable skeleton 단계이며, `taskmesh-contract`
(public 타입), `taskmesh-core`(Governor), `taskmesh-tokio`(facade) 3개 crate로
구성되어 있다.

skeleton을 usable governed runtime으로 끌어올리는 과정에서 다음 성질을 구조적으로
강제하고 싶다.

1. semantic policy와 worker governance의 분리 (AGENTS 규칙 1)
2. fail-closed, deterministic 거버넌스 로직을 async 런타임 세부사항에서 격리
3. executor(tokio/rayon)를 교체 가능한 어댑터로 취급, engine-specific pool 금지
4. public naming drift 0, product-neutral public API 유지

문제는 거버넌스 결정 로직(admission/fairness/memory/composite/inventory)이 tokio
primitive, IO, 시계, 메모리 측정 같은 부작용과 섞이면 결정성·테스트성·교체성이
모두 무너진다는 것이다.

## Decision

워크스페이스를 **헥사고날의 의존성 역전 경계(seam)** 를 따라 4개 crate로 자르고,
거버넌스 엔진 내부는 **feature slice × 헥사고날 레이어**로 구성한다.

### 1. Crate 경계 = 헥사곤 경계

| crate | 역할 | 의존 |
|---|---|---|
| `taskmesh-contract` | 어휘 + 모든 port trait. 헥사곤 경계 그 자체 | serde |
| `taskmesh-engine` | 순수 거버넌스 core (full hexagonal) | contract, parking_lot |
| `taskmesh` | host facade. tokio 어댑터 + Builder (semi-hexagonal) | contract, engine, tokio |
| `taskmesh-rayon` | rayon Executor 어댑터 | contract, rayon |

- `taskmesh-core`는 `taskmesh-engine`으로 개명한다.
- facade는 `crates/taskmesh-tokio/`에서 `crates/taskmesh/`로 이동하되,
  package 이름은 `taskmesh`를 유지한다.

### 2. 모든 port trait은 `taskmesh-contract`에 둔다

- Driving port: `Runtime` (`run_io/run_blocking/run_cpu/run_local`)
- Driven port: `CpuExecutor`, `Clock`, `PermitWaker`

contract가 단일 경계를 정의하므로 엔진은 경계 왼쪽 안, 어댑터는 오른쪽 밖에 위치한다.
의존 화살표는 항상 contract/engine 안쪽으로만 향한다.

> **갱신 (구현 후):** 초안은 driven port를 `Executor`/`Clock`/`MemoryProbe`로 적었으나
> 실제 구현은 다음으로 확정했다.
> - `Executor` → `CpuExecutor` (object-safe `fn spawn(Box<dyn FnOnce()+Send>)`. tokio
>   blocking-pool 기본 / rayon 어댑터가 같은 port를 구현. T09 oneshot 브리지와 일치).
> - **`MemoryProbe`는 채택하지 않음.** measured 메모리는 host가 *밀어넣는*
>   `Governor::reconcile_memory(permit, measured_bytes)` 입력으로 모델링했다 (T05).
>   pull 방식 probe port는 엔진이 타이머로 호출하는 부작용을 강제하므로,
>   push 방식이 결정성·단순성에서 우월하다.
> - `PermitWaker`(driven)를 추가: 큐잉된 admission이 promotion 시 깨어나도록
>   host가 tokio `Notify`로 구현. 엔진은 런타임 무지 상태를 유지한다.

### 3. 의존 그래프 (순환 불가)

```
                taskmesh-contract            (serde)
                ▲        ▲             ▲
        ┌───────┘        │             └──────────┐
 taskmesh-engine         │                  taskmesh-rayon
        ▲                │                        ▲
        └──────── taskmesh (host) ───────────────┘   feature = "rayon"
```

tokio와 rayon은 서로를 모르고, 엔진은 tokio를 모른다. rayon은 contract의
`Executor` port만 구현하므로 host와 완전히 독립적이다.

### 4. 엔진 내부: feature slice × 헥사고날

crate 경계는 헥사곤 seam을 따르지만, feature는 crate가 아니라 엔진 내부의
vertical slice(모듈)로 둔다. 각 slice는 `domain`(순수 규칙) / `service`(use-case
오케스트레이션) / 필요 시 slice-local port로 자기완결한다.

| slice | 티켓 | domain 책임 |
|---|---|---|
| `admission` | T03 | 정원 판정, UnknownClass fail-closed |
| `fairness` | T04 | queue discipline(FIFO/WFQ/DRR/Deadline/Scavenger), retry-after |
| `memory` | T05 | overcommit(Reject/Queue/Degrade), release, leak sweep |
| `composite` | T06 | child→root 귀속, 재귀 admission, deterministic reduce |
| `inventory` | T08 | capability-pool 무결성, snapshot 합성 |

`engine/governor.rs`가 composition root로서 port를 보유하고 각 slice service에
위임한다. 공유 가변 상태는 `engine/state.rs`(`Mutex<GovernedState>`)에 격리한다.

### 5. 디렉토리 구조

```
crates/
├── taskmesh-contract/src/
│   ├── lib.rs                  # 평면 re-export
│   ├── task/{spec,identity}.rs
│   ├── policy/{class_policy,fairness,memory,reduce,flow}.rs
│   ├── resource/  topology/
│   ├── verdict.rs  snapshot.rs  config.rs
│   ├── runtime.rs              # DRIVING port: Runtime
│   └── ports/{executor,clock,memory_probe}.rs   # DRIVEN ports
├── taskmesh-engine/src/
│   ├── lib.rs
│   ├── engine/{governor,state}.rs               # composition root
│   ├── shared/{permit,seq}.rs                   # 크로스-슬라이스 커널
│   └── features/
│       ├── admission/{domain,service}.rs
│       ├── fairness/{domain,queue,retry_after,service}.rs
│       ├── memory/{domain,leak_sweep,service}.rs
│       ├── composite/{domain,reduce,service}.rs
│       └── inventory/{domain,service}.rs
├── taskmesh/src/                                # package name = taskmesh
│   ├── lib.rs  builder.rs  runtime.rs
│   ├── executor/{mod,tokio_exec,cancel}.rs      # tokio = Executor 구현
│   └── adapters/{clock,memory_probe}.rs         # core port 바인딩
└── taskmesh-rayon/src/{lib,rayon_exec}.rs
```

## Consequences

### 긍정

- 거버넌스 `domain`은 tokio/IO를 import하지 않으므로 결정성·단위테스트가 쉽다.
  시계·메모리 측정은 전부 port 뒤로 빠진다(`Clock`, `MemoryProbe`).
- executor 교체가 구조적으로 자유롭다. rayon은 tokio 비의존 어댑터로 plug-in되고,
  순환 의존이 발생할 수 없다.
- feature slice가 티켓(T03~T08)에 1:1로 대응하여 작업 경계가 명확하다.
- public 타입 이름이 그대로이므로 downstream은 import 경로만 무변 영향
  (`taskmesh::*` 평면 유지) → public naming drift 0.

### 부정 / 트레이드오프

- crate 재배치(특히 facade를 `crates/taskmesh-tokio` → `crates/taskmesh`)는
  기존 jun-4-startup 플랜의 no-go("facade source = crates/taskmesh-tokio/src/lib.rs")를
  override한다. 플랜 문서를 이 ADR 기준으로 갱신해야 한다.
- port trait을 contract로 끌어올리면 contract가 async trait를 노출한다
  (Rust 1.95 기준 문제 없음, 단 contract의 표면적이 커진다).
- 모듈 분할이 늘어 초기 보일러플레이트(`mod.rs` re-export)가 증가한다.

### 대안

- **5-crate (별도 umbrella)**: `taskmesh` umbrella + `taskmesh-tokio` host를 분리해
  host 자체를 교체 가능하게. 다른 async 런타임 대비에는 유리하나, 현 시점 tokio가
  사실상 유일 host이므로 4-crate로 단순화했다. 향후 필요 시 facade를 umbrella로
  승격하는 것은 비파괴적 변경이다.
- **feature를 crate로 분리**: 컴파일·ergonomics 비용이 커서 기각. feature는 엔진
  내부 모듈로 둔다.

## 관련 문서

- [RFC 0001 — Governed Runtime](../rfcs/0001-governed-runtime.md)
- [Library Spec](../taskmesh-library-spec.md)
- [Jun-4 Startup Ticket Set](../plans/jun-4-startup/README.md)
