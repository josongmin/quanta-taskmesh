# 목표 구조와 불변식

## 경계

```text
Raw policy + topology + request
    -> validate / resolve immutable ExecutionPlan
    -> nonblocking intake credit (first-poll ownership)
    -> Pending -- semantic AND physical eligibility --> DispatchReserved
    -> executor Accepted -> Running -> Cleanup -> Terminal
          |                    |           |
          +------- caller response: Waiting / Delivered / TimedOut / Cancelled

Kernel: one narrow state lock, one transition authority
Effects: executor calls / wake / destructor / telemetry outside lock
```

새 actor framework, lock-free rewrite, engine별 pool, 자동 DAG 엔진을 추가하지 않는다. 현재 mutex 기반 상태를 좁히고 순수 transition/reference model과 out-lock effects 경계를 만든다. 위 상태명은 제안된 개념이며 현재 타입명이 아니다.

## 1. 소유권과 boundedness

- admission ownership은 future 생성이 아닌 first poll의 nonblocking credit 획득부터 시작한다. credit 앞에 무제한 semaphore waiter나 task spawn을 두지 않는다. unpolled future와 caller가 보관하는 payload는 caller 소유다.
- 요청의 소유 phase는 Pending/DispatchReserved/Accepted/Running/Cleanup 중 정확히 하나다. accepted-but-not-started를 pending에서 숨기지 않는다. ResultHeld와 Terminal metadata도 독립 retention bound를 갖는다.
- 전체 내부 request cell 수는 승인된 global outstanding bound 이하이며 class/capability별 bound도 적용한다. PendingDepth는 전체 outstanding 수와 다르다. shared pool capacity와 요청 분류 quota는 별개다.
- metadata bytes, return-result retention, cancellation tombstone, effect batch/diagnostic buffer에 상한과 초과 동작이 있다. preallocated per-request return slot 또는 동등한 방식으로 control completion 전달 용량을 보장한다.
- opaque closure가 이미 들고 있는 heap bytes를 추측하지 않는다. tracked reservation/declared payload bound와 실제 allocator hard cap은 다른 보장이다.
- strict runtime-owned lease는 runtime cleanup 증거만 release 가능하다. direct Governor manual permit·sweep API가 이 권한을 우회하지 못한다.

## 2. 단일 admission/scheduling 권위

ValidatedPolicy는 unknown class를 거절하고 canonical inventory completeness를 검증한다. ValidatedTopology는 min/max·플랫폼 한계·semaphore 최대 count를 construction 전에 검사한다. ExecutionPlan은 declared class, resolved resource class, semantic policy/control identity, capability, memory/stack requirements, config epoch를 고정한다.

Fairness가 고르는 후보는 semantic quota와 physical capacity를 모두 만족해야 한다. engine에서 먼저 승리한 요청이 substrate에서 무제한 기다리는 두 번째 scheduling queue를 만들지 않는다. 기본 제안은 class 내부 strict FIFO와 명시적 HOL tradeoff다. request별 capability로 우회할지 여부는 D08에서 결정한다.

DispatchReserved의 capacity와 adapter reservation은 같은 authority에 연결한다. `try_reserve -> submit/accepted -> started -> terminated` handshake에 request generation과 start authorization을 포함한다. cancel/shutdown 뒤 지연 도착한 effect는 start 전에 검증한다. accept 전 실패만 안전한 환급/재시도 후보이며 accepted-then-panic/unknown outcome은 quarantine하고 중복 실행하지 않는다.

여러 Runtime이 하나의 외부 executor를 공유하면 동일 token authority를 공유한다. managed pool 밖의 ambient Tokio/Rayon work를 taskmesh가 제한한다고 주장하지 않는다. legacy adapter가 capacity/start/termination을 증명하지 못하면 strict 거절 또는 승인된 conservative mode가 필요하다.

## 3. lock/effect와 공정성

- state transition 안에서는 bounded 순수 상태 변경만 수행한다. 외부 executor, 사용자 Clock, Waker clone/drop/wake, Arc 최종 destructor, telemetry callback은 밖에서 실행한다.
- transition이 생성한 stale effect의 generation과 보상 규칙을 검증한다. wake/retry/control-return 유실로 pending 작업이 영구 정지하지 않도록 continuation ownership을 둔다.
- promotion 한 번당 작업 예산 K를 두고 runnable pending이 남으면 bounded continuation을 예약한다. queue의 길이만 제한하고 cost/quantum 비례 루프를 남기지 않는다.
- DRR inactive/drained credit reset과 blocked class의 credit 정책을 명시한다. 큰 cost는 수학적 round skipping으로 처리하되 reference와 선택 순서를 비교한다.
- WFQ는 supported weight/cost domain, precision, renormalization을 정의한다. zero increment와 cancellation phantom debt를 제거한다. debit은 D08의 dispatch commit 시점이며 pre-start rollback과 연결한다.

## 4. accounting·clock

Per-request bytes→units 변환부터 checked arithmetic을 사용한다. aggregate는 exact wide representation이고 snapshot wire의 축소 변환은 명시적 오류다. observed overage는 이미 존재하는 사용량이므로 report 거부 후 옛 count 유지로 숨기지 않고 debt/pressure로 반영한다.

독립 oracle이 `aggregate == sum(live owned reservations/observed charges)`를 같은 snapshot에서 확인한다. request root/class/pool views는 별도 정책 의미를 유지하며 서로 다른 단위의 숫자를 무조건 합산하지 않는다. released reservation, measurement epoch, stage delta sequence를 분리해 중복/역순 report가 자원을 부활시키지 못한다.

외부 Clock은 lock 밖에서 읽고 commit의 logical time은 monotonic하게 처리한다. lock 획득 전 sampling을 곧바로 '현재 commit time'이라고 부르지 않는다. custom clock reentry·backward value·wall time 차이를 명시한다. heartbeat stale는 Suspected 신호일 뿐 running task 종료 증거가 아니다.

## 5. deadline·cleanup

Acquire 시간에는 admission lock wait를 포함하고 claim/start 직전 만료를 확인한다. RunFor는 실제 worker start timestamp를 기준으로 한다. inline executor도 timestamp ownership이 같아야 한다. completion-vs-deadline tie와 ZERO 의미는 D09에서 확정한다. lock/OS scheduler 지연을 포함하는 hard real-time 응답을 보장한다고 주장하지 않는다.

Caller timeout/cancel과 worker 종료는 다른 사건이다. timeout response 뒤에도 실제 실행/child/runtime teardown이 끝나기 전까지 charge를 유지한다. nonabortable blocking work에 강제종료 deadline을 약속하지 않는다. owned-runtime teardown이 caller response를 가로막지 않도록 bounded cleanup ownership으로 넘기되, 영원히 끝나지 않는 작업은 계속 charged 상태로 남고 shutdown은 NotDrained를 반환한다.

Tokio는 시작한 spawn_blocking 작업을 abort할 수 없고 shutdown timeout도 작업 자체를 종료하지 않는다. [Tokio spawn_blocking 공식 문서](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html). 조회 문서는 1.53.1, repo lock은 1.52.3이므로 이 참조는 개념적 제약이며 locked 버전 실행 검증을 대체하지 않는다.

## 6. 중첩 실행 지원 한계

cap=1 parent가 같은 pool의 child를 await하거나 pool 간 wait cycle을 만들면 bounded queue만으로 deadlock을 해결하지 못한다. 선언된 dependency/capability cycle은 사전 reject하고, opaque user closure 내부의 임의 wait graph 감지는 보장하지 않는다. capacity 대여나 structured DAG scheduler는 별도 기능 제안으로 남긴다. borrowed IO/!Send local payload는 host executor에 남겨 중앙 metadata kernel이 Send+'static을 강제하지 않게 한다.

## 7. 검증 구조

Benchmark는 validated input → measured event population → typed metric → compatible baseline judgement를 분리한다. PM는 validate → exact render plan → read-only lint / approved apply로 분리한다. Gate inventory는 required 집합과 독립 대조하고 unknown/missing/skipped-required를 fail-closed 처리한다. 최종 qualification은 immutable source receipt로 묶되 merge/consumer/deploy/activation을 독립 단계로 둔다.

