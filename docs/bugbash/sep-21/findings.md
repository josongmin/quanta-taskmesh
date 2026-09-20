# SEP-21 Taskmesh bugbash findings

## 판정과 범위

- 상태: **FINAL AUDIT / OPEN / release NO-GO**
- 최종 대조일: 2026-09-21
- 확정 open finding: **23건 — P0 3, P1 16, P2 4**
- 감사 기준: `hardening/sep-16`의 working tree, HEAD
  `1f04ccb245fc631507df780b97768f8d2f0e5e1a`, tree
  `65152e1f97e5b691df7ffe946c43a25bc7c61ee5`
- source 상태: tracked dirty 19개, audit 입력 untracked 1개(체크리스트), audit 산출물
  untracked 17개(finding·ticket plan 포함).
  이 보고서는 기존 변경을 수정하거나
  clean/exact-SHA qualification으로 승격하지 않는다.
- 범위: public contract, engine admission/promotion, Tokio host, capability topology,
  mutation/performance gate, qualification receipt
- 포함 기준: 현재 소스에서 도달 가능한 실패 경로, repository의 명시적 architecture
  rule 위반, false qualification 또는 release authority 상실을 만드는 검증기 결함만
  포함한다.
- 제외 기준: 단순 커버리지 부족, 일반적인 개선 제안, 재현되지 않은 운영 가정은
  포함하지 않는다.
- 이 문서는 수정 완료 증거가 아니다. 아래 항목은 모두 미해결로 취급한다.
- 구조적 RCA, dependency, write lane과 ticket별 작업 계획:
  [`tickets/README.md`](tickets/README.md)

## 요약

| ID | 우선순위 | 영역 | 체크리스트 | 결함 |
| --- | --- | --- | --- | --- |
| TM21-001 | P0 | nested work | CMP-06/07/08/09, LIF-04, SHUT-05 | child가 보유한 permit을 lineage에서 제외해 descendant wait cycle을 놓침 |
| TM21-002 | P0 | admission | ADM-08, CMP-07/09, FAIR-12, CON-06 | 첫 blocker 하나만 보아 복수 blocker 뒤의 wait cycle을 놓침 |
| TM21-003 | P0 | topology | SCP-02/05/08, ARC-05, RES-10/11, RUN-05/07 | 기본 blocking-family 실행기의 물리 동시성에 authoritative bound가 없음 |
| TM21-004 | P1 | lifecycle | ARC-08, LIF-04/11/12, CON-05/06 | custom `PermitWaker` destructor panic이 claimed permit을 유실시킬 수 있음 |
| TM21-005 | P1 | cancellation | TIME-01/02/06/11, ADM-12 | cancellation과 admission/claim 사이의 선형화 경쟁으로 취소된 작업이 시작될 수 있음 |
| TM21-006 | P1 | deadline | TIME-04/05/06, ADM-12 | zero acquire timeout이 이미 만료된 absolute deadline까지 무시함 |
| TM21-007 | P1 | validation | CFG-01, ADM-04, RUN-09, SEC-02 | invalid requested stack 크기를 admission 이후에 검증함 |
| TM21-008 | P1 | capability | SCP-04/06, CFG-08, RES-10, SEC-09 | `admit_resolved`의 미등록 capability가 무제한 pool로 fail-open함 |
| TM21-009 | P1 | fairness | ADM-07/08, FAIR-09/11/12 | capability-blocked head가 runnable follower를 가리면서 later newcomer는 추월 가능 |
| TM21-010 | P1 | public API | SCP-03, API-04/05/10 | `PlanSource`가 product-specific taxonomy를 public wire contract에 고정함 |
| TM21-011 | P1 | performance proof | PERF-11/12, CI-04/06/16, SEC-08 | IAI가 baseline data가 아닌 fingerprint stamp만으로 QUALIFIED를 발행함 |
| TM21-012 | P1 | mutation proof | TST-05/09/10, CI-06/16 | mutation classifier가 process exit와 unrelated failure를 판정에 반영하지 않음 |
| TM21-013 | P1 | release proof | API-07/10, OPS-04 | README가 약속한 semver 검증이 qualification gate에 없음 |
| TM21-014 | P1 | provenance | PRE-04/05/09, SEC-06/07/08, CI-10/11 | receipt가 qualification에 사용한 도구의 exact identity를 보존하지 않음 |
| TM21-015 | P1 | executor contract | RUN-02/05/11, TIME-03/06/07, ARC-05 | `CpuExecutor::spawn`의 동기 blocking 구현이 Tokio worker를 멈출 수 있음 |
| TM21-016 | P1 | memory accounting | CFG-03, RES-05/08, SEC-09 | explicit epoch `u64::MAX`가 이후 memory reconciliation을 영구 고착시킴 |
| TM21-017 | P1 | mutation isolation | PRE-07/08, TST-07/09/10, CI-06/16 | mutation runner가 shared source와 target을 직접 변경하며 campaign 격리가 없음 |
| TM21-018 | P1 | fuzz proof | TST-03/08/15/16, CI-02/12/16 | target별 실행·semantic checkpoint를 검증하지 않아 no-op harness도 CLEAN 가능 |
| TM21-019 | P2 | diagnostics | TIME-08, OBS-04/09 | timeout verdict가 현재 blocker가 아니라 intake blocker를 보고함 |
| TM21-020 | P2 | panic surface | ARC-05, RUN-05/06, SEC-02 | 문서화된 Rayon 생성자가 thread-pool build 실패를 panic으로 처리함 |
| TM21-021 | P2 | model-check proof | CON-01/02/07/08, CI-06/16 | Loom/Shuttle 실행 설정과 replay identity가 receipt에 없음 |
| TM21-022 | P1 | CI trust | PRE-04/05, SEC-06/07/08, CI-10/11 | mutable action ref와 PR write-token job이 qualification trust root를 약화함 |
| TM21-023 | P2 | composite validation | CFG-01/10, CMP-01/03, SEC-03 | 중복 stage 이름을 가진 모호한 governance plan이 admission을 통과함 |

## P0

### TM21-001 — descendant nested-wait cycle 미탐지

**근거**

- `TaskSpec::awaited_child_of` 계약은 parent가 보유한 모든 capacity 때문에 child가
  실행 불가능하면 `NestedWaitCycle`로 즉시 거부한다고 명시한다:
  `crates/taskmesh-contract/src/task.rs:206-216`.
- `declared_wait_cycle`은 동일 root라도 `TaskScope::Root` permit만 집계하고
  `TaskScope::Child` permit은 제외한다:
  `crates/taskmesh-engine/src/features/composite/mod.rs:91-145`.
- recursion guard identity도 `(root_operation_id, parent_stage)`뿐이다:
  `crates/taskmesh-engine/src/features/composite/mod.rs:65-89`.

**실패 경로**

1. capability limit이 1이다.
2. root `R` 아래 child `A`가 유일한 slot을 보유한다.
3. `A`가 서로 다른 stage의 awaited grandchild `B`를 제출한다.
4. `A`는 `Child`이므로 cycle 집계에서 제외되고 stage가 다르므로 recursion guard도
   통과한다.
5. `B`는 slot을 기다리고 `A`는 `B`를 기다리므로 영구 교착한다.

**필수 수정**

- permit과 queued request에 immediate-parent 또는 동등한 ancestry identity를 보존한다.
- cycle 판정은 동일 root 전체가 아니라 실제 wait parent lineage가 보유한 비선점
  capacity를 기준으로 한다.
- 모든 `Child`를 무조건 parent capacity로 세는 방식은 sibling false positive를
  만들므로 허용하지 않는다.

**완료 조건**

- child → grandchild의 class/capability/CPU/memory 단독 보유 cycle이 모두 typed reject된다.
- stranger 또는 독립 sibling이 capacity 일부를 보유한 경우에는 정상 queue된다.
- 서로 다른 stage의 정상 fan-out은 recursive admission으로 오판하지 않는다.
- deterministic unit test와 동일 결함을 죽이는 mutation entry가 추가된다.

### TM21-002 — 복수 blocker가 nested-wait cycle을 은폐

**근거**

- `capacity_for`는 `Inflight → Capability → CPU → Memory` 순서에서 첫 blocker 하나만
  반환한다: `crates/taskmesh-engine/src/engine/state.rs:483-525`.
- admission은 반환된 단일 blocker만 `declared_wait_cycle`에 전달한다:
  `crates/taskmesh-engine/src/features/admission/mod.rs:184-192,260-309`.
- promotion은 head가 현재 runnable인지 확인할 뿐 nested cycle을 다시 판정하거나
  terminalize하지 않는다:
  `crates/taskmesh-engine/src/features/fairness/scheduler.rs:202-238`,
  `crates/taskmesh-engine/src/engine/governor.rs:432-480`.

**실패 경로**

1. parent `P`가 capability의 유일한 slot을 보유하고 awaited child `C`를 기다린다.
2. unrelated task가 `C` class의 inflight limit을 채운다.
3. `C` admission은 첫 blocker인 `Inflight`만 보고 queue한다.
4. unrelated task가 끝나면 `Capability`만 남지만 그 slot은 `P`가 보유한다.
5. promotion은 cycle을 재평가하지 않아 `P`와 `C`가 영구 대기한다.

**필수 수정**

- capacity 판단을 단일 enum이 아닌 blocker set/bitset으로 표현한다.
- blocker 중 하나라도 parent-only irreversible dependency이면 queue 전에 거부한다.
- queued awaited child는 promotion 때 blocker와 wait cycle을 재평가하고, cycle이면
  ticket을 terminalize한 뒤 waiter를 깨운다.

**완료 조건**

- `Inflight+Capability`, `Inflight+CPU`, `Inflight+Memory` 순차 해제 재현이 모두 bounded
  시간 안에 `NestedWaitCycle`로 끝난다.
- reversible stranger blocker가 남은 경우에는 false reject하지 않는다.
- timeout이 있어야만 종료되는 테스트는 완료 증거로 인정하지 않는다.

### TM21-003 — 기본 blocking-family topology가 물리 실행기를 bound하지 않음

**근거**

- `TopologyConfig::default()`의 blocking, large-stack, maintenance, local-runtime slot은
  전부 0이며 0은 no limit이다:
  `crates/taskmesh-contract/src/topology.rs:175-184,263-273`.
- 기본 CPU adapter와 `run_blocking`은 모두 Tokio의 shared blocking pool을 사용한다:
  `crates/taskmesh/src/executor/tokio_exec.rs:8-26`,
  `crates/taskmesh/src/runtime.rs:419-442`.
- adapter는 실제 worker 수를 알 수 없어 `declared_workers=None`,
  `exclusive_pool=false`를 보고한다.
- engine은 blocking/large-stack/maintenance를 별도 capability gate로 계산하지만 물리
  executor의 총 concurrency와 그 gate 합계를 연결하지 않는다:
  `crates/taskmesh/src/builder.rs:175-204`.

**영향**

- 기본 host 구성의 실제 thread concurrency는 Taskmesh topology가 아니라 외부 Tokio
  runtime 설정에 의해 결정된다.
- 여러 논리 gate의 합이 shared physical executor capacity를 초과할 수 있다.
- semantic class limit은 physical executor authority의 대체물이 아니며, class 추가에
  따라 총량이 변한다.

**필수 수정**

- CPU fallback, blocking, maintenance 등 shared blocking executor 사용 경로에 단일
  authoritative physical concurrency bound를 둔다.
- host가 외부 executor를 사용할 경우 실제 worker limit과 exclusivity를 명시적으로
  선언하고 build 시 topology와 대조한다.
- engine-specific worker pool을 추가하는 방식은 사용하지 않는다.

**완료 조건**

- default topology를 유지한 usable runtime도 물리 blocking concurrency 상한이 유한하고
  snapshot과 config에서 확인 가능하다.
- 모든 blocking-family 경로를 동시에 포화시킨 test에서 실제 running worker 수가 그
  상한을 넘지 않는다.
- executor 선언과 topology가 불일치하면 build가 typed error로 fail-closed한다.

## P1 — runtime와 contract

### TM21-004 — `PermitWaker` destructor panic 이후 permit 유실

**근거**

- `Governor::claim`은 ticket을 제거하고 `pending_ticket=None`으로 만든 뒤 반환 전에
  `drop(waker)`를 보호 없이 실행한다:
  `crates/taskmesh-engine/src/engine/governor.rs:238-260`.
- `apply_effects`도 `wake()`만 catch한 뒤 loop variable destructor는 catch 범위 밖에서
  실행될 수 있다:
  `crates/taskmesh-engine/src/engine/governor.rs:509-535`.

마지막 `Arc<dyn PermitWaker>`의 destructor가 panic하면 caller는 `Ready(permit_id)`를
받지 못하지만 ticket은 이미 사라지고 permit은 `DispatchReserved`로 남을 수 있다.

**필수 수정 / 완료 조건**

- `wake` 호출과 마지막 reference drop을 각각 panic boundary 안에서 수행한다.
- claim 반환 전 panic이면 ticket/permit 전이를 rollback하거나 terminal 상태로 만들고
  waiter가 그 결과를 관측하게 한다.
- 한 waker의 wake/drop panic 뒤에도 같은 transition의 나머지 effect가 전부 실행된다.
- custom panicking waker test에서 live permit, ticket, queue가 남지 않는다.

### TM21-005 — cancellation과 admission/claim 선형화 경쟁

**근거**

- cancel은 synchronous governor admission 이전에만 검사한다:
  `crates/taskmesh/src/runtime.rs:139-150`.
- immediate `Admitted`와 queued `ClaimOutcome::Ready` 뒤에는 deadline만 재검사한다:
  `crates/taskmesh/src/runtime.rs:164-179,220-236`.
- public `Clock`은 engine lock 전에 호출되는 host code라 의도적으로 block할 수 있다:
  `crates/taskmesh-contract/src/ports.rs:9-29`.

cancel이 initial check 이후 admission 또는 claim 이전에 발생하면 취소된 요청이 permit을
받고 작업을 시작할 수 있다.

**필수 수정 / 완료 조건**

- `Admitted`와 `Ready` 직후 cancel을 다시 검사하고 permit은 `return_unstarted`한다.
- cancel, deadline, admission이 같은 경계에서 발생할 때의 우선순위를 public contract에
  정의한다.
- barrier clock/waker를 사용한 두 race가 작업 closure를 실행하지 않고 정확한 verdict와
  zero residual accounting으로 끝난다.

### TM21-006 — zero acquire timeout이 absolute deadline까지 무시

**근거**

- relative timeout과 absolute deadline은 하나의 `acquire_deadline`으로 합쳐진다:
  `crates/taskmesh/src/runtime.rs:311-329`.
- `acquisition_budget_expired`는 `acquire_timeout == ZERO`이면 deadline 종류와 무관하게
  항상 false를 반환한다: `crates/taskmesh/src/runtime.rs:821-832`.

`CompleteBy`가 admission lock 대기 중 만료돼도 zero acquire timeout이 설정되어 있으면
permit을 수락하고 작업을 시작할 수 있다.

**필수 수정 / 완료 조건**

- relative zero의 try-once 의미와 absolute completion deadline을 별도 상태로 유지한다.
- zero timeout은 uncontended 즉시 admission을 허용하되, 이미 만료된 absolute deadline은
  항상 `DeadlineExceeded`로 거부한다.
- barrier-clock test에서 closure/thread가 생성되지 않고 accounting이 원복된다.

### TM21-007 — invalid requested stack 검증이 admission 이후임

**근거**

- blocking 경로는 plan과 permit을 먼저 얻고 stack 크기는 worker dispatch 직전에
  검증한다: `crates/taskmesh/src/runtime.rs:393-414,453-475`.
- async requested-stack 경로도 presence만 먼저 확인하고 값 범위는 lease 이후에 검사한다:
  `crates/taskmesh/src/runtime.rs:527-575`.
- 16 GiB 상한 검사는 `requested_stack_size_bytes_v1`에만 있다:
  `crates/taskmesh/src/runtime.rs:1006-1037`.

따라서 명백히 invalid한 입력이 queue slot과 fairness 순서를 점유하고 permit/accounting을
변경한 뒤에야 거부된다.

**필수 수정 / 완료 조건**

- stack presence, upper bound, `usize` 변환을 plan/admission 전에 검증한다.
- sync와 async 경로가 같은 validator와 error contract를 사용한다.
- invalid request가 ticket, queue, permit, admitted counter, capability usage를 전혀
  변경하지 않음을 snapshot oracle로 검증한다.

### TM21-008 — 미등록 resolved capability가 무제한으로 fail-open

**근거**

- `admit_resolved`는 미등록 pool 이름도 즉석에서 intern한다:
  `crates/taskmesh-engine/src/engine/governor.rs:144-152,174-187`.
- `capacity_for`는 limit이 없으면 0으로 보고 0을 ungated로 해석한다:
  `crates/taskmesh-engine/src/engine/state.rs:498-504`.

Built-in host는 canonical pool만 전달하므로 기본 경로 P0는 아니다. 그러나 public
advanced API를 쓰는 custom embedder가 오타 또는 미등록 pool을 넘기면 물리 실행을
무제한 admission할 수 있다.

**필수 수정 / 완료 조건**

- `Some(pool)`은 registry와 limit authority에 존재해야 하며, unknown은 typed reject한다.
- ungated 실행은 `None`처럼 명시적인 별도 표현만 허용한다.
- typo, registered-without-limit, authority-only pool 케이스를 구분하는 negative test를
  추가한다.

### TM21-009 — cross-pool head가 runnable follower를 영구 가림

**근거**

- class queue는 하나의 FIFO이고 promotion은 front만 후보로 본다:
  `crates/taskmesh-engine/src/features/fairness/scheduler.rs:202-238`.
- 반면 새 요청은 front가 다른 capability에 막혔으면 직접 admit할 수 있다:
  `crates/taskmesh-engine/src/engine/state.rs:528-555`,
  `crates/taskmesh-engine/src/features/admission/mod.rs:193-215`.

**실패 경로**

1. 같은 class의 head `A1`이 pool A에 막힌다.
2. 이미 queue에 들어간 follower `B1`은 pool B가 비어도 front가 아니라 promote되지 않는다.
3. 이후 도착한 pool B 요청 `B2`, `B3`은 head가 다른 pool에 막혔다는 예외로 직접 admit된다.
4. pool A가 장시간 막히면 `B1`은 무기한 대기하면서 later B 요청은 계속 실행된다.

**필수 수정 / 완료 조건**

- capability-blocked head 뒤의 runnable request를 결정적으로 선택할 수 있는 queue/index
  구조를 둔다. capability별 별도 worker pool을 만드는 문제로 풀지 않는다.
- 동일 capability 안에서는 arrival order를 보존한다.
- earlier B follower가 later B newcomer보다 늦게 실행되지 않는 liveness test를 추가한다.
- selection cost와 scan bound를 명시한다.

### TM21-010 — public `PlanSource`가 product-specific taxonomy를 고정

**근거**

- 타입은 “product-neutral provenance”라고 설명하지만 variants가 `SearchAdapter`,
  `Warmup`, `Indexing`, `PublicSdk`, `FluentSdk`로 특정 제품/호출 형태를 public serialized
  enum에 박는다: `crates/taskmesh-contract/src/task.rs:52-61`.
- `TaskSpec` wire shape에 직접 포함된다: `crates/taskmesh-contract/src/task.rs:115-125`.

새 소비자 유형마다 core contract release가 필요하고 downstream 제품 taxonomy가
Taskmesh wire compatibility에 결합된다.

**필수 수정 / 완료 조건**

- core에는 범용 origin category 또는 validated opaque provenance key만 둔다.
- 제품별 source mapping은 adapter/integration layer가 소유한다.
- 기존 serialized values의 migration과 backward compatibility 정책을 CHANGELOG 및 semver
  proof에 포함한다.

## P1 — qualification과 release proof

### TM21-011 — IAI stamp-only false qualification

**근거**

- `target/iai/taskmesh-fingerprint`가 존재하고 값만 맞으면 실행 전 status를
  `QUALIFIED`로 설정한다: `tools/bench-iai.sh:111-121`.
- benchmark exit 0 이후 fingerprint만 다시 기록하며 baseline artifact 존재·digest 또는
  실제 comparison 결과를 검증하지 않는다: `tools/bench-iai.sh:124-144`.
- gate test의 fake `cargo bench`는 artifact를 하나도 만들지 않지만 두 번째 실행이
  `QUALIFIED`라고 기대한다: `tools/bench/tests/test_iai_gate.py:198-212,280-291`.

stamp만 남고 baseline data가 없거나 손상된 cache도 regression 비교 완료로 보고될 수 있다.

**필수 수정 / 완료 조건**

- baseline manifest에 source/tool fingerprint, 필수 artifact 목록, 크기와 digest를 기록한다.
- run 전에 manifest와 artifact를 검증하고 실제 comparison completion을 파싱한다.
- stamp-only, missing artifact, corrupt artifact, partial cache는 `NOT_RUN` 또는 FAIL이다.
- 실제 baseline 생성 run은 QUALIFIED가 아니며, 검증된 비교 run만 QUALIFIED이다.

### TM21-012 — mutation classifier가 process 결과를 무시

**근거**

- `evaluate`는 compile-error 문자열과 test output을 파싱하지만 `proc.returncode`를 판정에
  사용하지 않는다: `tools/verification/run_mutations.py:237-288`.
- control은 failure 이름이 없으면 nonzero exit도 `CONTROL_GREEN`으로 분류할 수 있다.
- expected test 이름과 메시지만 있으면 exit 0 또는 unrelated failure가 함께 있어도
  `KILLED`가 될 수 있다.
- receipt result에는 process exit code가 저장되지 않는다:
  `tools/verification/run_mutations.py:453-465`.
- unmutated baseline을 먼저 실행하지 않는다:
  `tools/verification/run_mutations.py:339-369`.

**필수 수정 / 완료 조건**

- control은 exit 0, 정상 completion marker, zero failures를 모두 요구한다.
- mutant는 nonzero exit와 선언된 exact failure set/reason을 요구하며 unrelated failure는
  invalid로 분리한다.
- 동일 command/env/profile별 unmutated baseline을 선행하고 성공한 baseline만 재사용한다.
- receipt에 exit code, command fingerprint, baseline identity를 보존한다.
- synthetic exit `101`/무실패, exit `0`/실패 문자열, expected+unrelated failure가 모두
  KILLED/CONTROL_GREEN이 되지 않는 negative test를 추가한다.

### TM21-013 — semver 검증이 qualification에 연결되지 않음

**근거**

- README는 매 release에 `cargo semver-checks` 실행을 약속한다:
  `README.md:112-122`.
- 실제 정의는 수동 release checklist뿐이다:
  `docs/release-checklist.md:124-144`.
- `Justfile`, gate inventory와 qualification workflow에는 exact-SHA semver gate/artifact가
  없다.

따라서 현재 qualification receipt는 public API compatibility를 검사하지 않고도
QUALIFIED가 될 수 있다.

**필수 수정 / 완료 조건**

- release tier에 crate별 semver gate를 등록한다.
- baseline ref/SHA, current SHA, tool version, feature set, raw 결과 digest를 receipt에 넣는다.
- facade re-export, serde/wire, return-type 및 behavioural blind spot의 수동 reconciliation
  결과도 machine-checkable manifest로 보존한다.
- required release gate가 NOT_RUN이면 release receipt는 NOT_QUALIFIED다.

### TM21-014 — qualification 도구 identity 불완전

**근거**

- CI는 floating `stable`/`nightly`와 별도 설치한 semgrep, cargo-deny, cargo-fuzz,
  cargo-llvm-cov, IAI runner를 사용한다: `.github/workflows/ci.yml:28-38,92-120,136-184`.
- receipt environment는 현재 `rustc`, lock digest, OS, host, Python만 기록한다:
  `tools/qualification/receipt.py:176-186`.

동일 소스의 결과가 tool update 때문에 바뀌어도 source regression과 구분할 수 없고,
nightly/MSRV/security database가 무엇이었는지 재구성할 수 없다.

**필수 수정 / 완료 조건**

- stable, dated nightly, MSRV의 exact verbose identity를 기록한다.
- 모든 qualification tool의 version과 config digest를 기록한다.
- cargo-deny/advisory 결과에는 advisory DB revision을 포함한다.
- required identity를 얻지 못하면 해당 gate는 PASS가 아니라 NOT_RUN/FAIL이다.
- validator가 receipt의 tool identity completeness와 gate별 사용 도구의 일치를 검사한다.

### TM21-015 — custom `CpuExecutor::spawn`이 host runtime을 동기 block 가능

**근거**

- public `CpuExecutor`는 `spawn` 호출 자체가 nonblocking이어야 한다고 강제하지 않으며
  `nonblocking_submit=false`를 합법적인 topology로 표현한다:
  `crates/taskmesh-contract/src/ports.rs:82-94`.
- ADR은 inline executor를 유효한 구현으로 들고 `nonblocking_submit`이 동작을 바꾸지 않는
  metadata라고 명시한다: `docs/adr/0003-sep-16-hardening-contracts.md:120-138`.
- Tokio host는 async task 안에서 `cpu.spawn(work)`를 동기 호출한 다음에야 detached 결과를
  기다린다: `crates/taskmesh/src/runtime.rs:719-767`.
- 기존 inline-executor deadline test는 5 ms budget과 60 ms work를 사용하지만 응답이
  budget 안에 반환되는지는 검증하지 않는다:
  `crates/taskmesh/tests/hardening_deadline_custody.rs:181-227`.

custom executor가 closure를 inline 실행하면 Tokio worker가 work 전체 시간 동안 막힌다.
그동안 caller wait의 cancel/deadline도 관측할 수 없고, 같은 worker에 예약된 다른 task도
진행하지 못한다.

**필수 수정 / 완료 조건**

- Builder가 `nonblocking_submit=false` executor를 typed error로 거부하거나, public port
  자체가 nonblocking/fallible submission을 강제하도록 변경한다.
- 보완을 위해 unbounded trampoline queue 또는 executor별 hidden pool을 추가하지 않는다.
- blocking `spawn` adapter를 사용한 barrier test에서 runtime worker가 work completion까지
  붙잡히지 않고 cancel/deadline이 bounded 시간 안에 반환된다.
- executor 선언과 실제 submit 동작의 불일치를 탐지하는 contract test를 둔다.

### TM21-016 — terminal memory epoch가 reconciliation을 영구 고착

**근거**

- advanced API `reconcile_memory_at`은 caller가 임의의 `u64` epoch를 전달할 수 있고
  `u64::MAX`를 거부하지 않는다: `crates/taskmesh-engine/src/engine/governor.rs:564-581`.
- implicit `reconcile_memory`는 현재 epoch에 `wrapping_add(1)`을 적용한다:
  `crates/taskmesh-engine/src/engine/governor.rs:583-590`.
- memory ledger는 새 epoch가 현재 값보다 크지 않으면 stale로 무시한다:
  `crates/taskmesh-engine/src/features/memory/mod.rs:100-121`.

낮은 측정값과 `u64::MAX`가 한 번 수락되면 implicit epoch는 0으로 wrap되고 이후 모든
명시적 epoch도 stale이다. permit 종료 전까지 실제 memory 증가가 반영되지 않아 stale한
저비용 상태로 새 작업을 admission할 수 있다. 높은 값으로 고착되면 반대로 영구
starvation이 가능하다.

**필수 수정 / 완료 조건**

- epoch 증가에 wrapping arithmetic을 사용하지 않는다.
- terminal epoch는 ledger를 변경하기 전에 typed reject하거나, engine이 고갈을 명시적으로
  처리하는 monotonic token을 발급한다.
- `MAX-1`, `MAX`, implicit-next, subsequent explicit update를 포함한 test에서 silent wrap,
  stale undercharge, permanent overcharge가 모두 불가능해야 한다.
- reject 후 permit/class/capability accounting이 변경되지 않음을 snapshot으로 검증한다.

### TM21-017 — mutation campaign이 source와 build state를 격리하지 않음

**근거**

- runner는 repository source file을 제자리에서 mutant로 덮고 각 실행 후 `finally`에서
  복원한다: `tools/verification/run_mutations.py:339-369`.
- clean-tree 검사는 optional `--require-clean`일 뿐이며 기본 실행은 dirty tree를 허용한다:
  `tools/verification/run_mutations.py:386-402`.
- campaign은 순차 loop만 수행하고 process lock, isolated worktree/copy, campaign 전용
  `CARGO_TARGET_DIR`을 사용하지 않는다:
  `tools/verification/run_mutations.py:522-531`.

동시에 실행된 cargo/coverage/다른 mutation campaign은 일시적인 mutant source나 공유 build
artifact를 관측할 수 있다. process 강제 종료 시 복원도 보장되지 않는다. 이는 false
KILLED/MISSED뿐 아니라 사용자의 실제 working tree 손상까지 만들 수 있다.

**필수 수정 / 완료 조건**

- campaign마다 immutable source identity에서 만든 isolated copy/worktree와 전용 target
  directory를 사용한다.
- 동일 source를 직접 변경하는 실행은 lock으로 거부하고, signal/abrupt-exit 이후에도 원본
  digest가 보존되도록 한다.
- receipt에 source-before/source-after digest, isolation root identity, target identity를
  기록한다.
- 두 campaign과 normal cargo command를 병렬 실행한 stress test에서 결과와 artifact가
  교차 오염되지 않고, 강제 종료 test에서도 원본 파일이 변하지 않는다.

### TM21-018 — fuzz gate가 무의미 실행도 CLEAN 처리

**근거**

- gate는 libFuzzer summary의 존재만 확인한다. target별 `runs > 0`, 유효 corpus 수,
  semantic branch 도달은 검사하지 않고 마지막에는 aggregate runs만 출력한다:
  `tools/fuzz/run.sh:55-71`.
- `FUZZ_SECONDS=0`도 허용한다. libFuzzer의 0은 zero-duration proof가 아니라 시간 제한을
  제거하므로 repository가 주장한 bounded gate가 무기한 실행될 수 있다:
  `tools/fuzz/run.sh:41-44,59-61`.
- corpus directory는 gitignore 대상이고 tracked seed가 0개다: `.gitignore:5-10`.
- CI도 empty corpus에서 시작한다고 명시한다: `.github/workflows/ci.yml:121-124`.
- `wire_formats` target은 decode가 모두 실패하면 핵심 assertion을 하나도 수행하지 않고
  정상 반환한다: `fuzz/fuzz_targets/wire_formats.rs:19-52`.

따라서 target body가 사실상 no-op이거나 모든 입력이 decode 전에 버려져도 process가
정상 종료하고 summary를 쓰면 CLEAN이다. receipt만으로는 핵심 production transition과 wire
oracle이 한 번이라도 실행됐는지 구분할 수 없다.

**필수 수정 / 완료 조건**

- duration 0을 거부하고 각 required target별 `runs > 0`을 검증한다.
- 각 wire type과 core state transition을 실제로 통과하는 최소 seed corpus를 version-control
  하고 digest를 receipt에 묶는다.
- target별 runs, valid-input count, semantic checkpoint count, corpus identity를 저장하며
  하나라도 0이면 NOT_RUN/FAIL이다.
- positive fixture를 제거하거나 decode/state-transition checkpoint를 비활성화한 intentional
  defect가 CLEAN을 얻지 못하는 anti-vacuity test를 추가한다.

### TM21-022 — CI action과 write token이 qualification trust root에서 고정되지 않음

**근거**

- qualification을 포함한 CI가 `actions/checkout@v4`, `dtolnay/rust-toolchain@stable`,
  `Swatinem/rust-cache@v2`, `taiki-e/install-action@v2` 등 mutable tag/channel ref를 직접
  실행한다: `.github/workflows/ci.yml:27-34,159-201`.
- benchmark workflow도 mutable third-party action을 사용한다:
  `.github/workflows/bench.yml:28-34,160-179`.
- `trend` job은 `pull_request`에서도 실행되며 job 전체에 `contents: write`를 부여하고
  third-party benchmark action에 `GITHUB_TOKEN`을 전달한다:
  `.github/workflows/bench.yml:9-12,122-170`.

receipt의 source SHA가 고정돼도 실행기 action과 설치 도구가 mutable하면 같은 SHA의 검증
코드와 결과가 달라질 수 있다. 특히 PR 비교와 main의 gh-pages publish가 한 write-capable
job에 섞여 있어 least-privilege 경계가 없다.

**필수 수정 / 완료 조건**

- 모든 third-party/official action을 검토한 full commit SHA에 pin하고 사람이 읽는 version
  주석과 자동 update 정책을 둔다.
- workflow top-level 기본 권한을 `contents: read`로 고정한다.
- PR benchmark는 read-only compare만 수행하고, gh-pages write/publish는 trusted main push의
  별도 job/environment로 분리한다. PR checkout code나 compare action에 write token을 주지 않는다.
- qualification receipt에 workflow/action manifest digest와 exact tool install versions를
  포함하고 mutable ref 또는 과도한 permission을 negative fixture가 거부한다.

## P2

### TM21-019 — timeout verdict가 현재 blocker가 아닌 intake blocker를 사용

**근거**

- pending request의 `blocked_on`은 enqueue 시 한 번 저장된다:
  `crates/taskmesh-engine/src/features/admission/mod.rs:354-375`.
- `pending_block_reason`도 이것이 현재 상태가 아닌 intake 진단값임을 명시한다:
  `crates/taskmesh-engine/src/engine/governor.rs:842-847`.
- host는 timeout 직전 이 값을 읽어 error variant를 선택한다:
  `crates/taskmesh/src/runtime.rs:226-235,834-843`.

예를 들어 최초 `Inflight`가 해제된 뒤 `Capability`에 막혀 timeout되어도
`PermitAcquireTimedOut`으로 보고되어 operator가 실제 병목을 잘못 진단한다.

**필수 수정 / 완료 조건**

- timeout 판정 시 현재 capacity blocker를 engine lock 안에서 재계산하거나 current blocker를
  transition마다 갱신한다.
- 복수 blocker라면 public verdict가 단일 원인만 표현할지 우선순위/집합을 표현할지 계약을
  정한다.
- blocker가 `Inflight → Capability`, `Capability → CPU`, `Memory → runnable`로 변하는
  deterministic test에서 마지막 실제 blocker와 verdict가 일치한다.

### TM21-020 — 문서화된 Rayon 생성자가 resource failure를 panic 처리

**근거**

- fallible `try_new`가 존재하지만 `new`와 `from_topology`는 pool build 실패를
  `.expect("rayon pool must build")`로 panic 처리한다:
  `crates/taskmesh-rayon/src/lib.rs:26-60`.
- README와 external-interface 문서는 `from_topology`를 정상 통합 경로로 안내한다:
  `README.md:392-407`, `docs/taskmesh-external-interface.md:45-47`.
- auto-wiring Builder는 `try_new`를 사용하므로 이 문제는 manual/documented integration
  surface에 한정된다: `crates/taskmesh/src/builder.rs:219-229`.

OS thread/resource 고갈이나 invalid builder state가 public integration path에서 process
panic으로 전환된다. fallible 대안이 있어 P2로 분류하지만 문서화된 기본 예제는 현재
fail-closed하지 않다.

**필수 수정 / 완료 조건**

- 문서와 예제를 `try_from_topology(...)?` 같은 fallible API로 전환한다.
- panicking constructor는 지원 surface에서 제거/deprecate하거나 이름과 계약에 panic을
  명시한다.
- forced build failure test에서 typed error가 facade 경계를 통과하고 process가 unwind 또는
  abort하지 않는다.

### TM21-021 — model-check 실행 설정과 replay identity 미보존

**근거**

- Shuttle tests는 여러 `shuttle::check_random`을 호출하지만 seed/replay identity를 외부
  artifact로 남기지 않는다: `crates/taskmesh-engine/tests/shuttle_governance.rs:115,166,211,276,347,597,714`.
- `Justfile`의 Loom/Shuttle recipes는 cargo test를 실행할 뿐 bounds, permutations, schedule
  count, seed를 receipt schema로 수집하지 않는다: `Justfile:118-144`.
- gate receipt는 PASS/exit/output digest는 보존하지만 model checker 탐색 공간이나 실패
  schedule replay token을 보존하지 않는다.

현재 PASS는 어떤 concurrency 탐색을 얼마나 수행했는지 재구성할 수 없고, random failure가
발생해도 동일 schedule replay를 release artifact만으로 보장할 수 없다.

**필수 수정 / 완료 조건**

- Loom의 max branches/preemptions/permutation count와 Shuttle의 seed, iteration/schedule count를
  gate artifact에 구조화해 기록한다.
- 실패 시 최소 replay command/token을 receipt에 포함하고 clean environment에서 동일 실패를
  재현한다.
- required parameter 또는 completion count가 누락되면 concurrency gate는 PASS가 아니라
  NOT_RUN이다.

### TM21-023 — 중복 stage 이름을 가진 plan이 admission을 통과

**근거**

- `TaskSpec::stages`는 shape validation, deterministic reduce, recursion/root attribution에
  authoritative한 governance plan이라고 공개 계약이 설명한다:
  `crates/taskmesh-contract/src/task.rs:102-125`.
- `validate_shape`는 empty와 class mismatch만 검사하고 stage identity 중복은 검사하지
  않는다: `crates/taskmesh-engine/src/features/composite/mod.rs:16-38`.
- reduce validator도 각 entry의 fan-out policy만 개별 검사한다:
  `crates/taskmesh-engine/src/features/composite/reduce.rs:17-35`.

동일 `TaskStage` 이름을 서로 다른 substrate/fan-out/reduce 선언으로 두 번 넣어도 plan은
admission된다. runtime은 첫 stage만 primary substrate로 사용하고 나머지는 governance
declaration으로 취급하므로 동일 stage identity에 복수 의미가 생기지만 이를 판정할 단일
규칙이 없다.

**필수 수정 / 완료 조건**

- 한 `TaskSpec` 안에서 stage identity를 unique하게 강제하고 duplicate를 admission 전
  `MalformedTask`로 거부한다.
- 동일 이름/동일 descriptor와 동일 이름/상충 descriptor를 모두 negative test로 고정한다.
- stage ordering이 의미를 갖는다면 sequence 규칙을 public contract에 명시하고, 의미가 없다면
  wire와 validator가 canonical deterministic order를 강제한다.
- invalid plan이 ticket, permit, root attribution, scheduler state를 변경하지 않음을 검증한다.

## 최종 보완 순서와 closure

서로 같은 authority를 건드리는 항목은 한 설계 변경으로 묶어 patch-on-patch를 피한다.

1. **admission/lifecycle:** TM21-001과 TM21-002를 ancestry identity + blocker set + promotion
   재평가로 함께 수정한다. TM21-004, TM21-005, TM21-006은 같은 transition boundary에서
   panic/cancel/deadline 선형화 규칙을 확정한다.
2. **physical execution:** TM21-003과 TM21-015를 단일 physical concurrency authority와
   nonblocking submission contract로 함께 수정한다. hidden/unbounded queue나 engine별 pool을
   추가하지 않는다.
3. **resource/fairness:** TM21-007, TM21-008, TM21-009, TM21-016, TM21-019를 pre-admission
   validation, registered capability authority, deterministic eligible selection, non-wrapping epoch,
   current blocker 진단으로 닫는다.
4. **public contract:** TM21-010과 TM21-020은 product-neutral wire migration 및 fallible
   integration API로 처리하고 semver baseline에 반영한다.
5. **proof authority:** TM21-011/TM21-014/TM21-022, TM21-012/TM21-017,
   TM21-018/TM21-021을 각각 artifact identity/CI trust, isolated mutation campaign,
   anti-vacuity/replay receipt 단위로 묶고 TM21-013의 release gate에 연결한다.
6. **plan validation:** TM21-023의 unique stage identity 규칙을 producer와 admission validator
   양쪽에 두지 말고 contract-owned validator 하나로 고정한다.

release closure는 다음 순서를 모두 만족해야 한다.

1. 각 finding의 deterministic regression/negative test와 intentional-defect oracle이 먼저
   추가된다.
2. 변경된 producer/authority owner를 수정하고 full workspace + feature matrix + model check +
   isolated mutation + non-vacuous fuzz + performance comparison을 frozen source에서 실행한다.
3. receipt에는 exact source, toolchain/tool/config, test count, model schedule, fuzz runs/corpus,
   mutation baseline/process result, performance artifacts가 결합되어야 한다.
4. required gate의 NOT_RUN, missing identity, derived-only PASS, stale artifact가 하나라도 있으면
   최종 상태는 `NOT_QUALIFIED`다.
5. clean checkout에서 receipt validator와 release semver gate를 재실행한 뒤에만 이 문서의
   상태를 GO 후보로 변경한다. 문서 체크박스나 개별 focused PASS는 closure가 아니다.

## 이번 목록에서 제외한 항목

다음은 후속 hardening 후보지만 이 감사에서 실제 결함 또는 false qualification으로
확정하지 않았으므로 구현 backlog에 넣지 않는다.

- default-feature doc example 분리 부족
- USL 통계와 platform matrix 부족
- branch/instantiation coverage 부족
- Tokio runtime 밖에서의 호출 precondition 문서화
- `RuntimeConfig`의 “validated” 문구와 deserialize 의미
- wall-clock jump에 대한 lease 정책
- payload 크기 제한은
  `docs/plans/sep-16-hardening/tickets/EXCEPTIONS.md:10`의 caller-owned heap 범위로
  명시되어 있어 이번 finding에서 제외

초기 리뷰에서 지적된 combined receipt의 derived mutation splice 후 summary 불일치는 현재
working tree의 `finalize_gate_receipt`와 sidecar 재기록으로 보완되어 있다:
`tools/qualification/receipt.py:386-449`. 따라서 별도 구현 finding에는 넣지 않는다. 단,
repository root의 기존 `receipt*.json`은 수정 전 HEAD `cc5b256704a1826e7dde36e2c4b127c6cba8aabf`에
묶인 historical artifact이고 재생성되지 않았으므로 current proof로 사용하면 안 된다.

## 현재 감사 증거의 한계

다음 focused 검사는 현재 tree에서 통과했다.

- `cargo check --locked -p taskmesh --lib`
- `cargo test --locked -p taskmesh-engine --test hardening_nested_wait --test hardening_effect_retirement`
  — 17 passed
- `cargo test --locked -p taskmesh --lib claim_acquisition_tests` — 13 passed
- `uv run python tools/gates/validate_inventory.py` — 24/24/24 PASS
- `uv run ruff check tools`
- `uv run pytest -q tools/qualification/tests/test_receipt.py tools/gates/tests/test_inventory.py
  tools/bench/tests/test_iai_gate.py tools/verification/tests/test_run_mutations.py` — 111 passed
- `git diff --check`

이 통과 결과는 위 counterexample을 다루는 테스트가 없거나 verifier가 해당 상태를
관측하지 않는다는 사실과 양립한다. full workspace, full mutation, fuzz, performance,
Loom/Shuttle 장기 탐색, hosted CI는 이번 감사에서 실행하지 않았다. 현재 local receipt는
수집하지 않았다. repository root의 historical combined receipt verdict도 `NOT_QUALIFIED`이며
현재 HEAD와 dirty working tree의 release closure 증거가 아니다.

현재 receipt producer는 curated inventory와 generated cargo-mutants sweep을 구분하고
generated sweep을 `NOT_RUN`으로 명시하도록 보완돼 있다. 이는 기존 100-entry 결과의
오해 가능성을 줄이지만, 중단됐던 generated sweep의 현재-source 재실행이나 낮은 historical
score를 대체하지 않는다. 따라서 generated full-sweep 증거 수준은 계속 `NOT_RUN`이다.
