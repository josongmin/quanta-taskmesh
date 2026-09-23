# Taskmesh SOTA Audit Checklist

이 문서는 `taskmesh`의 실제 용도—분석 시스템과 서비스 런타임 안에서 admission,
fairness, 자원 예산, 실행 substrate, cancellation/deadline, drain을 한 control-plane으로
통제하는 Rust 라이브러리—에 맞춘 감사 기준이다. 일반적인 Rust 스타일 체크리스트가
아니며, 이 저장소의 public contract와 실제 실행 경로를 함께 검사한다.

## 1. 사용 방법

### 1.1 감사 등급

| 등급 | 적용 시점 | 종료 조건 |
|---|---|---|
| `M` merge | 모든 production/tooling 변경 | 해당 변경과 reachable consumer에 관련된 `M` 항목이 모두 PASS |
| `R` release | crate 버전·공개 API·정책·런타임 릴리스 | 모든 `M/R` 항목 PASS, clean exact-SHA local receipt `QUALIFIED` |
| `D` deep | 분기별 또는 scheduler/accounting/concurrency 대수술 | 모든 `M/R/D` 항목 판정, generated mutation·장시간 fuzz/load/soak 포함 |

`R`과 `D`는 `M`을 포함한다. 적용되지 않는 항목은 빈칸으로 두지 말고 `N/A`와 근거를
기록한다.

### 1.2 허용 판정

- `PASS`: PASS 조건을 충족하는 현재 source의 증거가 있다.
- `FAIL`: 재현 가능한 계약 위반 또는 잘못된 구현이 있다.
- `PARTIAL`: 일부 surface만 확인됐다. 전체 PASS로 합산하지 않는다.
- `NOT_RUN`: 실행 가능한 검증을 실행하지 않았다.
- `BLOCKED_EXTERNAL`: hosted runner, consumer, 배포 환경 등 외부 권한/환경이 필요하다.
- `N/A`: Taskmesh의 명시적 non-goal이다. ADR/spec 근거가 필수다.

### 1.3 심각도

| 등급 | 기준 |
|---|---|
| `P0` | permit/accounting 위조, 무제한 대기열, fail-open admission, deadlock, data race, source와 무관한 허위 qualification |
| `P1` | reachable starvation, deadline/cancel custody 유실, pool oversubscription, 잘못된 public contract, release 차단 proof gap |
| `P2` | 관측 불완전, 특정 경계의 약한 테스트, 성능 회귀 위험, 유지보수성 결함 |
| `P3` | 문서·진단·ergonomics 개선. 실행 의미를 바꾸지 않음 |

### 1.4 증거 강도

| 단계 | 증거 | 허용 주장 |
|---|---|---|
| `E0` | 문서·코드 정독 | 후보 finding, 설계 일치 여부 |
| `E1` | 정적 검사·compile·API diff | 구조/타입/feature 호환 |
| `E2` | focused runtime test | 특정 경로의 관측 동작 |
| `E3` | 독립 oracle, mutation, model checking, fuzz | 테스트가 결함을 실제로 구분함 |
| `E4` | clean exact-SHA local qualification receipt | 저장소 릴리스 자격 |
| `E5` | 외부 consumer/replay/deploy/activation | 실제 통합·운영 활성화 |

낮은 단계는 높은 단계를 대체하지 않는다. 과거 receipt, dirty-tree 실행, focused test,
coverage percentage, 문서 체크박스는 각각의 범위를 넘어선 완료 증거가 아니다.

## 2. 비교 기준과 Taskmesh 적용 범위

| 기준 | 가져오는 원칙 | Taskmesh 적용 |
|---|---|---|
| [Tower `ServiceBuilder`](https://docs.rs/tower/latest/tower/struct.ServiceBuilder.html), [load shedding](https://docs.rs/tower/latest/tower/load_shed/) | buffer와 concurrency limit의 순서가 전체 outstanding 수를 바꾼다. 준비되지 않은 service는 명시적으로 shed한다. | admission 앞/뒤의 숨은 queue 금지, `queued + inflight` 전체 경계, typed saturation |
| [Tokio semaphore](https://docs.rs/tokio/latest/tokio/sync/struct.Semaphore.html), [`select!` cancellation safety](https://docs.rs/tokio/latest/tokio/macro.select.html) | semaphore fairness와 head-of-line blocking, queue 위치를 잃는 cancellation, `.await` drop 경계 | substrate acquire ordering, cancel-safe custody, caller future drop과 permit lifecycle 분리 |
| [Tokio graceful shutdown](https://tokio.rs/tokio/topics/shutdown) | shutdown 신호, 전파, 완료 대기는 별개 단계 | engine admission close, host drain, bounded wait, not-drained inventory |
| [Google SRE cascading failures](https://sre.google/sre-book/addressing-cascading-failures/) | 작은 bounded queue, overload 조기 거절, deadline propagation, retry amplification 방지 | per-class queue, absolute deadline, retry hint, load-shed/degrade 실험 |
| [DRR 원 논문](https://web.stanford.edu/class/ee384x/EE384X/papers/DRR.pdf) | active flow ring, deficit carry, bounded work, 비대칭 quantum에서 비례성 | Taskmesh DRR의 ring/cost/credit/cancellation/idle-reset oracle |
| [Dominant Resource Fairness](https://www.usenix.org/conference/nsdi11/dominant-resource-fairness-fair-allocation-multiple-resource-types) | heterogeneous CPU/memory demand에서 single-resource fairness만 보면 왜곡 가능 | DRF 구현을 요구하지는 않지만 CPU·memory·pool 병목별 starvation과 정책 설명을 검사 |
| [Kubernetes scheduling framework](https://kubernetes.io/docs/concepts/scheduling-eviction/scheduling-framework/) | scheduling/binding 단계 분리, reserve/permit, event-driven requeue/backoff | admission/reservation/claim/lease phase, capacity 변화 기반 promotion, busy polling 금지 |
| [Loom](https://docs.rs/loom/latest/loom/) | replacement primitive 밖 동시성은 model에 보이지 않는다 | replica가 아닌 production `Governor`, seam 밖의 Tokio/OS 경로는 TSan/host test로 보완 |
| [cargo-mutants outcomes](https://mutants.rs/using-results.html) | caught/missed/unviable/timeout은 서로 다른 결과다 | curated fault inventory와 generated sweep 분리, survivor 개별 판정 |
| [Cargo SemVer](https://doc.rust-lang.org/cargo/reference/semver.html), [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) | public item/feature/MSRV 변화는 소비자 계약이다 | facade 표면, `0.x` minor migration, external MSRV fixture, semver-checks |
| [OpenTelemetry semantic conventions](https://opentelemetry.io/docs/concepts/semantic-conventions/) | 저카디널리티 이름과 안정된 telemetry schema | verdict/phase/pool/class 관측 schema, operation id의 metric label 사용 금지 |
| [HdrHistogram coordinated omission 보정](https://hdrhistogram.github.io/HdrHistogram/JavaDoc/org/HdrHistogram/AbstractHistogram.html) | generator가 느려진 시간을 누락하면 tail latency가 과소 측정된다. | open-loop/raw와 corrected latency 동시 보존, goodput/reject 비율 분리 |

이 기준들은 비교 재료다. Taskmesh는 분산 cluster scheduler가 아니므로 node placement,
preemption, replicated consensus, durable job persistence를 자동 요구하지 않는다. 반대로 library
내 process-local control-plane이라는 이유로 queue, permit, cancellation, shutdown 정확성을 완화하지
않는다.

### 2.1 현재 저장소 owner map

| 감사 영역 | authority source | 주요 consumer/proof |
|---|---|---|
| public vocabulary·wire·ports | `crates/taskmesh-contract/src/` | contract tests, doc-examples, consumer-MSRV |
| state machine·회계 | `crates/taskmesh-engine/src/engine/` | permit/lifecycle/exact-accounting/differential tests |
| admission·fairness·memory·composite | `crates/taskmesh-engine/src/features/` | policy별 integration/property/Loom/Shuttle tests |
| Tokio host·custody | `crates/taskmesh/src/runtime.rs`, `runtime/`, `executor/` | `crates/taskmesh/tests/` |
| topology/config composition | `crates/taskmesh/src/builder.rs`, engine `PolicySet` | config/inventory/executor protocol tests |
| Rayon adapter | `crates/taskmesh-rayon/` | `test-rayon`, external consumer fixture |
| workload/performance model | `crates/taskmesh-bench/`, `tools/bench*` | criterion, IAI, allocation/loadgen tests |
| proof orchestration | `Justfile`, `tools/gates/`, `.github/workflows/` | gate parity tests, receipt collector |
| mutation evidence | `tools/verification/` | runner self-tests, curated receipt, generated sweep artifact |
| qualification authority | `tools/qualification/receipt.py` | hosted `qualification` job and uploaded artifacts |
| contracts/runbook | `README.md`, `docs/taskmesh-*`, `docs/adr/`, `docs/release-checklist.md` | doctest, extracted examples, source cross-check |

이 표의 경로만 검사하고 끝내면 안 된다. `git diff --name-status <base>...HEAD`와 untracked
목록에서 새 owner/consumer/tooling path를 추가하고, 삭제·이동된 path의 replacement authority를
확인한다.

## 3. 감사 시작 전 source freeze

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| PRE-01 | M | HEAD·branch·remote base 고정 | `git rev-parse HEAD`, branch, 비교 base를 보고서에 기록 |
| PRE-02 | M | dirty/untracked 소유권 | `git status --porcelain=v1 --untracked-files=all`; 기존 변경과 감사 변경을 분리 |
| PRE-03 | M | source identity | tracked+untracked path/content/mode digest와 paths count 기록 |
| PRE-04 | M | toolchain | `rustc -Vv`, `cargo -V`, Python/uv, OS/arch 기록 |
| PRE-05 | R | lockfile·dependency identity | `Cargo.lock` digest, advisory DB revision, cargo-deny version 기록 |
| PRE-06 | R | 증거 시간 경계 | 모든 실행에 start/end, exit code, command, selected/executed count 기록 |
| PRE-07 | R | 병렬 실행 오염 | mutation/perf/coverage/TSan은 같은 checkout·target·CPU lane에서 경쟁 실행하지 않음 |
| PRE-08 | R | source 이동 감지 | 장시간 실행 전후 HEAD·paths digest·dirty 상태 동일 |
| PRE-09 | M | 기존 receipt 재사용 금지 조건 | schema/source/toolchain/gate inventory 중 하나라도 다르면 historical로 분류 |
| PRE-10 | M | 감사 범위 선언 | architecture/API/engine/host/tests/perf/CI/docs/external 중 포함·제외를 명시 |

기본 수집 명령:

```bash
git rev-parse HEAD
git rev-parse --abbrev-ref HEAD
git status --porcelain=v1 --untracked-files=all
rustc -Vv
cargo -V
cargo metadata --format-version 1 --no-deps
```

## 4. 제품 범위와 계약

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| SCP-01 | M | 용도 일치 | 분석/service runtime의 process-local governed execution임을 README/spec/API가 동일하게 말함 |
| SCP-02 | M | semantic policy와 worker governance 분리 | class policy가 worker 수·pool 생성의 암묵적 owner가 아님 |
| SCP-03 | M | product-neutral public API | public type/variant/field에 consumer 제품 taxonomy가 없음 |
| SCP-04 | M | fail-closed default | unknown/disabled/malformed class·task가 default class로 admit되지 않음 |
| SCP-05 | M | topology와 policy 분리 | topology capacity와 class semantics가 독립 검증되고 오류가 typed |
| SCP-06 | M | capability-pool topology | built-in/extension substrate가 명시된 capability pool에만 매핑됨 |
| SCP-07 | M | engine-specific pool 금지 | 특정 algorithm/class 이름의 private worker pool이 생기지 않음 |
| SCP-08 | M | competing queue 단일성 | admission 앞뒤에 inventory 밖의 무제한 channel/semaphore wait가 없음 |
| SCP-09 | R | non-goal 명시 | cross-process persistence, killable blocking work, undeclared cross-root cycles 등 비보장 범위가 문서화됨 |
| SCP-10 | R | 계약-구현-테스트 추적 | 각 hard rule이 owner source, public surface, positive/negative test에 연결됨 |

## 5. 아키텍처와 의존 방향

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| ARC-01 | M | 4-crate 책임 | contract/engine/host/rayon 책임이 Cargo graph와 실제 import에 일치 |
| ARC-02 | M | inward dependency | engine은 Tokio/Rayon을 모르고 contract는 serde 외 runtime dependency가 없음 |
| ARC-03 | M | port ownership | `Runtime`, `CpuExecutor`, `Clock`, `PermitWaker`의 owner가 contract로 단일화됨 |
| ARC-04 | M | facade 통제 | 외부 사용자는 `taskmesh`만으로 지원 표면을 사용 가능 |
| ARC-05 | M | adapter honesty | adapter capability declaration이 실제 worker/pool semantics보다 강하지 않음 |
| ARC-06 | M | state authority | permit/resource/fairness 상태의 authoritative writer가 engine 하나 |
| ARC-07 | M | deterministic collections | observable ordering에 `HashMap` iteration/random polling order를 사용하지 않음 |
| ARC-08 | M | effects outside lock | clock read, waker/callback/drop, executor spawn 같은 host effect가 engine mutex 밖 |
| ARC-09 | R | feature topology | default와 `rayon` feature가 API·capacity 계약을 의도치 않게 바꾸지 않음 |
| ARC-10 | R | architecture ratchet | `just test-architecture`가 금지 dependency/import뿐 아니라 새 crate/path도 검사 |

필수 실행: `just test-architecture`, `cargo tree --workspace`, `cargo test -p taskmesh --features rayon`.

## 6. 구성·정책 검증

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| CFG-01 | M | build-time rejection | 불가능한 budget/topology/policy는 첫 admit이 아니라 `build`/`Governor::new`에서 거절 |
| CFG-02 | M | zero 의미 | limit 0이 disabled/unlimited 중 무엇인지 field마다 단일 의미이며 test가 있음 |
| CFG-03 | M | integer exactness | 합산·곱셈·conversion이 overflow/saturation으로 capacity를 늘리지 않음 |
| CFG-04 | M | per-request ceiling | class cost와 request override가 global/per-request ceiling을 우회하지 못함 |
| CFG-05 | M | fallback 검증 | fallback 존재·enabled·non-self·one-hop·resource scaling을 검증 |
| CFG-06 | M | fairness compatibility | 한 경쟁 domain에 섞을 수 없는 fairness tier/policy를 fail-closed 거절 |
| CFG-07 | M | cancellation compatibility | deadline을 지원하지 않는 class에 deadline을 주면 typed rejection |
| CFG-08 | M | substrate allowlist | name/kind/pool 조합이 SSOT inventory와 정확히 일치 |
| CFG-09 | R | config roundtrip | JSON deserialize→validate→serialize에서 의미와 exact integer 보존 |
| CFG-10 | R | malicious config bounds | 과도한 class/stage/string 크기와 duplicate key 처리 정책이 명시됨 |

## 7. Admission·queue·backpressure

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| ADM-01 | M | unknown/disabled rejection | 계상·queue·retry hint 없이 정확한 verdict로 종료 |
| ADM-02 | M | atomic admission | class inflight, CPU, memory, capability pool 판정과 charge가 한 transition |
| ADM-03 | M | bounded queue | class별 max depth가 실제 queue insert 전에 강제됨 |
| ADM-04 | M | total outstanding bound | host의 숨은 wait까지 포함해 `outstanding = queued + inflight` 계약이 참 |
| ADM-05 | M | queue vs shed | overflow policy별 Queue/Reject/Shed가 typed하고 테스트로 구분됨 |
| ADM-06 | M | head freeze | intake에서 고른 substrate/capability가 promotion 때 임의로 바뀌지 않음 |
| ADM-07 | M | head-of-line 예외 | 다른 pool에 막힌 head만 문서화된 범위에서 overtake 가능 |
| ADM-08 | M | promotion trigger | release/reconcile/cancel/abandon/capacity 변화가 필요한 queue를 event-driven promote |
| ADM-09 | M | bounded promotion | 한 transition의 promotion work가 상한을 가지며 continuation이 보장됨 |
| ADM-10 | M | budget-before-selection | promotion budget 소진 후 scheduler debt/cursor/virtual time을 먼저 변경하지 않음 |
| ADM-11 | M | admission lock timeout | acquire budget이 admission mutex와 substrate wait를 포함 |
| ADM-12 | M | late admission unwind | deadline 뒤 도착한 `Admitted`는 실행하지 않고 모든 charge를 반납 |
| ADM-13 | R | overload stability | steady overload에서 queue/memory/latency가 무한 증가하지 않고 useful goodput 유지 |
| ADM-14 | R | retry amplification | retry hint가 즉시 synchronized retry storm을 유도하지 않으며 consumer 책임이 문서화됨 |

## 8. Permit lifecycle·custody·회계

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| LIF-01 | M | phase state machine | Reserved→Accepted→Running→Cleanup/Terminal의 허용 전이만 존재 |
| LIF-02 | M | phase conservation | `inflight == reserved + accepted + running + cleanup_pending` |
| LIF-03 | M | cumulative conservation | admitted/started/terminated counter 관계가 모든 실패 경로에서 보존 |
| LIF-04 | M | exactly-once release | normal/error/panic/cancel/drop/spawn-failure에서 permit·pool·resource가 한 번만 반환 |
| LIF-05 | M | typed duplicate release | unknown/reclaimed/already-released가 silent success가 아님 |
| LIF-06 | M | lease ownership | 실행 custody 취득 후 raw permit ID로 release할 수 없고 token proof가 필요 |
| LIF-07 | M | lease uniqueness | governor/process 경계를 포함해 token collision/forgery가 capacity를 풀지 못함 |
| LIF-08 | M | response vs custody | caller 응답 종료가 worker 종료나 resource release로 오인되지 않음 |
| LIF-09 | M | caller future drop | future drop 후에도 실제 worker custody와 accounting이 끝까지 유지 |
| LIF-10 | M | spawn/setup failure | 시작 전 실패는 `started`로 세지 않고 reserved capacity를 회수 |
| LIF-11 | M | panic isolation | panic이 typed governor outcome이 되며 다른 waiter cleanup을 막지 않음 |
| LIF-12 | M | terminal ticket | promoted/reaped ticket이 무한 Pending이나 dead permit ID로 남지 않음 |
| LIF-13 | M | terminal retention bound | terminal history가 상한과 deterministic eviction을 가짐 |
| LIF-14 | D | linearizability oracle | 동시 admit/claim/release/close를 reference model 또는 model checker와 매 op 비교 |

## 9. Fairness scheduler

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| FAIR-01 | M | policy별 독립 oracle | FIFO/WFQ/DRR/EDF/scavenger가 구현 코드와 다른 reference로 검증됨 |
| FAIR-02 | M | asymmetric fixtures | weight/quantum/cost/deadline이 서로 다른 입력으로 priority와 fairness를 구분 |
| FAIR-03 | M | DRR active ring | runnable class ring, deficit carry/debit, wrap 순서가 결정적 |
| FAIR-04 | M | DRR idle reset | queue가 빈 class의 오래된 credit이 새 burst에 특혜를 주지 않음 |
| FAIR-05 | M | DRR bounded work | cost/quantum 크기와 무관하게 selection이 classes 수에 bounded |
| FAIR-06 | M | WFQ numeric domain | `1..=u32::MAX` weight/cost에서 increment 0, overflow, precision collapse 없음 |
| FAIR-07 | M | cancellation debt | cancel/abandon이 이미 받은 service debt를 지우지 않음 |
| FAIR-08 | M | fully-runnable selection | semantic resource와 capability pool을 모두 만족한 후보만 scheduler state 변경 |
| FAIR-09 | M | in-class FIFO | promotion budget 경계와 newcomer race에서도 같은 class의 앞선 runnable head 보존 |
| FAIR-10 | M | deadline ordering | absolute deadline 동률·없음·clock regression에서 stable tie-break |
| FAIR-11 | M | starvation bound | 지속 backlog와 mixed cost에서 각 eligible class의 service gap 상한을 실측 |
| FAIR-12 | R | multi-resource interaction | CPU-heavy/memory-heavy/pool-heavy class가 single-resource fairness 착시로 굶지 않음 |
| FAIR-13 | R | fairness metric honesty | Jain index·service ratio의 분모와 window가 명시되고 rejected work를 섞지 않음 |

## 10. Memory·CPU·capability governance

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| RES-01 | M | exact aggregates | CPU/memory held aggregate가 wide integer로 exact하며 saturation으로 결함을 숨기지 않음 |
| RES-02 | M | reservation facts 분리 | original estimate, remaining reservation, measured/effective 사용량이 혼합되지 않음 |
| RES-03 | M | reconcile direction | downward reconcile은 budget을 풀고 promotion, upward reconcile은 overcommit 정책을 따름 |
| RES-04 | M | no resurrection | stage release 뒤 reconcile이 반환한 memory를 다시 charge하지 않음 |
| RES-05 | M | epoch/sequence | stale·duplicate·out-of-order measurement/release report를 거절 |
| RES-06 | M | release policy | `OnTaskCompletion`은 stage release를 typed refusal, 허용 mode만 partial release |
| RES-07 | M | leak sweep eligibility | executor가 custody를 가진 work를 stale 시간만으로 회수하지 않음 |
| RES-08 | M | leak activity clock | claim/lease activity와 measurement epoch 갱신이 해당 transition 안에서 atomic |
| RES-09 | M | degrade accounting | fallback은 resource class만 재분류하고 deadline/cancel/provenance 의미는 원 class 유지 |
| RES-10 | M | pool authority | semaphore 실제 permit과 engine pool counter가 독립 double authority가 아님 |
| RES-11 | R | capacity calibration | topology worker 수와 engine pool limit의 관계가 adapter capability와 일치 |
| RES-12 | D | adversarial resource mix | CPU/memory/pool 병목의 모든 조합과 simultaneous release/reconcile을 property test |

## 11. Runtime adapter·executor 경계

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| RUN-01 | M | path/substrate match | `run_io`, `run_blocking`, `run_cpu`, `run_local`, requested-stack 경로가 올바른 substrate만 사용 |
| RUN-02 | M | event-loop blocking 금지 | sync/CPU/large-stack 작업이 Tokio async worker를 직접 block하지 않음 |
| RUN-03 | M | permit-before-spawn | worker/task를 만든 뒤 capacity를 기다리는 unbounded fan-out이 없음 |
| RUN-04 | M | spawn result handshake | setup 성공·worker start·task complete가 서로 다른 사건으로 관측됨 |
| RUN-05 | M | executor declaration | declared worker count, shared/exclusive, stack/cancel capability가 실제와 일치 |
| RUN-06 | M | Rayon opt-in | Rayon feature가 하나의 shared CPU adapter만 만들고 class별 pool을 만들지 않음 |
| RUN-07 | M | default fallback | Rayon 미사용 시 CPU fallback의 pool·blocking 의미가 문서/스냅샷과 일치 |
| RUN-08 | M | LocalSet affinity | non-`Send` future가 caller local task/thread 계약을 깨지 않음 |
| RUN-09 | M | requested stack | stack request는 `large_stack` pool을 소비하고 thread/runtime teardown이 bounded |
| RUN-10 | M | channel bounds | host 내부 mpsc/oneshot은 inventory에 없는 backlog를 만들지 않음 |
| RUN-11 | M | `select!` cancellation safety | queue 위치를 가진 future를 반복 select에서 drop/recreate해 progress를 잃지 않음 |
| RUN-12 | M | task abort semantics | Tokio abort 불가능한 blocking work를 취소됐다고 보고하거나 조기 release하지 않음 |
| RUN-13 | R | runtime shutdown behavior | runtime/handle drop 시 in-flight work의 documented outcome과 accounting이 일치 |

## 12. Cancellation·timeout·deadline·retry

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| TIME-01 | M | pre-submit cancel | 어떤 charge/queue/spawn도 만들지 않고 typed cancel |
| TIME-02 | M | queue cancel | ticket 제거, scheduler debt 보존, 뒤 작업 promotion, waker retirement |
| TIME-03 | M | accepted/running cancel | caller response와 worker custody를 분리하고 cooperative policy만 발효 |
| TIME-04 | M | absolute deadline propagation | admission lock→queue→substrate→execution에 하나의 absolute deadline 사용 |
| TIME-05 | M | zero budget | `Duration::ZERO`는 try-once이며 sleep이나 한 tick 유예가 없음 |
| TIME-06 | M | boundary semantics | deadline 직전/동시/직후 completion 판정이 단일 선형화 규칙 |
| TIME-07 | M | no late success | deadline 이후 도착한 worker result를 `Ok`로 반환하지 않음 |
| TIME-08 | M | timeout cause preservation | class/cpu/memory/pool/permit 대기 원인이 generic timeout으로 소실되지 않음 |
| TIME-09 | M | retry hint domain | retry-after가 0/overflow/비단조 clock에 깨지지 않고 saturation 원인에 맞음 |
| TIME-10 | R | retry ownership | Taskmesh가 자동 retry를 하지 않는다면 consumer 책임·idempotency·jitter 요구를 문서화 |
| TIME-11 | D | race matrix | cancel/deadline/complete/release/drop의 모든 2-way·핵심 3-way race를 model/host test |

## 13. Composite·fan-out·root attribution·nested wait

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| CMP-01 | M | well-formed stages | 0 stage, class mismatch, duplicate/invalid sequence를 admission 전에 거절 |
| CMP-02 | M | deterministic reduce required | fan-out에 reduce policy가 없으면 `MalformedTask` |
| CMP-03 | M | stable reduce key | sort key·tie break·error aggregation·partial ordering이 전부 명시됨 |
| CMP-04 | M | completion-order independence | worker completion 순서를 바꿔도 동일 input은 byte/semantic 동일 결과 |
| CMP-05 | M | root attribution | child permit/resource/error/terminal counter가 정확한 root operation에 귀속 |
| CMP-06 | M | recursive admission | 금지된 재귀와 허용된 child/sibling submission이 구분됨 |
| CMP-07 | M | declared nested wait | parent가 awaited child를 선언하고 자기 root만 보유한 capacity에 막히면 즉시 typed reject |
| CMP-08 | M | no false cycle | sibling/stranger/shared capacity/degrade 가능 경로는 cycle로 오판하지 않음 |
| CMP-09 | M | all resource kinds | class inflight, pool, CPU, memory 각각 nested-cycle 판정이 whole-block 조건을 확인 |
| CMP-10 | R | unsupported cycles | undeclared/cross-root cycle 범위와 consumer mitigation이 명시됨 |
| CMP-11 | D | large fan-out bound | stage/fan-out metadata와 reduce temporary allocation의 상한·복잡도 측정 |

## 14. Shutdown·drain·recovery

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| SHUT-01 | M | close linearization | admission close가 admit과 같은 authority/lock 아래 one-way transition |
| SHUT-02 | M | post-close rejection | close 뒤 제출은 queue/charge 없이 동일 typed verdict |
| SHUT-03 | M | drain scope | queued와 모든 custody phase가 0일 때만 성공 |
| SHUT-04 | M | event-driven wait | drain이 polling/busy-spin/fixed sleep에 의존하지 않음 |
| SHUT-05 | M | bounded timeout | timeout 시 class별 outstanding과 elapsed를 반환하고 runtime은 draining 유지 |
| SHUT-06 | M | concurrent close/admit | 어느 쪽이 선형화되어도 conservation과 typed outcome 보존 |
| SHUT-07 | M | wake coverage | release/abandon/terminal/cancel이 drain waiter를 빠짐없이 깨움 |
| SHUT-08 | R | teardown order | consumer runbook이 stop intake→drain→drop executor/runtime 순서를 고정 |
| SHUT-09 | R | crash boundary | process crash 시 미완료 작업 비지속성 또는 외부 replay owner가 명시됨 |

## 15. Snapshot·inventory·observability

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| OBS-01 | M | one-snapshot consistency | class/phase/resource/pool 값이 한 authority snapshot이며 서로 다른 시점 조합이 아님 |
| OBS-02 | M | deterministic ordering | classes, pools, substrates, ledgers가 stable order |
| OBS-03 | M | conservation helper | snapshot 자체가 위반을 검출하고 정상/negative fixture가 있음 |
| OBS-04 | M | typed outcomes | rejection, setup failure, panic, abandon, reclaimed lease를 서로 구분 |
| OBS-05 | M | cumulative vs gauge | monotonic counter와 현재 gauge를 이름·타입·문서에서 구분 |
| OBS-06 | M | exact wire integer | `u128` aggregate가 JSON number precision에 손실되지 않음 |
| OBS-07 | M | schema versioning | Snapshot/receipt/telemetry schema 변화에 version·migration·consumer test |
| OBS-08 | R | low cardinality | operation/root IDs를 metric label로 사용하지 않고 class/verdict/pool은 bounded inventory |
| OBS-09 | R | rejection visibility | overload/degrade/timeout/cancel 비율과 원인이 운영자가 구분 가능 |
| OBS-10 | R | sensitive data | source/reason/operation 문자열이 log·receipt에 무제한 또는 비식별 없이 노출되지 않음 |
| OBS-11 | R | inventory backing | snapshot inventory가 장식이 아니라 runtime capacity/dispatch 결정의 입력 |
| OBS-12 | D | telemetry compatibility | metric/event 이름 변경을 public telemetry schema 변화로 취급 |

## 16. Public API·wire·compatibility

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| API-01 | M | facade-only examples | README/external interface/CHANGELOG 예제가 `taskmesh` facade로 compile |
| API-02 | M | typed error separation | governor failure와 user task error가 `RunError<E>`에서 손실 없이 구분 |
| API-03 | M | `#[must_use]` outcomes | release/advance/stage-release처럼 무시하면 위험한 결과가 must-use |
| API-04 | M | enum evolution | downstream exhaustive match 파괴 위험과 `non_exhaustive` 정책이 일관됨 |
| API-05 | M | serde compatibility | field rename/default/unknown-field 정책과 `0.x` migration을 명시 |
| API-06 | M | feature additivity | `rayon` enable이 기존 public type 의미나 method availability를 파괴하지 않음 |
| API-07 | R | semver diff | 직전 릴리스 대비 `cargo semver-checks`와 human behavioral review |
| API-08 | R | MSRV | clean external fixture가 Rust 1.81 default와 rayon surface를 compile |
| API-09 | R | supported platform | OS/arch/nightly-only proof와 실제 library support를 혼동하지 않음 |
| API-10 | R | changelog migration | 모든 breaking change에 before/after, failure mode, rollout/rollback 설명 |
| API-11 | R | docs compile | doctest, extracted Markdown code, rustdoc link 모두 current API와 일치 |
| API-12 | D | downstream source audit | 실제 consumer의 config/taxonomy/error handling이 새 contract와 일치 |

## 17. 동시성 증명

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| CON-01 | M | production seam | Loom/Shuttle test가 toy replica가 아니라 production `Governor` transition 실행 |
| CON-02 | M | seam completeness | lock/atomic/Arc/thread 중 model 밖 primitive를 목록화하고 한계를 기록 |
| CON-03 | M | deterministic handshake | race test는 barrier/oneshot/state witness 사용; fixed sleep만으로 interleaving 주장 금지 |
| CON-04 | M | bounded tests | hang/deadlock test에 외부 상한과 release-on-drop cleanup |
| CON-05 | M | callback reentry | waker/drop/executor callback의 reentrant governor call이 deadlock하지 않음 |
| CON-06 | M | poisoned/panic path | panic 뒤 모든 waiter와 capacity가 관측 가능한 terminal 상태 |
| CON-07 | R | Loom exhaustive claim | max threads/branches/preemption/permutations와 pruned state를 receipt에 기록 |
| CON-08 | R | randomized schedules | Shuttle seed, schedule count, failing seed replay 방법 기록 |
| CON-09 | R | real memory model | TSan이 engine과 host adapter의 실제 thread/task 경로를 포함 |
| CON-10 | D | stress/soak | 반복 횟수·시간·CPU·failure count·quiescence를 기록하고 flaky retry로 숨기지 않음 |

## 18. 테스트·mutation·fuzz·coverage 품질

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| TST-01 | M | reachable production path | test가 helper replica가 아니라 public/owner entrypoint를 통과 |
| TST-02 | M | independent oracle | 구현과 같은 helper/formula를 복사해 self-oracle을 만들지 않음 |
| TST-03 | M | anti-vacuity | selected/executed/op count가 양수이고 expected branch를 실제 통과 |
| TST-04 | M | precise failure | broad `is_err`/panic 대신 정확 variant·fields·state conservation을 단언 |
| TST-05 | M | negative control | 정상 control이 같은 fixture에서 PASS하고 결함 하나만 넣으면 named reason으로 FAIL |
| TST-06 | M | deterministic timing | sleep-only, wall-clock luck, scheduler luck에 의존하지 않음 |
| TST-07 | M | failure cleanup | timeout/panic 후 thread/task/process와 mutated source가 복구됨 |
| TST-08 | M | feature/platform collection | default/rayon/nightly/Linux 조건에서 실제 test count가 0이 아님 |
| TST-09 | M | curated mutation integrity | full inventory digest·ID·runner·source scope·control·status counts 결박 |
| TST-10 | M | mutation outcome semantics | KILLED/CONTROL_GREEN/SURVIVED/WRONG_REASON/TIMEOUT/UNVIABLE를 합산하지 않음 |
| TST-11 | R | generated mutation disclosure | cargo-mutants 실행 여부·denominator·caught/missed/unviable/timeout을 별도 보고 |
| TST-12 | R | survivor adjudication | equivalent 선언마다 source reachability와 semantic equivalence 근거·reviewer |
| TST-13 | R | coverage completeness | line/region/function/instantiation/branch/MCDC의 collected 여부를 모두 표시 |
| TST-14 | R | coverage interpretation | percentage를 행동 정확성이나 mutation score로 승격하지 않음 |
| TST-15 | R | fuzz corpus | production transition, builder/config, wire format target이 current API에 compile |
| TST-16 | R | fuzz invariants | 매 step conservation/bounds/provenance와 마지막 quiescence 검증 |
| TST-17 | D | generated full sweep | clean copied tree, workspace consumer tests, package별 denominator와 raw outcomes 보존 |
| TST-18 | D | test duplication | 같은 SUT edge/oracle/failure class의 중복과 unit/integration layering을 구분 |

## 19. 성능·확장성·과부하

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| PERF-01 | M | benchmark validity | benchmark가 실제 operation을 수행하고 compiler-elided/zero-op가 아님 |
| PERF-02 | M | allocation self-check | allocator counter가 known allocation을 보며 분모 operation count가 양수 |
| PERF-03 | R | raw + corrected latency | open-loop/raw와 coordinated-omission-corrected histogram을 둘 다 보존 |
| PERF-04 | R | latency decomposition | queue/admission/substrate/execution/end-to-end 시간을 가능한 범위에서 분리 |
| PERF-05 | R | percentiles | p50/p95/p99/p99.9, max, sample count, dropped/synthetic sample 기록 |
| PERF-06 | R | goodput | accepted가 아니라 정상 completion 기준 throughput; reject/error/cancel 별도 |
| PERF-07 | R | overload curve | offered load를 saturation 전후로 올려 queue/reject/goodput/tail/fairness를 함께 측정 |
| PERF-08 | R | burst model | steady Poisson뿐 아니라 burst/MMPP와 trace replay 포함 |
| PERF-09 | R | mixed workloads | IO/blocking/CPU/local/large-stack과 heterogeneous cost/class 비율 포함 |
| PERF-10 | R | scalability | worker/concurrency 증가에 대한 contention과 USL domain/fit residual 기록 |
| PERF-11 | R | deterministic metric | Linux IAI instruction count와 wall-clock 결과를 별도 baseline으로 관리 |
| PERF-12 | R | baseline comparability | toolchain/CPU/OS/governor/bench/dependency fingerprint가 같은 baseline만 비교 |
| PERF-13 | R | cold/warm 분리 | compile/cache warmup과 runtime steady state를 섞지 않음 |
| PERF-14 | D | pathological complexity | queue depth/classes/stages/terminal history 최대치에서 time/memory complexity 실측 |
| PERF-15 | D | noisy-neighbor test | 다른 class/pool의 saturation이 무관한 class latency/capacity를 침범하는지 측정 |

## 20. 안전성·공급망·입력 경계

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| SEC-01 | M | unsafe inventory | production `unsafe`는 최소화·근거·Miri/targeted test; benchmark allocator와 구분 |
| SEC-02 | M | panic surface | consumer-controlled config/input이 panic/abort 대신 typed error |
| SEC-03 | M | memory amplification | queue count 외 task metadata/string/fan-out/terminal record 크기도 bounded 또는 owner 명시 |
| SEC-04 | M | lock amplification | user callback/drop/waker/formatting을 lock 아래 실행하지 않음 |
| SEC-05 | M | secret/PII handling | operation/source/reason이 Debug/log/snapshot에 노출되는 계약과 redaction owner 명시 |
| SEC-06 | R | dependency policy | license/advisory/source/duplicate 정책이 `cargo deny` config와 일치 |
| SEC-07 | R | workflow pinning | third-party Actions version/pinning과 permissions 최소화 검토 |
| SEC-08 | R | artifact integrity | receipt/benchmark/baseline artifact가 exact SHA와 digest로 결박 |
| SEC-09 | R | DoS behavior | unknown-class flood, queue-full flood, huge config, cancel storm에서 bounded work |
| SEC-10 | D | sanitizer matrix | TSan 외 필요 시 Miri/ASan/LSan 적용 가능성과 제외 근거 기록 |

## 21. CI·gate·receipt 무결성

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| CI-01 | M | gate SSOT parity | Justfile, workflow, inventory, required set이 1:1 |
| CI-02 | M | required skip fail | required gate의 NOT_RUN/SKIPPED/TIMEOUT이 PASS가 아님 |
| CI-03 | M | status-line contract | self-reporting gate의 마지막 정확 marker를 receipt가 보존 |
| CI-04 | M | stale artifact defense | runner 시작 전에 이전 sidecar/output을 제거하고 미생성 시 fail |
| CI-05 | M | summary recomputation | result append 후 qualified/not-run/not-passed를 결과에서 재계산 |
| CI-06 | M | detailed evidence binding | derived result가 detail receipt canonical digest·exit·duration에 결박 |
| CI-07 | M | duplicate/malformed result | duplicate ID, unknown shape, count/scope/status mismatch를 fail-closed |
| CI-08 | M | mutation scope honesty | curated와 generated sweep을 schema와 문구 모두에서 분리 |
| CI-09 | M | local vs hosted | local collection은 exact-source evidence지만 `QUALIFIED`를 발급하지 않음 |
| CI-10 | R | hosted attestation | Actions/CI/SHA/workspace/clean source check가 exact key set으로 모두 true |
| CI-11 | R | pre/post source identity | gate 실행 전후 path digest와 dirty 상태 동일 |
| CI-12 | R | platform completeness | Linux-only IAI와 nightly/TSan/fuzz가 실제 tool 설치 후 실행됨 |
| CI-13 | R | artifact retention | combined/gates/mutations/coverage/perf raw artifact를 실패 시에도 upload |
| CI-14 | R | no duplicate authority | consumer-MSRV 같은 gate를 receipt collector가 별도 재실행·재해석하지 않음 |
| CI-15 | R | branch protection | qualification job이 required이며 관리자 bypass/재실행 정책이 기록됨 |
| CI-16 | D | negative CI fixtures | stale sidecar, forged summary, wrong SHA, dirty tree, missing tool, zero tests를 실제로 red 처리 |

필수 실행:

```bash
just gates-inventory
just gate
just matrix
just release
uv run python tools/qualification/receipt.py validate <receipt>
```

`just release`의 성공은 local gate proof다. 릴리스 자격은 clean hosted checkout에서
clean local Linux에서 `collect --local-qualified`가 만든 exact-SHA receipt만 인정한다.

## 22. 문서·릴리스·외부 consumer·운영

| ID | 등급 | 검사항목 | PASS 조건 / 필수 증거 |
|---|---:|---|---|
| OPS-01 | M | source/document agreement | README/spec/ADR/interface의 현재 의미가 production source와 일치 |
| OPS-02 | M | historical evidence label | 과거 수치/receipt/commit을 current qualification처럼 쓰지 않음 |
| OPS-03 | M | examples compile | README/interface/CHANGELOG 코드가 facade against compile |
| OPS-04 | R | release checklist | 모든 항목에 current exact-SHA 결과 또는 NOT_RUN/BLOCKED 이유 |
| OPS-05 | R | rollout | opt-in/config migration, capacity defaults, consumer adoption 순서 명시 |
| OPS-06 | R | rollback | API/config/schema rollback 가능성, incompatible state/telemetry 처리 명시 |
| OPS-07 | R | overload runbook | queue/reject/degrade/drain 지표와 operator action, disable/tune 경로 |
| OPS-08 | R | graceful shutdown runbook | intake close→drain→timeout 판단→process termination 순서 |
| OPS-09 | R | external consumer | 실제 consumer SHA/config/feature/MSRV와 Taskmesh SHA를 함께 기록 |
| OPS-10 | R | activation boundary | merge/package/deploy/config-enable/runtime-observation을 별도 상태로 보고 |
| OPS-11 | D | rollback rehearsal | representative consumer에서 이전 version/config로 실제 복귀 |
| OPS-12 | D | incident injection | worker panic, capacity loss, overload, stuck blocking job, clock anomaly 대응 훈련 |

## 23. 필수 시나리오 행렬

아래 행은 unit test 하나의 존재가 아니라 engine→host→public outcome→snapshot 보존을 확인한다.

| 시나리오 | Engine | Tokio host | Rayon | Snapshot | Mutation/model |
|---|---:|---:|---:|---:|---:|
| unknown/disabled/malformed reject | 필수 | 필수 | N/A | 필수 | 필수 |
| class queue full·shed | 필수 | 필수 | N/A | 필수 | 필수 |
| CPU/memory/pool 동시 포화 | 필수 | 필수 | 조건부 | 필수 | 필수 |
| cross-pool head-of-line | 필수 | 필수 | 조건부 | 필수 | 필수 |
| FIFO/WFQ/DRR/EDF 비대칭 경쟁 | 필수 | 경계 | 조건부 | 필수 | 필수 |
| cancel: pre/queued/accepted/running | 필수 | 필수 | 조건부 | 필수 | 필수 |
| deadline: lock/queue/pool/run/boundary | 필수 | 필수 | 조건부 | 필수 | 필수 |
| caller future drop | 필수 | 필수 | 조건부 | 필수 | 필수 |
| spawn/setup failure·worker panic | 필수 | 필수 | 필수 | 필수 | 필수 |
| memory reconcile/stage release/stale epoch | 필수 | ext 경계 | N/A | 필수 | 필수 |
| leak sweep vs executor custody | 필수 | 필수 | 조건부 | 필수 | 필수 |
| fan-out completion permutation | 필수 | 필수 | 조건부 | root 필수 | 필수 |
| declared nested wait / false-positive cases | 필수 | 필수 | 조건부 | 필수 | 필수 |
| admission close race·drain timeout | 필수 | 필수 | 조건부 | 필수 | 필수 |
| clock regression·integer maximum | 필수 | 경계 | N/A | 필수 | property |
| steady overload·burst·recovery | simulation | 필수 | 조건부 | 필수 | load/soak |

`경계`는 engine semantics가 owner이고 host는 전달·custody를 검증한다는 뜻이다. `조건부`는
`rayon` feature가 켜진 release에는 필수다.

## 24. 표준 감사 실행 순서

1. `PRE-*`를 완료하고 source snapshot을 freeze한다.
2. 문서와 public API에서 주장 목록을 추출한다.
3. `ARC/CFG/SCP`로 owner와 dependency 방향을 확정한다.
4. `ADM/LIF/FAIR/RES`를 source transition 단위로 정독한다.
5. `RUN/TIME/CMP/SHUT`의 engine→host reachable path를 추적한다.
6. `OBS/API/SEC`에서 외부 노출과 operational failure mode를 검사한다.
7. 가장 좁은 focused test로 후보 finding을 재현한다.
8. 실제 결함에는 동일 단일 편집 mutation 또는 독립 model oracle을 요구한다.
9. `CON/TST/PERF` heavy rail은 같은 호스트에서 직렬 실행한다.
10. clean committed SHA의 local Linux에서 full qualification을 수집한다.
11. `OPS-*` consumer·rollout·activation을 repository qualification과 분리한다.

## 25. 감사 보고서 템플릿

```markdown
# Taskmesh Audit — <date>

## Source snapshot
- branch:
- HEAD:
- base:
- dirty paths and owner:
- paths digest/count:
- toolchain/platform:
- audit profile: M | R | D

## Verdict
- repository implementation: PASS | FAIL | PARTIAL
- local exact-source qualification: QUALIFIED | NOT_QUALIFIED | NOT_RUN
- external consumer/activation: VERIFIED | UNVERIFIED | BLOCKED_EXTERNAL

## Findings
| ID | Severity | Checklist IDs | Reachable path | Reproduction | Owner | Status |
|---|---|---|---|---|---|---|

## Checklist results
| Checklist ID | Result | Evidence | Limitations |
|---|---|---|---|

## Evidence ledger
| Command/artifact | Source SHA/digest | Selected/executed | Exit | Result |
|---|---|---:|---:|---|

## Remaining proof gaps
- NOT_RUN:
- platform skipped:
- external:
- generated mutation survivors/unviable/timeouts:

## Release decision
- decision: GO | NO_GO | CONDITIONAL
- exact blockers:
- rollback condition:
```

## 26. 최종 판정 규칙

- `P0/P1 FAIL`이 하나라도 있으면 `NO_GO`다.
- required `M/R` 항목의 `PARTIAL`, `NOT_RUN`, `SKIPPED`, `BLOCKED_EXTERNAL`은 자동 PASS가 아니다.
- coverage가 높아도 mutation survivor, weak oracle, zero test, unreachable adapter를 덮지 못한다.
- curated mutation 100%는 등록된 fault만 증명하며 generated cargo-mutants score가 아니다.
- Loom/Shuttle은 model seam 밖의 Tokio/OS memory behavior를 증명하지 않는다.
- platform-scoped local proof는 full local qualification이 아니고, qualification은 consumer activation이 아니다.
- 문서에 공개된 non-goal은 `N/A` 근거가 될 수 있지만 reachable corruption/deadlock을 정당화하지 못한다.
- 최종 `GO`에는 clean exact-SHA `QUALIFIED` receipt와 release 대상 feature/platform의 필수 rail이 필요하다.
