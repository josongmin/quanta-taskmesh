# SEP21-H02 — Acquisition boundary arbiter

- 상태: LOCALLY_VERIFIED
- 우선순위: P1
- 포함 finding: TM21-005, TM21-006, TM21-019
- 선행: H03
- write lane: `host`

## 목적

cancel, relative acquire timeout, absolute completion deadline와 permit handoff의 우선순위 및
linearization point를 하나의 arbiter로 만든다.

## RCA

- cancel은 admission 전 한 번만 검사된다.
- relative timeout과 absolute deadline을 merged `Option<Instant>`로 축약한 뒤 ZERO special-case가
  둘 다 비활성화한다.
- immediate `Admitted`와 queued `Ready`가 서로 다른 post-acquire 검사를 수행한다.
- permit을 받았지만 execution lease로 넘기기 전 최종 판정 owner가 없다.
- timeout error가 E04 current assessment가 아니라 intake `blocked_on` snapshot을 사용한다.

## 확정 근거

- cancel precheck만 있는 acquisition entry: `crates/taskmesh/src/runtime.rs:139-150`.
- `Admitted`/`Ready` 뒤 deadline만 재검사:
  `crates/taskmesh/src/runtime.rs:164-179,220-236`.
- relative/absolute deadline merge: `crates/taskmesh/src/runtime.rs:311-329`.
- ZERO가 merged deadline 판정을 전부 suppress:
  `crates/taskmesh/src/runtime.rs:821-832`.
- stale timeout cause projection: `crates/taskmesh/src/runtime.rs:226-235,834-843`.

## 목표 구조와 불변식

- `AcquisitionArbiter`가 cancel token, absolute deadline, relative wait budget을 원형대로
  보존한다.
- 우선순위는 cancel → absolute CompleteBy → relative acquire timeout이며 equality는 expired다.
- `Duration::ZERO`는 relative wait만 try-once로 만들고 absolute deadline을 suppress하지 않는다.
- `Admitted`/`Ready` 뒤 정확히 한 finalize step이 execution lease handoff를 결정한다.
- handoff 거부는 `return_unstarted` exactly once, worker/closure/thread 0이다.
- timeout verdict는 E04 current pending status의 blocker set을 deterministic precedence로
  public error에 투영한다.

## 작업 플랜

1. `crates/taskmesh/src/executor/cancel.rs`
   - acquisition budget mode와 absolute deadline을 분리해 표현한다.
2. `crates/taskmesh/src/runtime.rs`
   - `acquire`, `await_promotion`, `acquire_execution_lease`,
     `acquisition_budget_expired`의 분산 logic을 단일 arbiter로 교체한다.
   - immediate admit와 claim-ready가 같은 finalize function을 통과하게 한다.
   - timeout 직전 E04 current pending status를 읽고 intake snapshot 사용을 제거한다.
   - E02 compensated terminal을 인식해 `TicketGuard`가 duplicate abandon/release하지 않게 한다.
3. `crates/taskmesh/src/execution_plan.rs`
   - deadline/cancel policy를 lossy merge 없이 plan에 보존한다.
4. public error precedence 표를 closure evidence로 발행한다. shared library spec, external
   interface, ADR, CHANGELOG 반영은 R01이 통합한다.

## 테스트 플랜

- `crates/taskmesh/src/runtime/claim_acquisition_tests.rs`
  - blocking `Clock`/barrier로 initial check 이후 cancel race를 immediate/Ready 각각 재현.
  - effect compensation 직후 cancel/drop에서 permit/ticket release가 정확히 한 번인지 검증.
- `crates/taskmesh/tests/runtime_cancel_timeout.rs`
  - exact verdict와 user closure side effect 0.
  - `Inflight→Capability`, `Capability→CPU`, `Memory→Runnable` blocker 전환.
- `hardening_deadline_custody.rs`
  - ZERO + expired CompleteBy, admission-lock overrun, thread factory 0.
- `cancellation_policy.rs`
  - equality/tie와 policy별 precedence.
- intentional mutants: post-admit cancel check 제거, ZERO가 absolute를 suppress.

## DoD

- `SEP21-H02-A01`: sleep 없는 barrier test가 immediate/Ready cancel race를 결정적으로 잡는다.
- `SEP21-H02-A02`: cancel/deadline/relative-timeout tie의 exact variant가 문서와 test에서 동일하다.
- `SEP21-H02-A03`: rejected handoff 뒤 admitted==terminated, gauge 0, drain 성공이다.
- `SEP21-H02-A04`: normal admitted path에서 permit을 두 번 return하지 않는다.
- `SEP21-H02-A05`: default/rayon host surface가 동일 arbiter를 사용한다.
- `SEP21-H02-A06`: timeout cause가 마지막 current blocker와 일치한다.

## 금지되는 임시방편

- 경로별 token check 추가, `yield_now`/sleep timing test.
- ZERO special-case를 merged deadline에 유지.
- 먼저 spawn한 뒤 cancel 판정.
- engine에 Tokio `CancellationToken` 주입.

## 검증 명령 후보

```sh
cargo test --locked -p taskmesh --lib claim_acquisition_tests
cargo test --locked -p taskmesh --test runtime_cancel_timeout --test hardening_deadline_custody --test cancellation_policy
```

## Closure evidence (2026-09-21)

- source: uncommitted `host` lane atop `main@0fb1874`
- linearization owner: `AcquisitionArbiter` retains the cancel token, absolute CompleteBy, and
  relative budget as distinct values. `TokioRuntime::finalize_acquired` is the only
  permit-to-execution handoff for immediate and promoted permits.
- precedence: cancel -> absolute deadline -> relative timeout; equality is expired. ZERO is a
  distinct `TryOnce` relative mode and cannot suppress CompleteBy.
- timeout diagnostics: `Governor::pending_block_reason` is read immediately before the last claim;
  no intake blocker snapshot is stored or projected.

| Acceptance | Local evidence |
| --- | --- |
| SEP21-H02-A01 | `cancel_between_immediate_admit_and_handoff_returns_unstarted_once_v1` and `cancel_beats_claim_of_promoted_permit_and_guard_releases_it_v1` deterministically cover immediate and Ready races without sleeps. |
| SEP21-H02-A02 | Arbiter unit fixtures pin three-way cancel precedence, absolute-vs-relative equality, and ZERO+expired CompleteBy; deadline/cancel integration tests assert exact public variants. |
| SEP21-H02-A03 | Both rejected-handoff fixtures prove closure/thread effect 0, inflight==terminated, capability usage 0, and reusable capacity. |
| SEP21-H02-A04 | Normal immediate/queued transfer and caller-abandon fixtures prove one lease owner and exactly one release; settled terminal/invalid tickets disarm `TicketGuard` instead of duplicate abandon. |
| SEP21-H02-A05 | Default and Rayon host surfaces call the same runtime arbiter/finalize implementation; both full matrices pass. |
| SEP21-H02-A06 | `zero_budget_projects_the_current_capability_blocker_without_running_work` proves current capability state maps to `SubstratePoolTimedOut`; class contention maps to `PermitAcquireTimedOut`. E04 transition fixtures prove the pending view itself recomputes blockers. |

Intentional negatives cover token firing after initial check, promoted-permit timeout, reclaimed and
invalid terminal tickets, acquisition instant overflow, ZERO+absolute deadline, three-way ties,
capability-only timeout cause, and duplicate guard cleanup.

Validation: taskmesh default 160/160 PASS; taskmesh+rayon 160/160 PASS; focused acquisition lib
14/14 and runtime cancel/timeout 9/9 PASS; default/rayon clippy `-D warnings` PASS; consumer MSRV
default+rayon PASS on Rust 1.81. Workspace/release qualification remains R01.
