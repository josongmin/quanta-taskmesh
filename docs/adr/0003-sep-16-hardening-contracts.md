# 0003. Sep-16 hardening — 확정한 계약 (D01–D16)

- 상태: Accepted
- 날짜: 2026-09-16
- 결정자: Song Min
- 선행: [0001 — Feature-sliced 헥사고날 아키텍처](0001-hexagonal-feature-sliced-architecture.md), [0002 — DRR proportional ring](0002-drr-proportional-fairness.md)
- 관련 계획: [Sep-16 structural hardening](../plans/sep-16-hardening/tickets/README.md)
- 원본 감사: [Sep-16 general audit](../bugbash/sep-16-general/tickets/README.md)

## Context

2026-09-16 감사는 40개 finding을 남겼고, 그중 다수는 *구현 버그*가 아니라 **확정되지
않은 계약**의 증상이었다. 같은 질문에 코드의 두 지점이 서로 다르게 답하고 있었다는
뜻이다. 아래 결정들은 그 질문들을 하나씩 닫는다. 각 항목은 선택·근거·거부한
대안·소비자 영향 순서로 기록한다. D01–D12는 최초 구현에서, D13–D16과 D05/D08/D09/D10의
개정은 구현 직후 실행한 3-track 적대적 감사([AUDIT-2026-09-16](../plans/sep-16-hardening/AUDIT-2026-09-16.md))에서
확정했다.

이 ADR은 구현된 계약을 기술한다. 계획 문서의 `PROPOSED`와 달리 여기 적힌 것은
`crates/` 안에 존재하고 regression test가 붙어 있다. 검증 범위는 이 문서 마지막
절에 명시한다.

---

## D01 — 소유권은 first poll의 bounded credit에서 시작한다

**선택.** 요청은 admission transition에서 semantic capacity와 physical capability를
**동시에** 얻거나, 클래스 자신의 `OverflowPolicy` 아래 bounded queue에 들어가거나,
즉시 거절된다. admission 앞에 무제한 대기 공간은 존재하지 않는다.

**근거.** 기존 구조는 physical slot을 semaphore로 먼저 잡고 semantic admission을
나중에 했다. 그 semaphore에는 depth 제한이 없었으므로 `max_queue_depth`와
`OverflowPolicy::Reject`는 요청이 *두 번째로* 도달하는 큐를 통제하고 있었다.
`Reject` 클래스에 32건을 제출하면 32건이 조용히 대기했고 snapshot의 `queued`는 0이었다
(TM16-001).

**거부한 대안.** gate를 admission 뒤로 옮기기 — permit을 쥔 채 worker queue에 무제한
적재되므로 같은 문제가 위치만 바꾼다.

**소비자 영향 (breaking).** 포화된 capability pool에서 비-queueing 클래스는 이제
대기하지 않고 `AdmissionVerdict::SubstrateSaturated`로 즉시 거절된다. 기존의
"결국 실행된다" 동작이 필요하면 클래스에 `OverflowPolicy::QueueWithinDepth`와 양수
`max_queue_depth`를 선언한다. queueing 클래스의 bounded 대기 후 만료는 기존대로
`SubstratePoolTimedOut`이다.

**`0`의 의미는 재해석하지 않았다.** topology slot count `0`은 계속 "제한 없음"이다.

---

## D02 — reservation, measurement, release는 서로 다른 사실이다

**선택.** permit ledger는 세 값을 분리해 보관한다.

| 값 | 의미 | 변화 |
|---|---|---|
| `original_estimate_units` | 정책이 예약한 값 | 변하지 않음 (provenance) |
| `remaining_reservation_units` | 아직 보유한 예약 | stage release로 감소, 증가 없음 |
| `effective_units` | 현재 과금액 | mode에 따라 재계산 |

`Estimated`는 *remaining* reservation을 과금한다. `MemoryReleasePolicy`는 강제
계약이며 `OnTaskCompletion` 클래스는 stage release를 받지 않는다. stage release와
measurement는 각각 sequence/epoch를 가진다.

**근거.** `reserve 8 → release 6 → reconcile`이 8로 되돌아갔다 (TM16-011): reconcile이
*불변* 원본 예약을 반환하는 함수를 호출했기 때문이다. reconcile은 측정을 보고하는
행위이지 반환된 용량을 다시 예약하는 행위가 아니다. 또 `memory_release_policy`는
어디에서도 읽히지 않아 문서상 계약이 장식이었다 (TM16-005).

**소비자 영향 (breaking).** `Governor::release_stage_memory`의 반환 타입이 `u32`에서
`StageReleaseOutcome`으로 바뀌었다. 정책 거부를 "0 units freed"로 접으면 호출자가
*거부됨*과 *반환할 것이 없었음*을 구별할 수 없다. stage release를 쓰는 클래스는
`OnStageBoundary` 또는 `LeakDetecting`을 선언해야 한다.

---

## D03 — fallback은 자원 재분류이지 정책 이전이 아니다

**선택.** `MemoryOvercommitPolicy::DegradeToLight`는 요청을 다른 클래스의 *자원
계정*으로 넘긴다. cancellation policy, deadline 권한, 그리고 요구 capability는
선언된 클래스의 것으로 유지된다.

**근거.** capability는 *작업*의 속성이다. 메모리 압박 때문에 작업이 갑자기 다른
worker pool에서 실행 가능해지지는 않는다. full-policy fallback이 필요한 소비자가
확인되면 별도 variant로 추가한다 — 조용히 의미를 바꾸지 않는다.

**관측 방법.** 선언 클래스는 caller의 `TaskSpec`에, 유효 클래스는
`Governor::permit_ledger(permit).class`에 있다. 세 번째 사본은 두지 않았다.

**degrade는 한 hop이다.** 이미 degrade된 요청은 fallback 클래스의 자원 계정으로
admit되며, 그 fallback 자신의 `memory_overcommit_policy`는 다시 참조하지 않는다.
따라서 fallback이 다시 `DegradeToLight`를 선언하는 구성(`a → b → c` 체인, 또는
`a → b → a` 순환)은 런타임이 하지 않는 두 번째 hop을 약속하는 것이다 — 체인이면
조용히 건너뛰어지고, 순환이면 "두 번째 hop"이 방금 실패한 클래스다. 두 경우 모두
`Governor::validate_policy`가 구성 시점에 거절한다(`degrades to fallback …, which
itself degrades`). full-policy chained fallback이 필요한 소비자가 확인되면 hop 수를
계약으로 명시한 별도 variant로 추가한다.

---

## D04 — stack 요청은 large-stack capability를 소비한다

**선택.** `stack_size_bytes`가 있는 blocking-family 제출은 어떤 hint로 표기되었든
`large_stack` capability를 예약한다.

**근거.** stack 요청은 실제 실행 substrate를 전용 OS thread로 바꾸지만 capability
gate는 선언 hint를 따랐다. 결과적으로 `large_stack_slots = 1`인데 두 개의 전용
worker가 동시에 실행됐다 (TM16-023). `BackgroundOnly`도 같은 분기에 도달했다.

**거부한 대안.** hint 불일치를 사전 거절 — 기존 `TaskSpec::blocking(c).stack_size_bytes(n)`
호출자를 전부 깨뜨린다. 재분류는 호환되면서 cap을 실제로 적용한다.

---

## D05 — adapter는 자신이 보장하는 것만 선언한다

**선택.** `CpuExecutor`에 default-구현된 `capabilities()`를 추가했다. 기본값은
[`ExecutorCapabilities::legacy`] — submit이 blocking일 수 있고, worker 수 미상,
pool을 ambient 사용자와 공유. host는 adapter가 선언한 것 이상을 가정하지 않는다.

**근거.** inline executor(= `spawn`이 작업 완료 후 반환)는 합법이며, 그 경우
"spawn 반환 후 타이머 시작"은 의미가 없다 (TM16-022). 여러 Runtime이 하나의
`Arc<dyn CpuExecutor>`를 공유하면 각자 *자기* 제출만 제한한다 — 공유 pool 전체에
단일 budget이 있는 척하지 않는다.

**명시적 한계.** managed pool 밖의 ambient Tokio/Rayon 작업은 taskmesh가 제한하지
않는다. 그렇게 문서화했고 test로 고정했다.

**개정 (감사 A2-P0-2).** 최초 구현에서 `capabilities()`는 아무도 읽지 않는 선언이었다.
지금은 load-bearing이다: `Builder::build`는 adapter의 `declared_workers`가 topology가
resolve한 `cpu` gate보다 작으면 `TopologyError::ExecutorDeclaresFewerWorkers`로
거절한다 — gate가 pool보다 넓으면 pool이 동시에 실행할 수 없는 작업을 admit하고, 그
불일치는 "설명되지 않는 queueing"으로만 나타나기 때문이다. `RayonCpuExecutor`는 pool의
실제 thread 수와 exclusivity(`with_pool`은 shared)를 선언하고,
`BlockingPoolCpuExecutor`는 worker 수를 *unknown*으로 남긴다(unknown은 "fewer"가
아니므로 거절되지 않지만 보장으로 승격되지도 않는다). `TokioRuntime::executor_capabilities()`가
선언을 노출한다. `nonblocking_submit`은 동작을 바꾸지 않는다 — host는 모든 adapter에
대해 worker 자신의 시작 시각을 기준으로 삼으므로 inline adapter도 같은 규칙으로 안전하다;
이 필드는 operator가 읽는 선언이다.

---

## D06 — aggregate는 정확하고, wire는 그 정확도를 유지한다

**선택.** per-request cost는 `u32`, aggregate는 `u128`이며 모든 capacity 비교는
checked다. snapshot의 held 값은 decimal **string**으로 직렬화한다. bytes→units
변환은 `Result`를 반환한다.

**근거.** `saturating_add(cost) > budget`은 budget이 `u32::MAX`일 때 overflow를
감지하지 못했다: 각 `2^31`인 두 요청이 `2^32 - 1` budget에 모두 승인됐고, 포화된
합계가 consistency check에서도 문제를 가렸다 (TM16-008). 포화는 실제보다 *적은*
자원을 보고하며, 그것이 정확히 초과 승인이 스스로를 숨기는 방식이다.
`max_*_units = 0`(무제한)에서 aggregate가 `u32::MAX`를 정당하게 넘을 수 있으므로
폭 확장은 선택이 아니라 필요다.

**JSON number를 쓰지 않은 이유.** 큰 `u128`은 `f64`를 거쳐 왕복하면 값이 달라진다.

**소비자 영향 (breaking).** `Snapshot`/`ClassSnapshot`에 `schema_version`,
phase gauge, 누적 counter가 추가되고 held 필드의 폭과 wire 표현이 바뀌었다.
`SNAPSHOT_SCHEMA_VERSION = 2`.

---

## D07 — commit time은 monotonic하고, Clock은 lock 밖에서 읽는다

**선택.** engine은 state mutex를 **잡기 전에** `Clock`을 샘플링하고, transition
안에서 `max(sample, watermark)`로 clamp한 뒤 watermark를 전진시킨다.

**근거.** 두 제약이 동시에 성립해야 한다. driven port는 host 코드이므로 mutex 아래
호출하면 안 되고(재진입·blocking), 그렇다고 lock 밖 샘플을 그대로 쓰면 샘플과 commit
사이의 지연이 "생성되자마자 stale인 lease"를 만든다 (TM16-026). watermark는 그 간극을
닫는다: 늦게 도착한 샘플이 이미 commit된 것보다 과거를 기록할 수 없다.

**명시적 한계.** clamp된 값은 *논리적* commit time이다. lock 대기 시간의 실측이
아니며, 이 trait은 `now_ms` 자체의 monotonicity를 요구하지 않는다.

---

## D08 — fairness는 완전히 runnable한 후보만 고른다

**선택.** scheduler 후보는 semantic capacity와 head가 intake에서 동결한 capability를
**둘 다** 만족해야 한다. 클래스 내부는 strict FIFO를 유지한다. 취소된 요청은
service debt를 남기지 않고, drain된 큐는 credit을 이월하지 않는다.

**근거.** 물리적으로 실행 불가능한 클래스를 고르면 그 뒤에 두 번째 scheduling queue가
생기고 거기서 도착 순서가 클래스 fairness를 덮어쓴다. 취소 debt(TM16-014)와 idle
credit(TM16-037)은 둘 다 *현재 큐 상태가 동일한데 과거 이력이 순서를 결정하는* 형태였다.

**WFQ precision.** fixed-point scale을 `2^64`로 올려 허용 weight 전 범위
(`1..=u32::MAX`)에서 increment가 0이 되지 않게 했다. weight `0`은 비례 의미가 없으므로
construction에서 거절한다 — `max(1)` 강제는 명백히 잘못된 설정을 의도된 설정처럼
보이게 만든다.

**DRR 작업량.** ring을 **산술적으로** 순회한다. 같은 선택, 같은 deficit,
`O(classes)` — 기존 순진한 루프는 `O(cost/quantum)`이고 public 정책 범위에서
`u32::MAX`회 반복에 도달할 수 있었다 (TM16-012). `GovernedState::drr_ring_visits`가
검사한 ring 위치 수를 세는 work oracle이며, test-util에서 `Governor::drr_ring_visits()`로
읽는다 — wall-clock이 아니라 count로 상한을 고정한다.

**개정 (감사 A1-P1-1) — promotion budget 경계.** `promote`는 `select` *전에* budget을
검사한다. `select`는 선택된 클래스에 dispatch 비용(DRR deficit, ring cursor, WFQ
virtual time)을 부과하므로, 선택 후 budget 초과로 중단하면 64번째마다 dispatch되지
않은 비용이 남아 다음 pass가 한 클래스 늦게 시작했다. budget 소진 후 남은 작업 여부는
부작용 없는 `has_runnable`로 묻는다. reference 모델과의 일치는 두 budget 경계를
가로질러 검증한다.

**개정 — 도착 순서의 정확한 의미.** capacity가 비어 있는데 runnable한 작업이 queue에
있을 수 있는 순간은 하나뿐이다: promotion pass가 budget을 소진하고 lock을 놓은 뒤
continuation이 돌기 전(gap). 그 gap에 도착한 요청은 두 규칙을 따른다:
1. *같은 클래스*의 runnable한 head가 queue에 있으면 새 도착은 `CapacityBlock::QueuedBehind`로
   그 뒤에 선다 (`queued_head_blocks`).
2. continuation이 pending이면(`GovernedState::promotion_pending`) queue할 수 있고 *자기
   queue가 비어 있는* 클래스의 새 도착은 `QueuedBehind`로 queue되어 그 클래스의 head가 되고,
   continuation pass가 tier discipline으로 다른 클래스와의 순서를 정한다 — 클래스 *간* 추월도
   gap에서 일어나지 않는다. 자기 queue가 비어 있지 않으면 규칙 1이 결정한다(runnable head면
   뒤에 서고, 다른 pool에 막힌 head면 서지 않는다 — 막힌 head 뒤에 세우면 그 pool의 포화에
   묶이는 head-of-line 손실이 gap 안에서 재현된다). queue할 수 없는 클래스(`Reject` overflow
   *이고* memory 정책도 `Queue`가 아닌)는 기다릴 곳이 없으므로 admit된다; 이것이 gap에서
   runnable head를 추월할 수 있는 유일한 종류의 작업이다.
유일한 *동일 클래스* 추월 예외는 head가 *다른* capability pool에 막혀 있을 때다 —
같은 클래스, 다른 물리 pool이며, 이때 newcomer를 잡아두면 한 pool의 포화가 그 클래스가
쓸 수 있는 다른 모든 pool을 놀리게 한다. `QueuedBehind`는 항상 queue로 간다(class가 queue를
갖고 있다는 뜻이므로): `Reject`-overflow지만 memory `Queue`인 클래스도 queue에 합류한다.
`pending_block_reason`/`PendingView::blocked_on`은 *intake 시점*의 이유다(그 뒤 다른 한계에 막혀도
갱신하지 않는다 — 진단용이지 상태 머신의 입력이 아니다). 그리고 이 규칙의 대가를 한 문장으로: 같은
클래스에서 pool에 막힌 head *뒤에* 선 pool 불필요 요청 H2는, 그 pool이 차 있는 동안 나중에 온
pool 불필요 newcomer들에게 계속 추월당한다 — 클래스 내부 FIFO는 head를 넘지 못하고, 예외는 newcomer에게만
열리기 때문이다. 이것은 head-of-line 손실을 클래스 전체가 아니라 그 pool을 기다리는 요청들로 한정하는
선택이며, pool이 풀리면 H1·H2가 순서대로 나간다.
gap은 lock이 풀린 어느 admit에서든 관찰되지만, test는 promotion 중 waker에서 재진입하는
admit를 결정적 vantage point로 쓴다 (same-class, cross-class, Reject+memory-Queue, gap 안의
capability 예외 각 1개, gap 밖의 capability 예외 1개 — 각각 mutation을 가진다).

---

## D09 — acquisition budget은 모든 대기를 포함한다

**선택.** budget 만료 후에는 즉시 승인된 permit이라도 작업을 시작하지 않고 unwind한다.
`Some(Duration::ZERO)`는 기존 의미(*대기하지 말고 시도*)를 유지하므로 비경합 즉시
획득은 성공한다. `RunFor`는 worker 자신의 시작 시각을 기준으로 한다.
completion-at-deadline tie는 deadline이 이긴다.

**근거.** deadline select는 async wait에만 적용되고 synchronous admission의 lock
대기에는 적용되지 않았다. 이미 만료된 budget으로 받은 permit이 성공 처리되어 작업
side effect가 시작됐다 (TM16-032).

**명시적 한계.** 이것은 사후 검사다. OS preemption을 포함한 hard response-latency
상한을 제공하지 않는다.

**개정 (감사 A1-P2-1) — queued 경로의 대칭.** 최초 구현은 즉시 admit에만 이 규칙을
적용했고, queue 대기 후 timeout과 경합한 promotion은 "last-chance claim"으로
*시작*시켰다. 지금은 대칭이다: promotion이 budget 만료 후 도착하면(경합 포함) permit을
시작하지 않고 반환(`return_unstarted`)하며 caller는 timeout verdict를 받는다. timeout
시점의 마지막 조회는 engine이 이미 아는 이유(Reclaimed/Invalid)를 generic timeout보다
우선해 보고하기 위한 것이지, 늦은 permit을 통과시키기 위한 것이 아니다.

---

## D10 — caller의 응답과 작업의 custody는 다른 사건이다

**선택.** deadline 응답·취소·caller future drop은 *caller의* 대기를 끝낸다. worker는
실제로 종료할 때까지 — 소유한 runtime의 teardown을 포함해 — execution lease를 계속
보유한다. 정상 완료 시에는 custody가 결과와 함께 이동하므로 caller가 값을 볼 수 있는
시점에는 용량이 이미 반환되어 있다.

**근거.** 두 방향 모두 깨져 있었다. requested-stack worker는 결과를 보낸 *뒤* lease를
놓아서 완료 직후 snapshot에 유령 inflight가 남았고 (TM16-015), 반대로 소유 runtime
teardown이 응답보다 앞서서 blocking child가 deadline 응답을 무한정 지연시킬 수 있었다
(TM16-024). 성공 경로와 terminal 경로는 반대 순서를 원하므로 메시지를 두 variant로
나눴다.

**blocking `RunFor` (TM16-002).** unsupported 사전 거절 대신 **caller-wait deadline**을
택했다. 시작된 `spawn_blocking` 작업은 abort할 수 없으므로 budget은 caller의 대기를
제한하고 worker는 계속 charged 상태로 남는다. 동기 작업의 종료를 보장한다고 표현하지
않는다.

**개정 (감사 A1-P2-2) — panic 경로.** requested-stack async worker의 owned runtime은
panic boundary *밖*에서 만들어 이 frame이 소유한다. root future가 panic하면 caller에게
`WorkerPanicked`를 먼저 보내고 그 다음 runtime을 teardown한다(teardown은 abort할 수
없는 blocking child를 기다린다). 최초 구현은 runtime이 guarded closure의 local이라
unwind 중 teardown이 먼저 일어나 caller가 child가 끝날 때까지 panic을 듣지 못했다.
TM16-002/TM16-024/이 경로의 test는 모두 5초 상한과 release-on-drop fixture를 가져
regression이 hang이 아니라 assertion으로 나타난다.

---

## D11 — PM는 validate → render plan → apply로 분리한다

[H16-020](../plans/sep-16-hardening/tickets/H16-020-pm-validated-render-plan.md) 참조.
lint는 읽기 전용이고, target identity는 basename이 아닌 검증된 relative path이며,
중복 YAML key는 dict로 접히기 *전에* 거절한다. 실제 repo target overwrite는 이 결정의
범위가 아니며 별도 승인이 필요하다.

---

## D12 — 선언된 wait cycle은 지원하지 않는다

**선택.** bounded queue는 deadlock 회피 수단이 아니다. 같은 capability의
parent-await-child 같은 구조는 지원 범위 밖으로 명시한다. opaque user closure 내부의
임의 wait graph를 자동 감지한다고 약속하지 않는다. capacity 대여나 structured DAG
scheduler는 별도 RFC다.

**borrowed/`!Send` payload는 보존한다.** `run_local`의 non-`Send` future는 caller
task에 남는다. 중앙 kernel에는 metadata와 handle만 둔다 — 중앙화가 `Send + 'static`을
강제하는 재작성이 되어서는 안 된다.

---

## D13 — 모델 검사는 replica가 아니라 production Governor를 돌린다

**선택.** engine에 `src/sync.rs` seam을 두었다. production은 `parking_lot::Mutex`와
`std` atomics를, `--cfg loom --features loom` / `--cfg shuttle --features shuttle`
빌드는 checker의 mutex/atomics를 같은 `Governor` 코드에 컴파일한다.
`tests/loom_governance.rs`(5개, 9–810 interleaving 전수)와 `tests/shuttle_governance.rs`
(6개: 4개 × 10,000 schedule + D08 gap 모델 2개 × 5,000 schedule)는 실제 `admit`/`claim`/`abandon`/`release`/`reap_leaks`/
`reconcile_memory`를 호출한다: promote-claim-abandon 3-way exactly-once, `Pending` 뒤에
반드시 wake가 오는 lost-wakeup freedom, claim-vs-reap 양방향 fail-closed, 무순서 동시
reconcile 양쪽 적용, 4-thread churn conservation.

**근거.** 감사 A2-P0-1: 최초 구현의 loom/shuttle 파일은 locking 설계를 손으로 베낀
toy model이었고 새 seam을 전혀 타지 않았다. replica는 replica가 옳다는 것만 증명한다.

**거부한 대안.** loom을 dev-dependency로 두고 replica를 유지. 검사 대상과 배포 대상이
다르면 검사가 의미를 잃는다.

**소비자 영향.** 없음. checker crate는 optional feature 뒤에 있어 downstream lockfile에
resolve되지 않는다(consumer-MSRV 1.81 빌드는 그대로 PASS). cfg와 feature는 양방향으로
짝지어져 있다: cfg만 켜면 checker crate가 없어서, feature만 켜면 `#![cfg(loom)]` 모델 파일이
0개 test로 컴파일되어 rail이 조용히 green이 되므로, 둘 다 `compile_error!`로 거절한다.

---

## D14 — 조용히 무시되는 것은 없다: 실패는 typed이고 outcome은 must_use다

**선택.**
- `Governor::release` → `ReleaseOutcome::{Released, UnknownPermit}` (`#[must_use]`).
  `UnknownPermit`은 double release이거나 sweep에 reclaim된 lease이며, 둘 중 어느 쪽인지
  holder가 알아야 한다. host `ExecutionLease`는 dispatch 전 window의 reclaim만 정상으로
  받아들이고, dispatch 후의 `UnknownPermit`은 double release로 (debug) 단정한다.
- `ExecutionLease::advance` → `Result`. permit이 사라졌으면
  `GovernorError::LeaseReclaimed { permit_id }`로 작업을 시작하지 않는다 (내부적으로 lease를
  disarm하지만 그것은 방어적 조치다 — permit이 사라진 뒤에는 engine이 어차피 `UnknownPermit`을
  답하므로 관찰 가능한 차이는 없다).
  live permit이 phase 전진을 거부하면(host 순서 오류) `PolicyViolation`.
- `SubmitOptions::deadline`을 `CooperativeWithDeadline`이 아닌 클래스에 주면
  `GovernorError::DeadlineUnsupported { class, policy }`. 이전에는 deadline이 조용히
  버려지고 작업이 끝까지 실행된 뒤 `Ok`가 돌아왔다. **breaking**: `deadline_ignored_for_plain_cooperative`가
  보장하던 동작이 사라졌다.
- worker 실패는 `WorkerPanicked { context }` / `WorkerUnavailable { context, detail }`
  (spawn/runtime 생성 실패; 작업은 시작되지 않았다) / `JobAbandoned { context }`(adapter가
  closure를 실행하지 않고 버림)로 구분된다. 이전에는 모두 `PolicyViolation(String)`이었다.
  `PolicyViolation(String)`은 남아 있다 — host 구성/사용 오류(blocking 경로의 `CompleteBy`,
  stack size 누락/초과, `Instant` 범위 초과, 비단조 phase 선언)에 한해서다. 이들은 caller의
  코드가 고쳐야 하는 사용 오류고, 실행 중 일어나는 실패가 아니다.
- `MemoryOvercommitPolicy::DegradeToLight`의 fallback이 disabled(`max_inflight = 0`)면
  구성 시점에 거절한다. 이전에는 degrade된 모든 요청이 잘못된 클래스를 가리키는
  `ClassDisabled`로 shed됐다.

**근거.** 감사 A1 P2-4/P2-5/P2-6, A2 H16-011-A06. 사용자 규칙: 조용한 실패 금지.

---

## D15 — lease 활동과 epoch 배정은 그 transition 안에서 일어난다

**선택.** `claim`은 lease를 touch한다(sweep의 staleness 시계가 promotion이 아니라
claim에서 다시 시작). `reconcile_memory`는 epoch를 같은 lock 안에서 배정한다 — 두 lock에
걸쳐 "다음 epoch"를 읽고 적용하면 무순서 reporter 둘이 같은 epoch를 골라 늦은 쪽이
stale로 버려졌다(loom 모델 `unordered_concurrent_reconciles_both_apply`가 이 경합을
전수 검사한다). terminal ticket retention은 `O(log n)` 색인으로 바뀌었고
`Governor::retained_terminal_tickets()`로 관찰된다. 순수 spec 검증(`precheck`)은 lock
밖에서 한 번만 실행된다(transition 안에서는 `debug_assert`로만 확인), recursion guard 조회는
spec 문자열을 빌려 allocation 없이 한다.

**근거.** 감사 A1 P2-6/P2-7/P3.

---

## D16 — 증명 도구는 스스로를 증명한다

**선택.**
- 증명은 세 층이다: **모델**(loom/shuttle — production `Governor`를 checker의 메모리 모델
  위에서), **실제 메모리 시스템**(`just tsan` — nightly `-Zbuild-std -Zsanitizer=thread`로
  engine/host 동시성 test를 실제 parking_lot·Tokio·OS thread 위에서; 도구가 없으면 NOT_RUN),
  **객관 지표**(`just coverage-report` — cargo-llvm-cov 수치를 receipt에 기록; 절대 threshold가
  아니다. 이 저장소는 coverage gate를 약속하지 않는다). 첫 TSan 실행이 실제 취약점을 하나
  드러냈다: `stack_size_bytes(u64::MAX)`가 "OS가 거절한다"에 의존했는데 debug-built `std`에서는
  spawn 안에서 overflow panic이 났다 → host가 `MAX_REQUESTED_STACK_BYTES`(16 GiB) 초과 요청을
  결정적으로 거절한다. 환경 의존 동작은 계약이 아니다.
- CI `qualification` job이 clean checkout에서 `receipt.py collect`를 돌려 receipt of record를
  artifact로 남긴다; verdict가 QUALIFIED가 아니면 job이 red다. local receipt는 증거지 자격이
  아니다.
- mutation inventory(65개)의 모든 non-control entry는 `expect_message`를 가져야 하며
  runner가 이를 거부한다. 그 message는 named test *자신의* 출력 블록(libtest의
  `---- name stdout ----`, pytest의 `___ name ___`/`FAILED …::name`)에서만 찾는다 — 다른
  실패 test가 공유 helper 문자열로 이유를 대신 채울 수 없다. control은 `finding=control`로만 표기한다. `runner: pytest`
  entry가 Python 도구(allocation gate, receipt validator, gate-inventory parity, iai
  runner-version, arch checker)를 그 도구의 test로 증명한다. `profile: release`는 engine의
  debug invariant가 test보다 먼저 fire하는 경우(TM16-010)에 쓴다. CI는 `--require-clean`.
- hang-only였던 TM16-002/TM16-024 test는 5초 상한 + release-on-drop으로 bounded되었고
  각각 mutation을 가진다. TM16-012는 count oracle(`drr_ring_visits`)과 bounded thread 두 겹이다.
- `tools/bench-iai.sh`는 `iai-callgrind-runner --version`(존재하지 않는 flag)에 죽는
  대신 `cargo install --list` 또는 runner의 self-report로 버전을 읽고, 모든 early exit가
  진단을 출력하며, baseline stamp는 bench *성공 후*에만 쓴다. CI cache key는 script가
  계산하는 fingerprint 그 자체다. 이 script는 PATH shim으로 end-to-end test된다(13개).
- `just proof`는 `required.json` 전체로 expand되어야 하며 `gates-inventory`가 이를
  검증한다(workflow `run:` block은 YAML로 parse). `just matrix`가 proof-matrix job이다.
  parity는 *집행*이다: inventory gate를 호출하는 step에 `continue-on-error`, `if:`,
  `|| true`/`set +e`/`exit 0`이 있으면 validator가 거절한다 — 호출만 있고 실패할 수 없는
  gate는 아무것도 아닌 것 위의 green check다.
- receipt `validate`는 digest뿐 아니라 HEAD와 dirtiness를 tree에서 재도출하고 sub-receipt와의
  일관성을 검사한다. gate 결과 자체는 `collect`가 attest하며 그 경계를 문서화했다.
- semgrep test-quality 규칙은 `#[tokio::test]`(flavor 포함)를 인식하고, 38개 규칙 전부
  fire/clean fixture를 가진다(fixture 없는 규칙은 test가 실패시킨다). boundary 규칙은
  `src/**` 전체(in-src test module 포함)와 fully-qualified path를 본다.
- allocation gate threshold는 측정값(3.0)과 같고, probe는 알려진 1,000회 allocation을
  세는 self-check를 보고한다(`counter_check`). reject 경로는 real producer로 test된다.

**근거.** 감사 A2 P1-1/P1-3/P1-4/P1-5, A3 P0-1/P1-1/P1-2/P1-3/P1-4/P2-*.

---

## 소비자 표면 inventory

breaking 결정(D01/D02/D06/D10)이 무엇을 건드리는지 확정하기 위해 in-repo 소비자를
열거했다. **외부 소비자는 UNKNOWN이다** — 이 저장소 밖에서 이 API를 쓰는 코드는
조회하지 않았고, 조회할 권한도 이번 작업 범위에 없다.

| 표면 | in-repo 소비자 | 이번 변경의 영향 |
|---|---|---|
| `Builder` 기본 경로 (default CPU executor) | `crates/taskmesh/tests/**`, `crates/taskmesh/examples/quickstart.rs` | 없음. topology 검증이 추가되었으나 유효한 설정의 동작은 동일 |
| `Builder` + `rayon` feature | `crates/taskmesh-rayon/tests/rayon_smoke.rs` | 없음. worker 수는 이제 builder가 한 번 resolve해 공유 |
| 직접 `Governor` (via `taskmesh::ext`) | `crates/taskmesh-engine/tests/**`, `crates/taskmesh-bench/**` | **breaking**: `claim` → `ClaimOutcome`, `release_stage_memory` → `StageReleaseOutcome`, snapshot 폭/필드 |
| custom `CpuExecutor` | `taskmesh-rayon`, host 기본 adapter, 6개 test adapter | additive: `capabilities()`는 default 구현이 있으므로 기존 impl은 그대로 컴파일된다. **단**, `declared_workers`를 선언하는 adapter가 topology의 `cpu` gate보다 작으면 `build()`가 거절한다 (D05 개정) |
| `SubmitOptions::deadline` on non-`CooperativeWithDeadline` class | `deadline_cancel.rs` (test 1건) | **breaking**: `DeadlineUnsupported`로 거절 (D14). 이전에는 무시 |
| `GovernorError` match arms | host tests 10곳 | additive (`non_exhaustive`): `DeadlineUnsupported`, `LeaseReclaimed`, `WorkerUnavailable`, `WorkerPanicked`, `JobAbandoned`; worker 실패가 더 이상 `PolicyViolation`이 아니다 |
| `Governor::release` | engine/bench tests 145곳 | **breaking**: `ReleaseOutcome` (`#[must_use]`); 통계상 statement-form 호출은 `assert_eq!(…, Released)`로 바뀌었다 |
| custom `Clock` | `SystemClock`, `ManualClock` (contract 제공분만) | 없음. 호출 위치가 lock 밖으로 고정되었을 뿐 trait은 그대로 |
| `run_local` non-`Send` payload | `runtime_local`, `local_runtime_guard`, `e2e_proof`, `e2e_scenarios`, `host_edge_paths` bench | 없음 — 의도적으로 보존 (D12) |
| `Snapshot` serde 소비자 | `contract_roundtrip` test | **breaking**: `schema_version = 2`, held 필드가 decimal string |

`PolicySet`의 substrate registry는 public field에서 accessor로 바뀌었다. in-repo
소비자는 `builder.rs` 한 곳뿐이었다.

## 검증 범위

- 위 계약들은 각각 regression test를 가진다:
  `crates/taskmesh-engine/tests/hardening_*.rs`,
  `crates/taskmesh/tests/hardening_*.rs`,
  `crates/taskmesh/src/runtime/claim_acquisition_tests.rs`,
  `crates/taskmesh-contract/tests/topology_validation.rs`.
- `just mutants-critical`: 65 entries — 64 KILLED + 1 CONTROL_GREEN
  (`tools/verification/mutations.json`; 각 entry는 named test와 named assertion
  message로 kill된다). `just loom` 5/5, `just shuttle` 6/6 — production `Governor`.
- `just tsan` CLEAN (engine 3 + host 10 test binaries + the rayon adapter, macOS aarch64), `just coverage-report`
  REPORTED (수치는 receipt). `tests/differential_model.rs`: admission/promotion 상태기계를 ~150줄
  실행 가능한 명세와 무작위 op 시퀀스(proptest, 매 op 뒤 verdict·gauge·ticket·ledger 동치)로 대조 —
  engine mutation(release가 promote하지 않음, queued abandon이 promote하지 않음 — 이건 이 test만 잡는다 —,
  LIFO promotion, inflight 경계 off-by-one, claim이 ticket을 남김, terminal record 유실, 잘못된 QueueFull
  verdict, head의 pool 무시 등)을 최소 반례로 잡는다; inventory entry `abandoning-a-queued-ticket-never-promotes`가
  그중 하나를 고정한다. D08 gap 규칙은 단일 스레드(waker 재진입 제외)에서 도달 불가하므로 shuttle 모델이 담당.
  범위 밖(명시): memory mode, DRR/WFQ, waker/wake 경로, child scope·recursion guard, stale-lease reap,
  retry-after, best-effort/deadline tier, budget 경계(≤12 queued). verdict 우선순위(Inflight → Capability →
  Cpu → Memory)와 "queue가 꽉 찼을 때는 막힌 자원의 verdict"는 library spec에 표로 적었고 모델은 그 표를 따른다.
- 이 ADR은 **local** 검증 상태를 기술한다. hosted CI, Linux instruction-count
  qualification, 배포/activation은 여기서 주장하지 않는다. consumer MSRV는 local
  1.81 toolchain에서 default+rayon PASS다.
