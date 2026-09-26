# Taskmesh 엔진 필수 유즈케이스·적대 시나리오 체크리스트

- 기준: `main@76559c483bdd1d6b0b5226f7c5b5591b919afae9` (2026-09-25)의 원본 oracle inventory. 현재 매핑과 잔여 작업은 `docs/plans/bugbash-sep-25-general/tickets/{scenario-evidence.json,OPEN-FOLLOWUPS.md,EXTERNAL-ADOPTION.md}`를 따른다. 당시 구현 감사는 `docs/adr/0007-sep-25-implementation-closure.md`에 보존한다. 아래 fixture 참조는 clean-HEAD 실행 증거가 아니다.
- 범위: 현재 `taskmesh-contract` / `taskmesh-engine` / Tokio facade / Rayon adapter / benchmark harness의 기능과 그 경계에 필요한 안전 시나리오. 현행 보장과 미결정 목표 계약은 각 행에서 구분한다. 미래의 실행형 flow·분산 복구·자동 checkpoint 실행은 제외한다.
- 상태: 시나리오 설계 목록. 관련 테스트 파일은 탐색 앵커일 뿐, 각 행의 완전한 커버리지나 현재 HEAD의 PASS 영수증이 아니다. 원본 목록 작성 시 테스트·mutation·nightly는 실행하지 않았다. 2026-09-26의 별도 재검증 범위는 아래에 기록한다.
- 규모: 기본 16·엣지 28·헬게이트 35·코너 25, 총 104개. 미결정 계약과 source-backed 실패 후보는 해당 행에 표시했다.
- 용어: **기본**=대표 사용 경로, **엣지**=단일 경계·실패 경로, **헬게이트**=두 개 이상 기능/경쟁 상태가 겹친 적대 경로, **코너**=수치·순서·직렬화의 정확한 끝점.

## 판정 원칙

모든 시나리오는 `입력/상황 → 관측 가능한 결과`로 판정한다. 단순히 테스트가 종료하거나 `snapshot().conservation_violation() == None`인 것만으로 성공 처리하지 않는다.

1. **보존:** `admitted_total == inflight + terminated_total`; phase 합계는 `inflight`; class·global CPU/메모리와 capability occupancy는 독립 permit ledger 재계산과 일치한다. 정상 정책에서 hard cap을 넘지 않는다. 측정된 메모리 초과는 현실로 기록하고 추가 메모리를 요구하는 새 입장만 막는다. `Snapshot::conservation_violation()` 단독으로 held/ledger 일치를 증명하지 않는다.
2. **닫힌 실패:** 알 수 없는 class/pool, 잘못된 plan/policy/topology, 지원하지 않는 deadline 형태, 회수된 ticket/lease는 해당 경로의 typed 결과로 끝난다. work closure가 실행되거나 queue·계수가 변해서는 안 되는 경로를 별도로 확인한다. `RunFor` 산술 overflow의 경로별 현행 동작은 D05로 분리한다.
3. **진전:** 해제 가능한 capacity가 있고 유효한 대기자가 있으면 유한 단계 안에 promotion/claim 또는 명시적 terminal 결과가 나온다. permit 없는 admission 대기를 별도 executor queue로 우회하지 않는다. 이미 accepted된 미시작 작업의 executor 보유와 custody는 H19로 판정한다.
4. **소유권:** caller 응답, worker 종료, permit release, drain 완료는 별개 사건이다. 시작한 sync worker가 남아 있으면 caller가 취소되어도 그 자원은 계속 charged다.
5. **결정성:** 동일 정책·입장 순서·clock·작업 결과이면 verdict와 promotion 순서가 재현된다. 실제 스레드 실행 순서나 wall-clock p99의 비트 동일성은 주장하지 않는다.
6. **정책과 실행 분리:** `TaskSpec`의 뒤 stage·fan-out·reduce·checkpoint는 선언/검증 경계다. host의 한 `run_*` 호출은 실제 첫 dispatch 하나만 실행한다.

## A. 기본 유즈케이스 — 제품 사용자가 반드시 성립한다고 기대할 경로

| ID | 상황 | 필수 관측/판정 |
|---|---|---|
| A01 | 유효한 class/topology/budget으로 `Builder::build`, `run_io` 성공 | 제출한 future의 작업 결과를 한 번 반환하고 permit·phase·pool이 idle로 복귀. future의 poll 횟수를 1회로 제한하지 않음. |
| A02 | 각 경로에 맞는 spec을 `run_blocking`, `run_cpu`, `run_local`에 제출 | 각각 실제 substrate에서 1회 실행; `!Send` local은 허용된 local runtime에서만 실행. |
| A03 | default CPU adapter와 Rayon adapter를 각각 설치 | `Builder`가 executor의 worker **선언**과 resolved domain limit의 일치를 검사하고, Taskmesh 제출을 그 상한으로 제한. Rayon 자체 pool의 실제 worker 수는 조회할 수 있지만 default Tokio shared pool의 실제/외부 점유는 이 선언으로 증명되지 않음. |
| A04 | `max_inflight=1`, 두 요청, 유한 class queue | 하나 실행·하나 queue; release 뒤 대기자가 한 번만 promoted/claimed; queue·inflight가 설정치를 넘지 않음. |
| A05 | 서로 다른 class가 global CPU 또는 memory를 공유 | 합산 예산 안에서 admission; class별 snapshot과 독립 permit ledger의 합이 일치; 한 class 해제 후 다른 class 진전. |
| A06 | blocking/CPU/large-stack/local/maintenance capability 사용 | blocking·maintenance는 shared, CPU는 설치된 executor, requested-stack은 dedicated physical domain과 role pool을 원자적으로 예약/반납. local은 `local_runtime` role만 예약하며 IO는 별도 pool이 없다. snapshot의 이름·limit·occupancy가 이 구분과 일치. |
| A07 | queue 정책과 `RetryAfterPolicy::{None, FixedMs, Adaptive}` | 과부하 거절에만 해당 정책의 힌트; 비-backpressure 오류에는 힌트 없음; 재시도 후 capacity가 생기면 진전. |
| A08 | Estimated/Measured/Hybrid 메모리 측정 후 reconcile | 설정된 단위 변환·estimate floor·현재 reservation 기준으로 held가 계산되고 해제 시 0으로 복귀. |
| A09 | `OnStageBoundary` 정책으로 일부 memory release | permit은 계속 inflight, 반환된 memory만 새 요청이 사용; 최종 release에서 남은 held가 반환. |
| A10 | root와 declared child를 별도 제출 | immediate parent·root attribution·provenance가 admission→queue→claim 동안 보존. 허용 가능한 자식은 진전. |
| A11 | 순차 stage 선언과 fan-out/reduce policy 선언 | plan은 검증되지만 뒤 stage와 reducer는 자동 실행되지 않음; caller가 별도 제출한 실제 작업에만 새 permit. |
| A12 | 한 class 안에서 capability가 겹치는 요청의 순서, 여러 class의 동일 tier fairness discipline | 겹치는 이전 runnable 요청의 순서와 선택된 cross-class 정책이 재현된다. disjoint capability 요청의 허용된 선행은 FIFO 위반으로 오판하지 않음. |
| A13 | idle/active `drain(timeout)`과 여러 runtime handle | admission close는 모든 clone에 공유되는 one-way 상태. 기존 queue·inflight가 0일 때만 `Ok`, timeout은 per-class 미완료를 보고하며 새 **유효한** 제출은 `RuntimeUnavailable`. malformed/foreign-capability 입력의 preflight 우선순위는 D17로 분리. |
| A14 | snapshot, inventory, provenance, typed error를 facade 소비자가 조회 | 외부 소비자는 공개 facade로 상태와 `GovernorError`/`RunError`를 타입으로 구별한다. `Snapshot`·`TaskSpec`·정책·직렬화 가능한 verdict/terminal reason의 wire 의미는 별도로 보존; runtime-only error 자체의 Serde 왕복은 요구하지 않음. |
| A15 | requested-stack async `!Send` root와 그 안에서 await하는 `tokio::spawn` child | 명시된 stack의 owned current-thread runtime에서 root와 `Send` child가 실행되고 종료 시 dedicated slot이 반환됨. `spawn_local` child 지원을 이 경로에서 암시하지 않음. |
| A16 | custom substrate/capability를 명시적으로 등록 | 추가 substrate는 config·snapshot inventory와 direct 거버넌스 authority에만 나타남. 닫힌 `SubstrateHint`와 facade에 새 `run_*` 대상이 자동으로 생기지 않음. |

관련 앵커: `crates/taskmesh/tests/{runtime_io,runtime_blocking,runtime_cpu,runtime_local,runtime_cpu_executor,e2e_scenarios,hardening_consumer_surface,config_inventory}.rs`, `crates/taskmesh-engine/tests/{admission_queue,memory_modes,provenance_preservation,substrate_inventory}.rs`.

## B. 엣지 — 거절·취소·장애의 단일 경계

| ID | 상황 | 필수 관측/판정 |
|---|---|---|
| B01 | unknown/disabled class, malformed policy class, duplicate class registration | unknown/disabled는 입장 전 typed reject; 구성 오류·중복은 build에서 fail closed; 암묵적 기본 class/마지막 정책 덮어쓰기 없음. |
| B02 | zero stage, 중복·충돌 stage, class 불일치, parent identity 누락, reduce 누락/불필요 | 필수 field가 없는 JSON은 Serde decode 단계에서, 구조적으로 깨진 raw `TaskSpec`은 plan validator에서 각각 거절; 어떤 permit·ticket도 만들지 않음. parent stage의 **존재성** 검사는 B22로 분리. |
| B03 | hint와 `run_*` 불일치, invalid requested stack 크기/shape | dispatch·worker 생성 전에 typed reject; snapshot·job side effect 0. |
| B04 | per-request 비용이 예산을 넘거나 불가능한 degrade chain; 별도로 불가능한 topology/CPU executor | 정책은 `Builder`와 direct `Governor::new`에서 reject; `TopologyConfig`와 host executor 선언은 입력을 소유한 `Builder::build`에서 reject. fairness tier의 정확한 경계는 B26. 두 경로에 존재하지 않는 동일 검사를 요구하지 않음. |
| B05 | class quota, capability pool, CPU budget, memory budget 각각 단독 포화 | 정의된 blocker 우선순위에 맞는 verdict. class/CPU/capability가 primary이면 일반 overflow가 결정하며, **Memory가 primary일 때만** 별도 `MemoryOvercommitPolicy::Queue`가 일반 `Reject`여도 bounded queue에 넣을 수 있다. `QueuedBehind`도 기존 queue에 합류. 복합 포화는 H32로 분리. |
| B06 | queue depth 정확히 가득 참 | 한 건 더 오면 blocker 원인에 맞는 typed shed; depth+1을 저장하거나 permit 없는 admission 대기자를 executor backlog로 우회 전송하지 않는다. 이미 accepted된 미시작 closure는 permit/capacity를 계속 점유한다. |
| B07 | 제출 전 cancel; `PreSubmitOnly` class의 실행 중 cancel | 전자는 정책과 무관하게 거절; 후자는 실행 중 강제 취소를 약속하지 않음. |
| B08 | queued cancel/acquire timeout/caller future drop | ticket abandon 또는 terminal 결과가 한 번만 적용; 대기·recursion slot·waker가 남지 않음. |
| B09 | 지원하지 않는 class의 run deadline 또는 sync/CPU 경로의 `CompleteBy` | `DeadlineUnsupported` 등 typed preflight 오류; job 미시작·permit 미보유. |
| B10 | worker panic, spawn/setup 실패, adapter가 받은 closure를 실행하지 않고 **drop** | `WorkerPanicked` / `WorkerUnavailable` / `JobAbandoned`를 구별; task가 반환한 `Err`는 `RunError::Task`로 유지. closure를 무기한 보유하는 경우는 `JobAbandoned`가 아니며 H19로 분리. |
| B11 | 같은 Governor에서 released/unknown permit, 잘못된 lease token, double release | 존재하지 않거나 이미 release된 local ID는 `UnknownPermit`; leased permit의 raw release와 잘못된 nonce는 `HeldByLease` 등 정확한 결과. 다른 live permit은 보존. 다른 Governor의 **raw** 숫자 ID는 소유권 증거가 아니며 B28로 분리. |
| B12 | promoted-but-unclaimed permit이 sweep/release로 종료되거나 queued ticket이 irreversible cycle로 terminalized; 별도로 queued ticket abandon·unknown local ticket claim | 보존된 종료는 `Terminal(reason)`, local에 없는 숫자 ticket은 `Invalid`; 영구 `Pending`이나 이미 죽은 permit id 반환 없음. sweep이 permit 없는 queued ticket을 직접 회수한다고 기대하지 않음(B23). 다른 Governor의 raw ticket 충돌은 B28. |
| B13 | `LeakDetecting` stale 미시작 permit과 stale 실행 중 lease | 전자만 reclaim/promote; 후자는 `retained_active`로 보고하고 worker 종료 전까지 charged. |
| B14 | memory overcommit `Reject`/`Queue`/`DegradeToLight` | 각각 정확한 거절·bounded 대기·단일 fallback 재계상; disabled/unknown/연쇄 fallback은 구성 단계 거절. |
| B15 | `OnTaskCompletion`의 stage memory release, stale/duplicate measurement | 금지/오래된 보고는 typed no-change; stage release가 이미 돌려준 메모리를 reconcile이 되살리지 않음. |
| B16 | 잘못된 substrate inventory/pool authority, resolved domain worker 수보다 작거나 **큰** CPU executor 선언, blocking/unknown submit protocol | 구성 단계에서 reject; executor 선언과 resolved worker 수는 정확히 같아야 한다. runtime snapshot에 보이는 capacity authority와 admission authority가 하나. |
| B17 | `drain(Duration::ZERO)` 및 유한 timeout에 실제 작업 잔존 | ZERO는 한 번 관측, timeout은 per-class `NotDrained`와 elapsed; admission closed 상태가 유지됨. |
| B18 | custom `PlanSource`와 legacy alias를 source API/Serde로 제출 | source key의 길이·문자 검증은 일관되며, 유효한 legacy spelling은 보존된다. `ClassificationRationale`은 enum이므로 문자열 길이 검증 대상으로 취급하지 않음; 일반 identifier 끝점은 D03. |
| B19 | 구조가 틀린 raw `TaskSpec` JSON과 같은 값의 `ValidatedTaskPlan` JSON | raw Serde decode는 구조 검증을 약속하지 않음. `ValidatedTaskPlan` decode나 명시적 `validate()`가 reject한다. strict ingress는 `ValidatedTaskPlan`을 만들기 전까지 runtime handle을 받지 않는 순수 decode/validation 경계이므로 caller는 실패 값을 admission/worker 경로에 제출할 수 없다. |
| B20 | local role pool만 포화, 다른 physical domain은 여유 | local 제출은 `local_runtime` blocker를 정확히 보고하고, 무관한 shared/dedicated occupancy를 늘리지 않음. |
| B21 | 동일 `(root_operation_id, operation)`의 요청 두 건을 class·stage를 달리하거나 첫 건을 queue에 둔 채 제출 | 두 번째는 용량이 남아도 `RecursiveAdmission`이고 계수·queue·waker 부작용이 없다. 다른 root의 같은 operation은 허용하며 첫 identity가 release·abandon·terminalize되면 재사용할 수 있다. |
| B22 | live parent를 가리키지만 그 parent plan에 없는 유효한 `parent_stage`로 child 제출 | 현재 validator는 `parent_stage`의 문자열 형식만 검사하고 stage membership은 확인하지 않는다. 현재의 attribution/recursion 동작을 고정해 관측하고, cross-plan membership을 API 계약으로 요구할지 별도 결정한다. 현행 `MalformedTask`를 미리 기대하지 않음. |
| B23 | `LeakDetecting` class의 요청이 permit 없이 queue에 오래 머무는 동안 clock 전진과 `reap_leaks()` 반복 | sweep은 live permit만 검사하므로 queued ticket을 시간만으로 없애지 않는다. host acquire timeout 또는 caller abandon이 대기 취소를 소유하며, capacity release가 오면 여전히 promotion 대상이다. |
| B24 | 문법상 유효하지만 실제 분류와 맞지 않는 `source`·`reason`·`class` metadata 제출 | 엔진은 분류의 사실 여부를 추론하지 않고 provenance를 그대로 보존한다. 외부 분류 어댑터가 신뢰 경계에서 생성·검증해야 하며 엔진의 거절을 oracle로 삼지 않음. |
| B25 | `run_blocking` 또는 기본 CPU facade future를 Tokio runtime 밖에서 poll | 지원하는 호출 context를 공개 계약으로 정한다. 미지원이라면 admission/dispatch 전 typed reject가 목표이며, 현 경로에서 panic이 발생하더라도 permit이 남지 않는지 별도 확인한다. 이를 현재 보장된 typed 오류로 주장하지 않음. |
| B26 | 같은 primary tier의 FIFO·WFQ 혼합, 같은 종류의 weight/quantum 차이, primary와 best-effort의 서로 다른 fairness 종류 | 같은 tier의 **종류** 혼합만 구성 거절; 같은 종류의 파라미터 차이와 tier 간 다른 종류는 허용한다. disabled class는 dispatch하지 않아 종류 합의 대상에서 제외. |
| B27 | malformed `admit_waitable` 또는 invalid resolved capability와 final-reference `PermitWaker`의 destructor panic | preflight가 waker를 직접 drop하므로 panic은 그대로 전파되지만 permit·ticket·queue 상태는 생성되지 않는다. H14의 transition-effect 전체 배출/보상 oracle을 이 경로에 기계적으로 적용하지 않음. |
| B28 | 두 Governor가 같은 local sequence의 `PermitId`/`Ticket`을 발급하고 한 인스턴스의 opaque handle을 다른 인스턴스의 `release`/`advance_phase`/`claim`/`abandon`에 전달 | handle의 governor authority가 달라 foreign operation은 `UnknownPermit`/`Invalid` 등 typed no-change로 끝난다. 같은 local sequence만으로 다른 Governor의 상태에 작용할 수 없고 local permit/ticket은 보존된다. |

관련 앵커: `crates/taskmesh-contract/tests/{task_plan_validation,topology_validation,error_surface}.rs`, `crates/taskmesh-engine/tests/{config_validation,hardening_policy_inventory,permit_lifecycle,hardening_lease_token,hardening_child_scope,leak_sweep,memory_overcommit,substrate_inventory,pending_resolver,recursive_rejection}.rs`, `crates/taskmesh-engine/src/{shared/mod,engine/governor}.rs`, `crates/taskmesh/tests/{runtime_cancel_timeout,hardening_executor_protocol,hardening_drain,config_inventory}.rs`.

## C. 헬게이트 — 기능 간 충돌과 적대 스케줄

각 행은 **독립 oracle**을 요구한다. 구현의 snapshot을 그대로 expected 값으로 복사하지 말고, fixture가 받은 permit/ticket/closure 실행 횟수와 모델의 기대 상태를 별도로 계산한다.

| ID | 결합 시나리오 | 깨지면 안 되는 결과 / 실패 신호 |
|---|---|---|
| H01 | 여러 class × IO/blocking/CPU/large-stack × 급격한 open-loop 과부하 | class queue·capability·physical domain 상한 동시 보존, 비queueing 즉시 shed, 유한 대기자는 종결. 도착률을 제어하고 응답시간만으로 과부하를 판정하지 않음. |
| H02 | class quota는 남지만 공유 physical domain이 포화; 다른 class/role도 같은 executor 사용 | engine이 실제 공유 domain을 한 번만 계상, 포화 verdict/queue 원인이 domain을 가리킴; 앞단 gate 뒤의 숨은 무한 대기열 없음. |
| H03 | CPU fallback·blocking·maintenance가 `physical.shared_blocking`을 동시에 사용 | 세 경로 합계가 같은 유한 상한 이내. Rayon의 별도 `physical.cpu` 격리는 H18의 Rayon selector에서 별도 판정한다. shared Tokio pool의 **외부** 작업까지 Taskmesh가 제한한다고 주장하지 않음. |
| H04 | capacity release ↔ queued promotion ↔ waiter cancel/claim/timeout/sweep 네 방향 경합 | permit은 정확히 한 소유자에게만 이전. 늦은 claim은 소유권 이전이면 `Ready`, 보존된 종료면 `Terminal(reason)`, abandon/retention 밖이면 `Invalid`; 중복 시작·누락·영구 대기 없음. |
| H05 | `PROMOTION_BUDGET`보다 긴 backlog와 release 한 번, 동시에 새 요청 도착 | continuation이 추가 release 없이 진행; 기존 queue와 capability가 겹치는 queueable same-class newcomer는 앞선 runnable head 뒤에 선다. disjoint capability newcomer의 선행, 다른 pool에 막힌 head의 교차-pool 예외, queue를 사용하지 않는 `Reject` newcomer의 continuation-gap 즉시 입장은 따로 판정한다. |
| H06 | WFQ/DRR의 불균일 weight·cost와 중간 취소·cross-pool follower 선행 | reference scheduler와 정확한 선택 순서; 취소된 작업의 가상 시간 debt/DRR credit이 생존 요청을 왜곡하지 않음. starve·무한 quantum 순회 없음. |
| H07 | primary tier 포화, best-effort scavenger와 memory fallback이 함께 경쟁 | primary가 runnable이면 정책 우선권 유지; fallback도 새 class/pool 비용을 재검사; `DropBestEffort`가 조용한 소실로 바뀌지 않음. |
| H08 | 부모가 자식을 await하며 자신이 class/pool/CPU/memory 전량 점유 | 선언된 exact ancestor chain만 `NestedWaitCycle { held_by_root }`로 거절. 독립 sibling/stranger가 해제할 수 있는 슬롯이면 false cycle로 거절하지 않음. |
| H09 | root/parent operation 이름 재사용과 queued child promotion/abandon | live parent generation/정확한 immediate parent에 결속; 재사용된 이름으로 ancestry가 바뀌거나 recursion guard가 새 root에 새지 않음. |
| H10 | stage memory release ↔ measured/hybrid reconcile ↔ queue promotion ↔ leak sweep | 반환된 단위는 재등장하지 않고, epoch/sequence는 순서대로만 적용. 아직 실행 중인 lease는 sweep이 뺏지 않음. |
| H11 | pre-submit cancel, `CompleteBy`, acquire timeout이 같은 tick에 겹침; admission 직후 budget 만료 | 문서화한 cancel→absolute deadline→relative timeout 우선순위; equality는 expired. 늦게 얻은 permit을 work 시작 전에 unwind. |
| H12 | started sync worker에 `RunFor` deadline·cancel 또는 caller future drop이 발생한다. worker가 살아 있을 때 두 번째 요청을 **drain 전에** 제출·queue하고 drain을 시작한다. | deadline·cancel은 제때 typed 응답; caller drop에는 응답이 없다. worker 종료까지 lease·pool이 charged이고 두 번째 요청은 시작하지 않으며 drain `Ok`도 금지된다. worker 종료 뒤 이미 queue된 요청이 진전하고 그 요청까지 끝나야 drain `Ok`; drain 시작 후 새 제출은 `RuntimeUnavailable`. 각 종료 원인별 판정과 이 결합 순서의 판정은 구분한다. [Tokio `spawn_blocking`](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)도 시작된 작업은 abort할 수 없다고 명시한다. |
| H13 | direct `governor()` admit/release/abandon/reap ↔ host `drain(Duration::MAX)` | engine gauge가 idle이 된 뒤 settlement signal로 drain이 깨어남; close↔첫 snapshot, snapshot↔wait, 여러 drain caller의 lost wake 없음. 현재 HEAD의 `SettlementWaker` 경로를 기준으로 함. |
| H14 | `PermitWaker`의 `wake`/`drop` 또는 `SettlementWaker`가 재진입하거나 panic; promotion backlog도 큼 | 외부 callback은 engine mutex 밖에서 실행; 가능한 나머지 waiter/effect/continuation 처리 뒤 첫 panic을 전파한다. reentrant snapshot·release가 deadlock하지 않고 engine ledger가 보존된다. panic한 사용자 callback 자체의 알림 전달까지 보장한다고 가정하지 않음. |
| H15 | executor가 `spawn`에서 panic/거부/closure 유기; caller가 accepted 뒤 즉시 사라짐 | 작업은 최대 1회; started/accepted/never-started 구분에 따라 custody가 끝까지 유지되거나 정확히 반환. |
| H16 | 실제 Governor의 짧은 상태 기계 시퀀스를 두 스레드가 교차 실행 | 각 작업의 선형화 가능한 history 또는 barrier로 구획한 정지 상태를 독립 모델의 admitted/queued/permit ledger/terminal reason과 비교. 임의 동시 snapshot을 순차 모델의 다음 한 단계로 오인하지 않음. 실패 seed·schedule replay 가능; production `sync` seam을 쓴 Loom/Shuttle는 좁은 interleaving에 적용. [Loom 문서](https://docs.rs/loom/latest/loom/)는 replacement type 밖의 동시성은 관측하지 못한다고 설명한다. |
| H17 | benchmark **시뮬레이터**의 open-loop generator/admission-wait recorder에 overload, undersaturation, 여러 seed | `offered = completed + rejected + leftover_queued`, `completed = admitted_immediately + queued_then_promoted`, queue bound, 고정 seed 재현. raw latency 표본은 **시작한 요청의 admission wait**에 한정되고 실제 host 실행 latency나 rejected 요청 시간을 뜻하지 않는다. 현행 `dropped()==0`만으로 계측 누락 방지를 증명하지 않는다; 포화 구간의 p99 단조성도 무조건 요구하지 않음. |
| H18 | 공개 wire roundtrip·old alias·feature(Rayon)/MSRV 소비자가 동일 의미로 호출 | source/serde 호환 계약이 지켜지고 host·direct의 **서로 다른** multi-stage 예약 범위를 명시적으로 관측; fixture가 내부 API 의존으로 소비자 결함을 숨기지 않음. default·Rayon owned pool·Rayon shared pool은 각각 선택 범위를 기록한다. |
| H19 | CPU executor가 closure를 accepted 상태로 무기한 보유하고 시작/반납하지 않음; 동시에 `RunFor`, caller cancel/drop, 다른 입장과 drain | `RunFor`는 worker 시작 전 진행된 것으로 처리하지 않음. closure가 살아 있는 동안 lease·pool은 charged, 두 번째 요청은 bound, drain은 `NotDrained`; cancel은 caller wait만 끝낼 수 있다. closure를 실제 실행하거나 drop할 때만 최종 settlement. `JobAbandoned` 조기 반환 금지. |
| H20 | requested-stack async root가 deadline/panic으로 답을 마쳤지만 owned runtime의 blocking child가 아직 실행 중 | caller 답은 child teardown을 기다리지 않아도 된다. deadline 경로는 `cleanup_pending`, panic 경로는 `running` phase에서 teardown을 기다릴 수 있으나 두 경우 모두 lease·dedicated capacity는 child 종료까지 charged다. 그 전 drain `Ok`·capacity 재판매는 금지. |
| H21 | `run_io`/`run_local` caller future panic/drop와, 별도로 root가 raw `tokio::spawn`한 child | caller-owned root의 RAII lease는 panic/drop 시 반환되어 다음 admission이 진전한다. panic을 detached worker의 `WorkerPanicked`로 재분류하지 않는다. raw detached child는 자동 추적 대상이 아니므로 root 종료 후 drain `Ok`가 그 child의 종료를 뜻하지 않는다. |
| H22 | holder와 queued ticket이 있을 때 `close_admission`, 그 뒤 신규 제출과 holder release | 신규 **유효** 제출은 거절, 이미 queue된 ticket은 계속 promote/claim되어 완료; drain은 기존 queued/inflight가 끝나야 성공한다. |
| H23 | awaited child가 처음엔 stranger 때문에 가역적으로 queue됨; 그 뒤 자기 parent가 마지막 pool slot을 점유하고 stranger가 release | promotion 시 다시 평가하여 `IrreversibleWaitCycle`로 terminalize, waiter를 깨우고 recursion guard를 반납. intake 때의 가역 판정을 고정하지 않음. |
| H24 | 같은 root·declared parent_stage에 서로 다른 child operation 두 개를 병렬 제출 | 현재 recursion guard는 이 조합을 같은 slot으로 취급한다. 두 번째는 첫 child가 guard를 비울 때까지 `RecursiveAdmission`; caller는 branch별 고유 stage identity가 필요하다. fan-out 선언만으로 이 충돌이 풀린다고 가정하지 않음. |
| H25 | queued ticket에 class/pool/CPU/memory blocker가 동시에 존재하고, 하나씩 해제·reconcile | `pending_assessment`가 현재 상태를 재평가한다. 진단 경로는 `promotion_pending`이면 물리적 capacity가 비어도 `QueuedBehind`를 포함할 수 있고, 실제 scheduler는 runnable head를 별도로 평가한다. `pending_block_reason`은 reversible일 때 진단 blocker의 primary, runnable일 때 `Ok`, irreversible cycle일 때 접수 당시 `blocked_on` fallback이다. 단일 reason을 전체 물리적 blocker set으로 오인하지 않음. |
| H26 | 같은 class의 queue head가 {A,B}를 요구하고 A만 포화; follower는 B 또는 무관한 C 요구 | B와 겹치는 follower는 head 뒤에 남고, 독립 C follower만 정책상 허용될 때 선행 가능. A가 풀리면 head는 A+B를 **원자적으로 각각 한 번** charge하고 release가 둘 다 반환. |
| H27 | 두 TokioRuntime이 같은 `CpuExecutor` 인스턴스를 공유하고 각자 CPU domain limit까지 제출 | 각 runtime의 엔진은 자기 제출만 제한한다. 두 runtime 합산 physical worker 독점은 보장되지 않음을 관측하고, 외부 공유 pool 전체 상한이 필요하면 호출자가 별도 authority를 제공해야 함. |
| H28 | 통제된 finite open-loop burst를 실제 host와 benchmark simulator에 제출하고, 별도 host 성능 workload를 계측 | 결정적 CI fixture에서는 두 경로가 공통으로 모델링하는 offered/completed/rejected/max-queue만 비교한다. simulator의 admission-wait를 host end-to-end latency로 보고하지 않고, simulator가 모델링하지 않는 task error·cancel·deadline의 건수 동치를 요구하지 않는다. 별도 quiet-host 성능 자격에서는 intended send/offered, terminal response 유형, 미응답, 응답 뒤 남은 worker custody를 **서로 독립적으로** 계측하고 class/path별 표본 모집단·warmup·환경을 기록한다. |
| H29 | promoted ticket의 `claim()`에서 마지막 `PermitWaker` drop이 panic; 같은 pool에 다음 waiter 대기 | custody 전달 전 `Claiming` permit을 전량 보상하고 첫 ticket은 `ClaimDeliveryFailed` terminal이 된다. 다음 waiter를 즉시 promote/notify한 뒤 첫 panic을 전파한다. 중복 permit·계수 누락·영구 대기 없음. |
| H30 | custom `CpuExecutor::capabilities()`가 `Builder::build`에서는 유효하지만 설치 후 domain/worker/submit 선언을 바꿈; IO/local/blocking/CPU 경로를 교차 제출 | build 때 검증한 descriptor를 runtime이 동결하며 제출·debug·accessor는 adapter를 재조회하지 않는다. 변경된 adapter 선언이 panic·work 시작·permit charge·authority drift를 만들지 않음을 검증한다. |
| H31 | host가 예약한 permit을 embedder가 direct `governor().advance_phase`로 먼저 lease한 뒤 host가 dispatch하고 drain도 시작 | host가 소유하지 않은 token 아래에서 user work를 시작하지 않고 typed `PolicyViolation`을 반환한다. host lease drop은 외부 token의 capacity를 반환하지 않으며, `release_leased(token)` 전 drain `Ok`가 나오지 않는다. |
| H32 | 대상 class inflight가 가득 차고 다른 class의 measured overcommit으로 global memory도 포화; 대상의 일반 overflow는 `Reject`, memory policy는 `Queue`; 그 뒤 대상 holder를 release해도 memory 부족이 남음 | 처음에는 우선순위가 높은 `Inflight`가 primary라 즉시 거절한다. class blocker가 사라지고 Memory만 primary인 새 요청은 bounded queue에 들어간다. memory policy가 모든 blocker를 덮는다고 오판하지 않음. |
| H33 | JSON child의 `parent_awaits` 누락/오타로 false가 된 상태에서 실제 parent가 child 결과를 기다리고 단일 class/pool slot을 점유 | 현 `ValidatedTaskPlan` 형식 검증만으로 선언 누락을 감지하지 못하며 엔진은 undeclared wait를 추론하지 않아 queue할 수 있다. 외부 ingress의 strict decode/명시적 wait 선언을 통합 안전 조건으로 결정하고, 이를 엔진의 `NestedWaitCycle` 거절 성공으로 오인하지 않음. |
| H34 | requested-stack async root는 `RunFor`/`CompleteBy` 전에 정상 완료했으나 owned runtime의 `spawn_blocking` child가 오래 살아 있음 | `CompleteBy`는 caller 응답까지 제한한다. teardown이 기한을 넘으면 준비된 `Ok`/task `Err`를 버리고 `DeadlineExceeded`를 반환하며, child 종료까지 dedicated lease·pool custody는 유지한다. |
| H35 | `run_local` root가 `spawn_local` child를 띄운 뒤 await하지 않고 반환; 같은 root가 raw `tokio::spawn` child도 띄움 | 호출별 `LocalSet::run_until(root)`은 root 완료 시 끝나므로 local child 완료를 암묵적으로 약속하지 않는다. local child의 drop/side effect와 ambient Tokio child의 별도 수명을 구분하고, drain이 어떤 child를 추적하는지 명시한다. |

관련 앵커: `crates/taskmesh-bench/tests/{hellgate,inferno,fairness_property}.rs`, `crates/taskmesh-engine/tests/{hardening_fairness_reference,differential_model,hardening_effect_retirement,loom_governance,shuttle_governance,pending_resolver,hardening_close_admission}.rs`, `crates/taskmesh/tests/{e2e_chaos,hardening_drain,hardening_executor_authority,hardening_deadline_custody,hardening_executor_protocol,host_inferno}.rs`, `crates/taskmesh/src/runtime/claim_acquisition_tests.rs`. 기존 `hellgate.rs`는 **벤치 시뮬레이터/계측**의 헬게이트이고, `inferno.rs`는 direct Governor 적대 불변식이다. 위 H 항목 전체를 두 파일이 다 보증한다는 뜻은 아니다.

## D. 코너 — 경계값·정렬·wire 정확성

| ID | 경계 입력 | 필수 관측/판정 |
|---|---|---|
| D01 | quota/depth/pool/budget이 0, 1, 정확히 한계, 한계+1 | `0`이 각 필드에서 의미하는 바(무제한/비queue/invalid)를 혼동하지 않음; equality는 정책대로 허용·초과는 거절. |
| D02 | CPU/메모리 총량이 `u32` 도메인을 넘는 여러 permit; `u128` wire 왕복 | 공개 입력 경로에서는 큰 합계가 포화·wrap·JSON 부동소수점 손실 없이 정확. aggregate representability 실패의 sticky `RuntimeUnavailable`은 현 구조의 `u128` 총량·`u32` 단건 비용상 정상 API로 재현 불가하므로 내부 fault injection/모델 oracle로 분리. |
| D03 | `MAX_TASK_IDENTIFIER_LEN`의 -1/정확히/ +1 byte, 공백·NUL·비ASCII | byte 기준 검증과 오류 위치가 일정; thread 이름 sanitization은 governance identity를 변경하지 않음. |
| D04 | stage 0개/1개/중복/매우 많음, untrusted JSON bytes 대량 | core 형태 검증은 결정적이고 raw DTO Serde는 호환성을 유지한다. additive strict ingress의 `parse_task_spec`이 pre-decode byte/depth/stage 상한과 duplicate/unknown key를 강제하므로 외부 bytes 경계는 이 entrypoint를 명시적으로 채택해야 한다. |
| D05 | `Duration::ZERO`, 정확히 deadline 시각, `Instant::checked_add` overflow | ZERO acquire는 즉시 시도; deadline equality는 만료. async IO/local/requested-stack `RunFor`의 overflow는 typed `PolicyViolation`과 lease 반환, 시작된 sync detached `RunFor`의 overflow는 해당 timer만 사실상 unbounded이고 worker custody는 유지한다. 두 경로의 차이를 동일한 기대값으로 합치지 않음. |
| D06 | `MeasurementSequence=u64::MAX`, 같은 epoch 재전송, 측정 byte→unit 변환 불가 | `EpochExhausted`/stale/변환 오류가 상태를 바꾸지 않음; terminal sequence도 permit은 별도 release 필요. |
| D07 | leak staleness `now == last_touched + threshold`와 그 직후, clock 역행 | 경계는 명시된 strict 비교; monotonic commit watermark로 과거 시각이 새 lease를 늙게 만들지 않음. |
| D08 | WFQ weight 1과 극단값, DRR 큰 cost/작은 quantum, tie deadline | zero weight 구성 거절; 비영 가상 increment, 산술적 O(classes) 선택, 동일 deadline은 안정된 arrival 순서. `WFQ.burst`는 현재 예약 필드라 동작 효과를 기대하지 않음. |
| D09 | promotion pass 정확히 `PROMOTION_BUDGET` 및 +1 요청 | 정확히 경계면 가짜 continuation 없음; +1이면 이어서 처리. same-class head와 타 class 모두 순서 보존. |
| D10 | terminal ticket retention 정확히 `MAX_TERMINAL_TICKETS` 및 +1 | 보존 상한을 넘지 않고 oldest를 정의된 순서로 제거. `ticket_status()`는 read-only라 기록을 유지하고 terminal `claim()`/`abandon()`만 그 슬롯을 제거한다. |
| D11 | direct plan에 같은 capability hint 중복 및 여러 다른 hint | 중복 pool은 permit당 한 번만 예약; host는 실제 첫 dispatch의 role/physical domain만 예약; 뒤 stage의 실행·예약을 암시하지 않음. |
| D12 | `reconcile_memory`로 측정치가 상한을 초과한 live job | 실제 사용량을 snapshot에 보존하고 신규 admission을 닫음; 기존 사용량을 cap에 맞춰 거짓으로 깎지 않음. |
| D13 | `Snapshot`/`RuntimeConfig`/`TaskSpec`·legacy `PlanSource` alias 왕복 | **Snapshot만** `schema_version=2`를 가진다. `u128` decimal-string을 정확히 보존하고 raw config/spec·alias는 각 wire 계약대로 왕복한다. `Snapshot`의 derived Deserialize는 버전/보존식을 자동 검사하지 않으므로 수신 소비자가 기대 version과 helper가 검사하는 phase/누적/pool 상한을 확인해야 한다. helper 통과는 permit ledger와 held 자원 일치의 증명이 아니다. 모호한 legacy child parent는 임의 추측하지 않음. |
| D14 | requested-stack OS thread label에 허용된 `/`·`:`·최대 길이 class 이름; 별도로 NUL·비ASCII·상한 초과 이름 | 유효한 class는 label에서 안전하게 정규화·절단하되 실제 class/provenance는 원래 값 유지한다. 금지 문자는 spec validation에서 worker 생성 전에 `MalformedTask`로 거절; OS spawn 실패는 별도 typed worker 오류. |
| D15 | stack bytes `0`, `1`, `MAX_REQUESTED_STACK_BYTES`, 상한+1, target `usize` 초과; blocking/CPU/async dispatch 교차 | 숫자 validator의 허용/거절과 실제 OS thread 생성 성공은 별개다. 0·상한 초과·변환 불가는 typed preflight 오류, 숫자상 허용값도 플랫폼 spawn 실패면 typed worker 오류; 허용 경로에만 실제 dedicated dispatch. |
| D16 | `CpuMode::Auto`인 `RuntimeConfig`를 JSON 왕복하고 다른 machine에서 build | portable 선언과 inventory만 보존된다. resolved worker/capability limit은 `Governor::snapshot`, 설치 executor 선언은 `TokioRuntime::executor_capabilities`에서 확인; config JSON만으로 동일한 실행 capacity가 재현됐다고 주장하지 않음. |
| D17 | `close_admission` 이후 malformed raw spec 또는 foreign resolved capability 제출 | direct `admit`/`admit_waitable`의 malformed 검증과 `admit_resolved`의 capability 검증이 close 검사보다 앞서 각각 `MalformedTask`/resolution error가 우선한다. 유효 요청만 `RuntimeUnavailable`; rustdoc과 exact no-side-effect fixture가 이 순서를 고정한다. |
| D18 | custom pool을 등록했으나 다른 policy의 resolved handle 또는 이름 오타로 direct admit | foreign/unknown capability는 admission 전에 typed resolution error; 같은 문자열만으로 authority를 위조할 수 없고 state·ticket·waker 등록이 변하지 않음. |
| D19 | queued ticket을 읽기만 하면서 manual clock을 전진시키고, 그 뒤 governor transition을 발생 | `pending_view.queue_wait_ms`는 새 wall-clock sample이 아니라 마지막 committed watermark와 `enqueued_at_ms`의 차이. idle read만으로 증가했다고 기대하지 않고, 전이 후 monotonic하게 갱신되는지 판정. |
| D20 | wire `Snapshot`의 phase/누적 등식은 맞지만 `inflight=0`에 양수 `cpu_units_held`를 넣거나 capability inventory와 occupancy를 불일치시킴 | `conservation_violation()==None`이 나와도 완전한 원장 검증으로 취급하지 않는다. 현 helper 범위를 명시하고, 실제 governor의 `permit_ledgers()`·정책 inventory와 독립 재계산한 경우에만 내부 원장 일치를 판정한다. |
| D21 | 완전한 설정 JSON에 모르는 키를 추가하거나 `physical_domains`를 `physical_domans`로 오기 | raw derived Serde의 permissive 호환성은 유지한다. strict `parse_runtime_config`는 unknown/duplicate key를 builder 승격 전에 거절하므로 배포의 untrusted bytes 경계가 이 entrypoint를 채택해야 한다. |
| D22 | child JSON에서 `parent_awaits`를 빼거나 키를 오타 내고 `ValidatedTaskPlan`으로 decode | raw DTO는 호환성상 default/unknown-key 동작을 유지한다. strict `parse_task_spec`은 child wait 선언과 unknown key를 명시적으로 검사한다. decode 성공만으로 cycle 안전성을 주장하지 않고 배포 bytes 경계의 strict entrypoint 채택을 별도 추적한다. |
| D23 | 미래 `AdmissionVerdict`/`TerminalReason` variant를 구버전 Serde 소비자가 읽음 | Rust의 `#[non_exhaustive]`는 패턴 매칭 범위만 넓힌다. derived enum Deserialize의 unknown variant는 decode 오류이므로 소비자는 오류·version negotiation 경로를 갖고, `Snapshot.schema_version`이 다른 타입의 버전 표식인 것처럼 취급하지 않는다. |
| D24 | direct `advance_phase`를 `DispatchReserved → Running` 또는 `CleanupPending`으로 건너뛰고 반복·역행도 시도 | 첫 forward move만 lease token을 발급하고 `started_total`은 처음 started phase 진입 때 한 번만 증가한다. 반복·역행은 `NotLater`, release는 token 소유자만 가능. 인접 phase를 반드시 순서대로 호출해야 한다는 가짜 제약을 두지 않음. |
| D25 | blocking `TaskSpec` JSON에서 `stack_size_bytes`만 `stack_size_byte`로 오타 내거나 누락 | raw DTO는 permissive 호환성을 유지한다. strict ingress는 unknown dispatch key를 거절하고 blocking dispatch tag를 명시적으로 요구하며, accepted requested-stack payload는 실제 dedicated worker와 `large_stack` charge로 연결된다. 배포 bytes 경계의 strict entrypoint 채택은 외부 증거다. |

관련 앵커: `crates/taskmesh-contract/tests/{contract_roundtrip,snapshot_oracle,task_plan_validation}.rs`, `crates/taskmesh-engine/tests/{hardening_exact_accounting,hardening_memory_epochs,hardening_lifecycle,hardening_fairness_reference,substrate_inventory}.rs`, `crates/taskmesh/tests/{hardening_dispatch_resolution,hardening_executor_protocol,config_inventory}.rs`, `crates/taskmesh-contract/src/task.rs`, `crates/taskmesh/src/{execution_plan,runtime}.rs`.

## 최소 조합 매트릭스와 실행 배치

전체 직교곱 대신 **유효한 경로와 무효 조합의 사전 거절**을 모두 확인한다. 아래 각 행의 교차는 해당 기능을 지원하는 class·substrate에서만 수행한다.

| 축 | 필수 교차와 적용 범위 |
|---|---|
| admission blocker × 정책 | class quota / role pool / physical domain / CPU는 각 class의 `Reject` / `QueueWithinDepth` / `DropBestEffort`와 교차. memory는 별도 `MemoryOvercommitPolicy::{Reject,Queue,DegradeToLight}`와 교차하며, 일반 overflow와 다를 때 B05를 판정. |
| dispatch × 결과 | IO·local은 정상 / task error / caller cancel·drop·panic; blocking·CPU(default/Rayon)·requested-stack은 정상 / task error / worker panic·spawn 실패 / caller cancel·drop / 시작된 worker의 custody를 교차. `CompleteBy`와 stack 크기는 지원 경로만 실행하고 나머지는 typed preflight reject로 판정. |
| custody × 경쟁 | reserved는 release/sweep, queued·promoted-unclaimed는 abandon/claim/timeout, accepted-never-started는 closure drop/실행/cancel/drain, running·cleanup은 worker 종료/cancel/drain으로 나눠 교차. 상태에 없는 연산은 typed no-change로 판정. raw ID의 교차 Governor 충돌과 lease token 검증은 B28로 별도 판정. |
| scheduler × 교란 | FIFO / WFQ / DRR / deadline / scavenger의 지원 policy에서 중간 취소 / 다른 pool head / 빈 class 재진입 / promotion-budget 경계를 교차. tie와 장기 진전은 별도 독립 모델로 비교. |
| memory × lifecycle | Estimated / Measured / Hybrid와 stage release / reconcile / overcommit / sweep / worker-active를 지원 policy 범위에서 교차. `OnTaskCompletion`의 stage release는 B15의 typed reject를 기대. |
| scope × capacity | root / awaited child / non-awaited child / grandchild를 class / pool / CPU / memory blocker 및 자기 root만 / sibling·stranger 공유와 교차. 필수 parent field 누락은 Serde decode에서, 잘못된 identity 관계는 validator에서 거절; live parent stage의 존재성은 B22의 미결정 계약, 선언된 wait cycle은 engine resolver로 판정. |
| wire 신뢰 × 실행 의미 | raw/validated task와 외부 ingress의 strict/기본값 decode를 교차. `parent_awaits` 누락·`stack_size_bytes` 오타·설정 unknown key·미래 verdict variant를 각각 구조 검증, dispatch/입장 의미, 소비자 decode 오류로 나눠 판정. |
| child 수명 × 응답 | IO의 ambient `tokio::spawn`, local의 `spawn_local`, requested-stack owned runtime의 `spawn_blocking`을 root 정상 완료·취소·deadline과 교차. caller 응답과 worker custody를 별도 원장으로 관측. |
| executor 선언 × 제출 | build 시 검증한 domain/worker/submit 선언을 submit 전에 고정·변경하고 IO/local/blocking/CPU preflight를 교차. 두 runtime이 같은 bounded executor를 공유하는 동시 제출(H27)도 분리. 현재 재조회 범위와 목표 fail-closed 계약을 분리. |

## 현재 재감사: 시나리오·증거·코드 분리 (2026-09-26)

감사 소스는 clean `0588d26847ba575f1c059f8d86a37d8420ea5589`다. 해당 HEAD의 macOS CI receipt는 `--expected-head`로 검증됐고 필수 16개 gate가 PASS였다. 아래 문서 수정 이후의 새 HEAD 또는 dirty tree에 이 receipt를 재사용하지 않는다. 기존 관련 테스트 6개를 별도로 재실행해 통과했지만, 그 결과가 빠진 결합 순서나 성능 측정을 증명하지는 않는다.

| ID | 시나리오 문제 | 증거 문제 | 코드 판정·남은 작업 |
|---|---|---|---|
| H12 | 기존의 caller drop에 대한 typed 응답과 drain 뒤 신규 입장은 불가능한 oracle이었다. 위 행은 drop의 무응답, drain **전** queue된 요청의 진전, drain 뒤 신규 입장 거절로 정정했다. | `host_open_loop::terminal_caller_keeps_worker_charged_through_pre_drain_queue`가 deadline·cancel·caller drop 세 변형에서 응답/무응답 → worker custody → 두 번째 queue → drain `NotDrained` → holder 종료 → queue 진전 → drain `Ok`를 barrier로 고정했다. 후보 tree의 focused 1/1 및 `just dev` 558/558 PASS; 최종 clean-HEAD CI 증거는 별도다. | `runtime.rs::run_detached_job`/`await_detached`의 worker lease 및 `runtime/drain.rs::drain`의 close/wait 계약과 결과가 일치한다. 확인된 제품 코드 위반 없음. |
| H28 | 결정적 host↔simulator 비교와 quiet-host 성능 자격을 한 행에 섞었다. simulator에는 task error·cancel·deadline/worker cleanup 모델이 없으므로 그 terminal 유형까지 건수 동치를 요구하지 않는다. | `host_simulator_comparison`은 단일 class 9건 burst의 offered/completed/rejected/max-queue를 비교한다. `unanswered = offered - terminal`은 독립 모집단 계측이 아니며, host의 응답 후 살아 있는 worker, 여러 class/path, warmup·환경을 다루지 않는다. | simulator·host의 공통 admission 계정 CI fixture는 유효하다. 제품 코드 결함은 확인되지 않았다. 별도 host harness/quiet-host report가 필요한 경우 독립 offered·terminal·unanswered·custody 원장과 class/path·환경 메타데이터를 수집한다. |
| A05 | 요구 자체는 유효하다. | `hardening_admission_ledger::cross_class_global_and_pool_limits_follow_only_admitted_events`가 입력 기반 permit 원장으로 class별 CPU·memory·inflight와 pool 점유를 확인했으나 manifest에 누락됐었다. 현재 매핑의 supporting case로 연결했다. | 추가 제품 코드 변경 근거 없음. manifest의 정적 검증과 실제 CI 실행 판정은 분리한다. |

H12의 최종 clean-HEAD CI 자격과 H28 성능 자격의 실행 작업은 `docs/plans/bugbash-sep-25-general/tickets/OPEN-FOLLOWUPS.md`에서 추적한다. 104개 ID의 `MAPPED`는 테스트 선택 목록이며 모든 행의 의미 충족이나 release 자격이 아니다.

## 역사적 소스 감사 스냅샷

아래 표와 우선순위는 `76559c4` 당시의 감사 입력이며 현재 미해결 목록이 아니다. H30, H34, D17, B28 등은 이후 구현됐다. 현재 판정은 위 2026-09-26 재감사, `scenario-evidence.json`, `OPEN-FOLLOWUPS.md`를 따르고 당시 구현 판정은 ADR 0007에서 확인한다. fixture의 존재는 해당 시나리오 전체의 PASS나 clean-HEAD 자격을 뜻하지 않는다.

| 대상 | 확인된 근거 | 남은 판정 |
|---|---|---|
| H13·H14 direct settlement/drain | `crates/taskmesh/src/runtime/drain.rs`의 `SettlementWaker`; `crates/taskmesh/tests/hardening_drain.rs`의 direct release/abandon/reap 사례. | 두 drain caller, close↔snapshot↔wait lost wake, settlement callback panic 결합은 독립 barrier fixture 필요. |
| H19 accepted-never-started | `crates/taskmesh/tests/{hardening_executor_protocol,hardening_deadline_custody}.rs`의 custody·deadline 부분 사례. | closure 보유 상태의 `RunFor`/cancel/drop/drain 결합과 독립 occupancy oracle 필요. |
| H21·H35 child 수명 | `crates/taskmesh/src/runtime.rs`의 caller-owned IO/local lease와 호출별 `LocalSet::run_until`. | caller panic/drop, 미완료 `spawn_local`, ambient `tokio::spawn`의 서로 다른 수명을 명시할 fixture 필요. |
| H28 실제 host 부하 | `crates/taskmesh-bench/src/loadgen.rs`·`tests/hellgate.rs`는 simulator; `tools/bench-gate.sh`는 allocation gate, `tools/bench/perf-gate.json`은 allocation·instruction 기준. | 실제 facade open-loop 응답/worker custody ledger와 end-to-end latency를 별도 측정해야 한다. |
| D04·D21·D22·D25·H33 외부 입력 | `crates/taskmesh-contract/src/{task,validation,config,topology}.rs`: stage-count/preparse byte cap 부재, unknown key 허용, `parent_awaits` default false, optional `stack_size_bytes` default None. | ingress byte/key/await/dispatch 선언 검증 주체를 결정. 형식 decode 성공을 cycle 안전성·stack 선택·배포 설정 승인으로 취급하지 않음. |
| D13·D20·D23 wire 소비자 | `crates/taskmesh-contract/src/{snapshot,verdict}.rs`: Snapshot만 version 필드가 있고 helper는 phase·누적·pool 상한을 검사; enum은 derived Serde. | version·unknown variant 처리와 held/permit ledger 일치를 별도 확인. helper PASS를 전체 원장 증명으로 쓰지 않음. |
| D17 close 오류 우선순위 | `crates/taskmesh-engine/src/engine/governor.rs`의 preflight가 close보다 앞설 수 있고 `docs/taskmesh-external-interface.md`는 이를 명시. | `Governor::close_admission` rustdoc의 “every admit*” 표현을 현행 동작과 정렬할 계약 결정. |
| H25·D19 pending 진단 | `governor.rs::pending_view`는 현재 assessment에 continuation guard를 포함하고 cycle에서는 intake reason을 사용; wait 시간은 committed watermark 기준. | `docs/taskmesh-library-spec.md`의 intake 고정 설명을 현행 source와 정렬. |
| A14·D14 공개 설명 | `crates/taskmesh-contract/src/{verdict,validation}.rs`: runtime error는 non-Serde, NUL·비ASCII class는 사전 거절. | `docs/taskmesh-library-spec.md`의 NUL 실행 설명과 `runtime.rs` thread-label 주석을 소스와 정렬. |
| H30 executor 선언 변경 | `crates/taskmesh/src/builder.rs`는 build 때 선언을 검증하지만 `runtime.rs::plan()`은 제출 때 domain을 다시 읽어 `expect(...)`한다. | stateful adapter의 build→submit 변경을 주입해 panic/authority drift를 재현하고 선언 고정 또는 typed fail-closed를 결정. source-backed 미검증 실패 후보. |
| H31 direct lease 선점 | `crates/taskmesh/src/runtime.rs::ExecutionLease::advance`와 내부 `runtime/claim_acquisition_tests.rs`에 user work 거절의 좁은 사례. | 외부 token 보유와 drain을 겹쳐 custody가 반환될 때까지 기다리는지 확인. |
| B05·H32 복합 blocker | `crates/taskmesh-engine/src/features/admission/{pending.rs,mod.rs}`의 `Inflight → Capability → Cpu → Memory` primary 순서. | measured overcommit을 포함한 두 blocker fixture에서 정책 분기와 queue를 독립 판정. |
| H10·H16·H25 모델 범위 | `differential_model.rs`는 FIFO·class/CPU/두 pool 중심이고 memory reconcile·fairness·child scope·waker·reap·promotion budget을 명시적으로 제외한다. `model_replay_fixture.rs`는 실제 Governor의 두 admission 순서가 뒤집힌 schedule을 의도적으로 거짓인 선착순 가정의 실패로 저장·재생한다. | 실제 Governor의 stage release→reconcile→promotion→sweep 결합, 동시 history 독립 모델, class/pool/CPU/memory blocker 진단과 continuation 경계를 별도 fixture로 판정. 의도적 replay 실패를 Governor 결함 증거로 승격하지 않음. |
| H34 정상 root 뒤 teardown | `crates/taskmesh/src/runtime.rs`는 정상 requested-stack 결과를 owned runtime drop 이후 전달; 기존 `hardening_deadline_custody.rs` child fixture는 deadline/panic 중심. | live blocking child를 가진 정상 root의 응답 기한과 lease 점유를 재현. source-backed 계약/실패 후보. |
| B22·B25 입력/호출 범위 | parent stage는 문자열만 검증하며, blocking facade는 Tokio `spawn_blocking`을 호출한다. | parent stage membership과 Tokio runtime 밖 polling의 지원·실패 계약을 결정. |
| B27 preflight callback | `crates/taskmesh-engine/src/engine/governor.rs`는 일부 preflight 오류에서 `drop(waker)`를 직접 호출한다. | final-reference destructor panic의 무입장·무누수 결과를 transition-effect callback과 구분해 확인. |
| B28 raw ID 인스턴스 경계 | `PermitId`/`Ticket`은 `u64`, `Governor::construct`는 둘 다 1부터 시작; `hardening_child_scope.rs`가 숫자 충돌을 이미 보여준다. lease token만 전역 nonce를 쓴다. | raw ID 교차 전달 시 local 상태 작용을 명시적으로 재현하고 direct API scope 또는 governor-bound handle 계약을 결정. |
| H27 공유 executor 실측 | `hardening_executor_protocol.rs::two_runtimes_sharing_an_executor_each_govern_their_own_submissions`는 순차 제출이고 adapter가 요청마다 새 OS thread를 만든다. | 실제 고정 크기 공유 executor에서 두 runtime을 동시에 포화시켜 각자 상한과 합산 물리 occupancy의 차이를 관측. |

## 테스트 대응 감사 (2026-09-25, 정적)

분류 기준은 **K**=핵심 결과를 직접 단언하는 기존 fixture 있음, **P**=관련 fixture는 있으나 이 행의 조합·독립 oracle 일부가 빠짐, **G**=직접 fixture를 찾지 못했거나 목표 계약 결정이 선행됨이다. K도 행의 모든 조합이 검증됐다는 뜻은 아니다. 기준은 위 HEAD의 소스와 테스트를 읽은 결과이며, 실행 PASS·clean-HEAD 영수증·line coverage 백분율이 아니다.

| 구역 | K: 핵심 oracle 직접 단언 | P: 일부만 단언 | G: 직접 fixture/계약 공백 |
|---|---|---|---|
| A (16) | A01–A04, A06–A10, A12–A16 | A05, A11 | 없음 |
| B (28) | B01–B04, B06–B17, B19, B26 | B05, B18, B21, B24 | B20, B22, B23, B25, B27, B28 |
| H (35) | H03, H05, H08, H09, H22–H24, H26, H29 | H01, H02, H04, H06, H07, H10–H21, H25, H27, H31 | H28, H30, H32–H35 |
| D (25) | D02, D06–D12, D18, D24 | D01, D03–D05, D13–D16 | D17, D19–D23, D25 |
| 합계 (104) | 51 | 34 | 19 |

대표 근거: A01–A04는 `runtime_{io,blocking,cpu,local}.rs`와 `admission_queue.rs`, A12·H05는 `hardening_fairness_reference.rs`, H03은 `hardening_executor_authority.rs`, H08·H09는 `hardening_nested_wait.rs`, H22는 `hardening_close_admission.rs`/`hardening_drain.rs`, H23·H26은 `pending_resolver.rs`, H29는 `hardening_effect_retirement.rs`, D02는 `hardening_exact_accounting.rs`, D24는 `hardening_snapshot_projection.rs`에서 직접 관측한다. 이 파일들의 다른 테스트가 자동으로 같은 수준의 증거가 되는 것은 아니다.

| 우선 | 미충족 판정과 근거 | 필요한 fixture/결정 |
|---|---|---|
| P0 | H30: build 때 검증한 executor 선언을 submit 때 다시 읽고 `expect`한다(`runtime.rs::plan`). 선언 변경을 주입하는 테스트가 없다. | mutable `CpuExecutor`로 domain/worker/submit을 각각 바꿔 IO·local·blocking·CPU preflight, panic·work 시작·permit/authority drift를 확인한 뒤 선언 고정 또는 typed 거절을 결정. |
| P0 | H34: 정상 완료한 requested-stack root가 오래 사는 `spawn_blocking` child를 남길 때 caller의 `RunFor`/`CompleteBy` 기한을 검사하는 fixture가 없다. `hardening_deadline_custody.rs`는 deadline/panic 경로만 갖는다. | 정상 결과와 child teardown barrier를 분리해 응답 시각·phase·dedicated slot·drain을 함께 판정하고 기한 계약을 확정. |
| P0 | H33·D22·D21: `parent_awaits` 누락은 Serde 기본값 `false`, unknown key는 허용된다. `contract_builders.rs`의 누락 테스트는 `parent_operation_id`도 빠져 이 경계를 검증하지 않는다. | parent identity를 모두 채운 JSON에서 `parent_awaits`만 누락/오타 내고, 설정 unknown key도 별도로 넣어 decode→validation→입장 결과를 확인. strict ingress 소유자를 결정. |
| P1 | H19·H20·H12: accepted-held CPU와 requested-stack child의 개별 custody fixture는 있으나 `RunFor`/caller drop/두 번째 입장/drain/worker 종료를 한 원장으로 교차하지 않는다. | barrier로 accepted·running·cleanup을 고정하고 응답과 lease 해제를 독립 기록. |
| P1 | H10·H16·H25: memory epoch, differential model, Loom/Shuttle, pending 진단 테스트는 각각 좁은 경로를 검증한다. 현재 모델은 결합 memory/fairness/child/waker/reap 경로를 제외하며 replay fixture도 Governor failure가 아니다. | 주입 clock·barrier와 독립 원장으로 결합 상태를 작은 deterministic fixture에 먼저 고정하고, 그 상태 기계의 제한된 interleaving을 modelcheck에 추가. |
| P1 | H13·H14·H04: direct settlement, waker panic, claim 경쟁은 개별 검증되나 동시 drain caller와 close→snapshot→wait 경계, 네 방향 경합을 묶은 선형화 fixture가 없다. | 둘 이상의 drain waiter 및 release/claim/abandon/timeout/sweep barrier와 독립 permit/ticket 원장. |
| P1 | H28·H17: `taskmesh-bench/tests/hellgate.rs`는 단일 class 시뮬레이터다. `loadgen.rs` 단위 테스트는 단일 class 과부하에서 `lat.len() == started()`를 이미 확인하지만, 여러 class·seed의 모집단 fixture는 없다. host `e2e_chaos.rs`는 고정 제출 soak이며 open-loop 응답·custody를 계측하지 않는다. | simulator의 여러 class·seed에서 표본 모집단을 확장 검증하고, host에서 offered/terminal/unanswered와 응답 뒤 worker custody를 독립 계측. 성능 숫자는 별도 환경·baseline에서만 판단. |
| P1 | H27: 공유 executor 테스트는 두 runtime을 순차 제출하고 요청마다 새 OS thread를 만든다. 동시 물리 상한의 실제 관측은 없다. | 고정 크기 공유 executor와 barrier로 두 runtime의 동시 점유·queue·release를 판정. |
| P1 | B28: raw permit/ticket ID는 인스턴스별 숫자이며 충돌할 수 있다. 다른 Governor의 handle을 넘겨도 local 번호에 작용하는 direct API 범위가 문서상 선명하지 않다. | 교차 인스턴스 release/advance/claim/abandon 충돌 fixture 후 governor-scoped 사용 계약 또는 opaque handle 변경을 결정. |
| P1 | D25: JSON `stack_size_bytes` 오타가 optional None으로 조용히 바뀌면 blocking 작업의 dispatch·pool이 달라질 수 있다. | raw/validated decode와 실제 dedicated/shared dispatch를 같은 fixture에서 관측하고 strict ingress 경계를 결정. |
| P1 | B20·B23·B25·B27, H32, D17·D19·D20·D23: 각 항목의 정확한 반례 입력과 관측을 가진 테스트를 찾지 못했다. | local-only pool, queued sweep, Tokio 밖 poll, preflight waker drop panic, 복합 blocker, closed malformed input, pending clock watermark, forged held snapshot, 미래 enum variant를 각각 작은 결정적 fixture로 추가. |
| 계약 | B22·B24·B28·D04·D21·D22·D23·D25: parent stage membership, metadata 진실성, raw ID 인스턴스 범위, stage-count/JSON byte 제한, unknown key 및 미래 variant 처리는 현재 엔진이 일괄 거절하는 계약이 아니다. | API/ingress/consumer 중 검증 주체를 먼저 정하고, 현행 동작 고정 테스트와 목표 계약 테스트를 구분. |
| P2 | B18·B21·D03·D14: source/identifier의 NUL·비ASCII·UTF-8 byte 경계, 다른 class의 동일 root-operation 충돌, 잘못된 class의 OS spawn 전 거절은 현재 fixture의 핵심 결과 밖에 남는다. | 작은 입력표와 side-effect 0 assertion으로 각 하위 경계를 고정. |

**gate 연결 확인:** `just test`는 기본 feature의 workspace lib/tests를 실행한다. `just test-rayon`은 `taskmesh --features rayon --lib`와 `hardening_executor_authority`의 테스트 1개만 실행하므로 Rayon feature 전체 host integration matrix를 뜻하지 않는다. `just dev`는 bench harness를 제외하고, `just verify-macos-ci`의 CI profile은 bench/consumer/doc matrix를 포함한다. `.github/workflows/ci.yml`은 현재 `workflow_dispatch` 전용이며 push/PR 자동 검증이 아니다. `coverage-report`는 수치 보고만 하고 시나리오별 assertion coverage나 합격 문턱을 제공하지 않는다. modelcheck/TSan/fuzz 및 mutation은 별도 nightly/release 비용 rail이며 이번 감사에서는 실행하지 않았다.

**실행 rail:** A·B·D 및 좁은 H 회귀는 owner-local `contract`/`engine`/`taskmesh`/`bench` 타깃에 둔다. 주입 가능한 clock/barrier로 재현 가능한 H를 먼저 일반 `test`에 넣고, 깨끗한 동일 HEAD의 CI profile에서 소비자·feature·문서 타깃까지 묶는다. H16의 확장된 Loom/Shuttle·TSan, fuzz 및 생성 mutation은 별도 nightly/명시적 허가 rail이다. 성능 판단은 기능 PASS와 분리하여 고정 seed·독립 계측·환경·baseline을 기록한다. 이 문서는 실행 명령이나 합격 영수증이 아니다.

## 공백 해소 실행 계획

P 34개와 G 19개의 최종 정적 매핑은 [BG25 execution packet](../plans/bugbash-sep-25-general/tickets/README.md)과 `scenario-evidence.json`에서 관리한다. [S25 초안](../archive/2026-09-25/sep-25-engine-coverage/tickets/README.md)은 대체된 계획 이력이다. 이 체크리스트는 시나리오와 원래 정적 감사 상태를 보존한다.

## 근거 위치

- 현재 계약: `docs/taskmesh-library-spec.md`, `docs/taskmesh-external-interface.md`; crate 경계: `docs/adr/0001-hexagonal-feature-sliced-architecture.md`.
- 공개 타입·정책: `crates/taskmesh-contract/src/{policy,task,validation,topology,verdict,snapshot}.rs`.
- 실제 결정·소유권: `crates/taskmesh-engine/src/engine/{governor,state}.rs`, `crates/taskmesh-engine/src/features/**`, `crates/taskmesh/src/{builder,runtime,execution_plan}.rs`, `crates/taskmesh/src/runtime/drain.rs`.
- 선행 검증 카탈로그: `tools/gates/{inventory,required}.json`, `crates/*/tests/**`. 파일명·테스트명은 범위 탐색 증거이며 실행 결과가 아니다.
