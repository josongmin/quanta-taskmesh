# Finding / quality → 실행 작업 추적

## 원본 40개 — primary owner 정확히 하나

모든 원본 audit 파일은 그대로 보존한다. 아래는 문제를 중복 집계하지 않기 위한 primary ticket이며 supporting integration은 dependency DAG를 따른다. runtime finding은 H16-010/013/014 통합 proof, benchmark는 H16-017, gate는 H16-018, 전체는 H16-022에서 qualification한다. TM16-005는 계약 결정이다.

| 원본 finding | Primary | 통합 종료점 |
|---|---|---|
| [TM16-001](../../../bugbash/sep-16-general/tickets/TM16-001-substrate-waiters-bypass-bounded-admission.md) | [H16-009](H16-009-bounded-intake.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-002](../../../bugbash/sep-16-general/tickets/TM16-002-blocking-runfor-deadline-is-inert.md) | [H16-012](H16-012-deadlines-and-cleanup.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-003](../../../bugbash/sep-16-general/tickets/TM16-003-invalid-topology-panics.md) | [H16-002](H16-002-validated-policy-topology.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-004](../../../bugbash/sep-16-general/tickets/TM16-004-policyset-default-inventory-bypass.md) | [H16-002](H16-002-validated-policy-topology.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-005](../../../bugbash/sep-16-general/tickets/TM16-005-memory-release-policy-not-enforced.md) | [H16-005](H16-005-memory-epochs-and-lease-clock.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-006](../../../bugbash/sep-16-general/tickets/TM16-006-local-gates-not-enforced-by-ci.md) | [H16-018](H16-018-gate-inventory-and-ci.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-007](../../../bugbash/sep-16-general/tickets/TM16-007-dependency-advisories.md) | [H16-019](H16-019-dependency-and-toolchain.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-008](../../../bugbash/sep-16-general/tickets/TM16-008-resource-counter-saturation.md) | [H16-004](H16-004-exact-resource-accounting.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-009](../../../bugbash/sep-16-general/tickets/TM16-009-stage-activity-does-not-touch-leak-lease.md) | [H16-005](H16-005-memory-epochs-and-lease-clock.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-010](../../../bugbash/sep-16-general/tickets/TM16-010-leak-sweep-leaves-dead-claim.md) | [H16-003](H16-003-terminal-ticket-lifecycle.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-011](../../../bugbash/sep-16-general/tickets/TM16-011-estimated-reconcile-undoes-stage-release.md) | [H16-005](H16-005-memory-epochs-and-lease-clock.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-012](../../../bugbash/sep-16-general/tickets/TM16-012-drr-unbounded-work-under-lock.md) | [H16-007](H16-007-fairness-reference-and-credit.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-013](../../../bugbash/sep-16-general/tickets/TM16-013-wfq-zero-increment.md) | [H16-007](H16-007-fairness-reference-and-credit.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-014](../../../bugbash/sep-16-general/tickets/TM16-014-wfq-cancelled-service-debt.md) | [H16-007](H16-007-fairness-reference-and-credit.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-015](../../../bugbash/sep-16-general/tickets/TM16-015-requested-stack-completion-before-release.md) | [H16-012](H16-012-deadlines-and-cleanup.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-016](../../../bugbash/sep-16-general/tickets/TM16-016-architecture-checker-false-green.md) | [H16-018](H16-018-gate-inventory-and-ci.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-017](../../../bugbash/sep-16-general/tickets/TM16-017-semgrep-test-enrollment-gap.md) | [H16-018](H16-018-gate-inventory-and-ci.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-018](../../../bugbash/sep-16-general/tickets/TM16-018-benchmarks-skip-intended-work.md) | [H16-015](H16-015-benchmark-measurement.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-019](../../../bugbash/sep-16-general/tickets/TM16-019-benchmark-workflow-swallowed-failures.md) | [H16-017](H16-017-performance-gates.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-020](../../../bugbash/sep-16-general/tickets/TM16-020-iai-proof-config-and-baseline-mismatch.md) | [H16-017](H16-017-performance-gates.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-021](../../../bugbash/sep-16-general/tickets/TM16-021-developer-toolchain-floor.md) | [H16-019](H16-019-dependency-and-toolchain.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-022](../../../bugbash/sep-16-general/tickets/TM16-022-cpu-relative-deadline-starts-after-spawn.md) | [H16-012](H16-012-deadlines-and-cleanup.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-023](../../../bugbash/sep-16-general/tickets/TM16-023-stack-dispatch-bypasses-capability.md) | [H16-008](H16-008-resolved-execution-plan.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-024](../../../bugbash/sep-16-general/tickets/TM16-024-requested-stack-shutdown-delays-deadline.md) | [H16-012](H16-012-deadlines-and-cleanup.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-025](../../../bugbash/sep-16-general/tickets/TM16-025-prompt-manager-real-target-drift.md) | [H16-020](H16-020-pm-validated-render-plan.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-026](../../../bugbash/sep-16-general/tickets/TM16-026-lease-timestamp-before-state-commit.md) | [H16-005](H16-005-memory-epochs-and-lease-clock.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-027](../../../bugbash/sep-16-general/tickets/TM16-027-invalid-trace-times-not-rejected.md) | [H16-016](H16-016-workload-and-latency-model.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-028](../../../bugbash/sep-16-general/tickets/TM16-028-mmpp-crosses-phases-with-old-rate.md) | [H16-016](H16-016-workload-and-latency-model.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-029](../../../bugbash/sep-16-general/tickets/TM16-029-open-loop-double-corrects-latency.md) | [H16-016](H16-016-workload-and-latency-model.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-030](../../../bugbash/sep-16-general/tickets/TM16-030-waker-destructor-under-governor-lock.md) | [H16-006](H16-006-effects-and-bounded-promotion.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-031](../../../bugbash/sep-16-general/tickets/TM16-031-class-nul-panics-thread-construction.md) | [H16-011](H16-011-executor-protocol-and-setup.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-032](../../../bugbash/sep-16-general/tickets/TM16-032-acquire-timeout-ignores-admission-lock-wait.md) | [H16-012](H16-012-deadlines-and-cleanup.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-033](../../../bugbash/sep-16-general/tickets/TM16-033-test-quality-refers-to-missing-gates.md) | [H16-018](H16-018-gate-inventory-and-ci.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-034](../../../bugbash/sep-16-general/tickets/TM16-034-iai-input-teardown-in-measurement.md) | [H16-015](H16-015-benchmark-measurement.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-035](../../../bugbash/sep-16-general/tickets/TM16-035-pm-template-path-collapses-to-basename.md) | [H16-020](H16-020-pm-validated-render-plan.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-036](../../../bugbash/sep-16-general/tickets/TM16-036-contention-bench-iteration-denominator.md) | [H16-015](H16-015-benchmark-measurement.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-037](../../../bugbash/sep-16-general/tickets/TM16-037-drr-idle-credit-accumulation.md) | [H16-007](H16-007-fairness-reference-and-credit.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-038](../../../bugbash/sep-16-general/tickets/TM16-038-allocation-gate-malformed-number-passes.md) | [H16-017](H16-017-performance-gates.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-039](../../../bugbash/sep-16-general/tickets/TM16-039-pm-duplicate-target-key-hides-output.md) | [H16-020](H16-020-pm-validated-render-plan.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |
| [TM16-040](../../../bugbash/sep-16-general/tickets/TM16-040-mixed-soak-discards-all-run-errors.md) | [H16-014](H16-014-production-regression-proof.md) | [H16-022](H16-022-qualification-and-rollout.md) — primary acceptance 및 관련 통합 proof |

## 품질 항목 33개 — 결함 수와 별도

아래 disposition은 2026-09-16 구현 결과다 (굵은 글씨). 괄호 안은 원래 계획이다. 마지막 열은 source/test 링크 또는 남은 owner다. 원본 [품질·범위 메모](../../../bugbash/sep-16-general/tickets/QUALITY-AND-SCOPE.md)의 후보를 오류·의도된 동작·검증 부채로 분리했다. **EXTERNAL 1건(Q33)**만 미완료로 남는다.

| ID | 항목 | 담당 티켓 | disposition | 근거 / 남은 owner |
|---|---|---|---|---|
| Q01 | 진단 error context | [H16-011](H16-011-executor-protocol-and-setup.md), [H16-013](H16-013-observability-and-invariants.md) | **FIXED** (계획: FIX) | `GovernorError::TicketClaimTerminated{ticket,reason}` / `InvalidTicketClaim`; `PendingView.blocked_on`로 timeout 원인 구분; worker 오류는 task/panic/spawn/receiver-gone 별도 — `hardening_executor_protocol.rs` |
| Q02 | expected receiver drop/send failure | [H16-006](H16-006-effects-and-bounded-promotion.md), [H16-018](H16-018-gate-inventory-and-ci.md) | **CLASSIFIED** (계획: CLASSIFY) | receiver-gone send 실패는 정상 전달 결과로 명시(`_undelivered` binding + 주석). control-return 유실 경로 없음: control은 lock 아래 직접 transition — `runtime.rs run_detached_job` |
| Q03 | optional ambient runtime lookup | [H16-021](H16-021-quality-and-doc-migration.md) | **RETAINED** (계획: RETAIN) | `Handle::try_current().ok()`는 optional ambient lookup — `run_blocking_on_dedicated_thread` |
| Q04 | control lookup 중복 | [H16-008](H16-008-resolved-execution-plan.md) | **CONSOLIDATED** (계획: CONSOLIDATE) | `crates/taskmesh/src/execution_plan.rs` 단일 resolver; 5개 진입점의 중복 cancel/deadline lookup 제거 |
| Q05 | worker handoff 중복 | [H16-011](H16-011-executor-protocol-and-setup.md), [H16-012](H16-012-deadlines-and-cleanup.md) | **CONSOLIDATED** (계획: CONSOLIDATE) | `run_detached_job` 단일 worker wrapper (blocking pool / CPU port / dedicated thread); local affinity variant(`run_local`)는 caller future로 유지 |
| Q06 | RequestKey/죽은 pending fields | [H16-021](H16-021-quality-and-doc-migration.md) | **REMOVED** (계획: AUDIT_REMOVE) | `PendingRequest.class`는 유지(ticket index 조회에 필요), `request_key`/`enqueued_at_ms`/`leased_at_ms`/`original_estimate_units`는 `PendingView`/`PermitLedgerView`로 노출되어 dead가 아님; unused `policies` 파라미터 제거 |
| Q07 | built-in definition 중복 | [H16-002](H16-002-validated-policy-topology.md) | **CONSOLIDATED** (계획: CONSOLIDATE) | `inventory::builtin_records()` 단일 정의 + `Governor::validate_inventory`가 registry를 그것과 대조; `PolicySet::default`가 `new`에 위임 |
| Q08 | 중복 local/CI rail | [H16-018](H16-018-gate-inventory-and-ci.md) | **CONSOLIDATED** (계획: CONSOLIDATE) | ci.yml/bench.yml에서 loom 중복 제거; loom·shuttle 각 1개 job이 `just loom`/`just shuttle` 호출 |
| Q09 | scanner rule/metadata 권위 | [H16-018](H16-018-gate-inventory-and-ci.md) | **FIXED** (계획: FIX) | `check_crate_boundaries.py`: unknown member/외부 edge/optionality/read failure/missing dir 모두 fail — `tools/arch/tests/` |
| Q10 | PM branding/template drift | [H16-020](H16-020-pm-validated-render-plan.md), [H16-021](H16-021-quality-and-doc-migration.md) | **FIXED** (계획: FIX) | logq 잔재 sources/templates 제거; `targets.yaml`은 빈 집합을 명시적으로 선언 — `tools/pm/README.md` |
| Q11 | reserved burst/DropBestEffort | [H16-021](H16-021-quality-and-doc-migration.md) | **RETAINED** (계획: RETAIN_OR_DEFER) | `burst`·`DropBestEffort`는 reserved로 문서화 유지; 기능 구현 없음 (`policy.rs` doc) |
| Q12 | DAG/reduce/checkpoint 선언면 | [H16-001](H16-001-contracts-and-compatibility.md), [H16-021](H16-021-quality-and-doc-migration.md) | **RETAINED** (계획: RETAIN) | stages/reduce/checkpoint는 governance-validated declaration — `task.rs` doc, ADR 0003 |
| Q13 | recursion/root dedupe | [H16-001](H16-001-contracts-and-compatibility.md), [H16-021](H16-021-quality-and-doc-migration.md) | **RETAINED** (계획: RETAIN) | root dedupe 미구현; recursion guard는 `(root, stage)` 키 유지 |
| Q14 | 빈/Unicode class identity | [H16-001](H16-001-contracts-and-compatibility.md), [H16-002](H16-002-validated-policy-topology.md), [H16-021](H16-021-quality-and-doc-migration.md) | **DECIDED** (계획: DECIDE) | class identity는 그대로(빈/Unicode/NUL 허용); OS thread label만 sanitize — `requested_stack_thread_name_v1`, ADR 0003 |
| Q15 | fallback policy identity | [H16-008](H16-008-resolved-execution-plan.md) | **DECIDED** (계획: DECIDE) | D03 resource-only fallback 채택; controls는 선언 클래스 — `execution_plan.rs`, `admission/mod.rs` |
| Q16 | running stale reclaim | [H16-005](H16-005-memory-epochs-and-lease-clock.md), [H16-012](H16-012-deadlines-and-cleanup.md) | **FIXED** (계획: FIX) | sweep는 `DispatchReserved`만 회수, running은 `retained_active` — `hardening_memory_epochs.rs::a_running_lease_is_suspected_but_not_reclaimed` |
| Q17 | per-request measured soft accounting | [H16-005](H16-005-memory-epochs-and-lease-clock.md), [H16-021](H16-021-quality-and-doc-migration.md) | **CLARIFIED** (계획: CLARIFY) | `memory/mod.rs` 모듈 doc: 측정 memory는 soft accounting이며 allocator hard cap이 아님 |
| Q18 | shared external executor capacity | [H16-011](H16-011-executor-protocol-and-setup.md) | **FIXED** (계획: FIX_OR_REJECT) | `CpuExecutor::capabilities()` + `ExecutorCapabilities::legacy()`; 공유 pool은 각 runtime이 자기 제출만 제한 — `two_runtimes_sharing_an_executor_each_govern_their_own_submissions` |
| Q19 | retry hint zero | [H16-007](H16-007-fairness-reference-and-credit.md), [H16-021](H16-021-quality-and-doc-migration.md) | **CLARIFIED** (계획: CLARIFY) | retry hint는 backoff 힌트이며 admission 보장이 아님 — `retry_after.rs` doc 유지; scheduler domain과의 불일치 없음 |
| Q20 | monotonic clock | [H16-005](H16-005-memory-epochs-and-lease-clock.md) | **FIXED** (계획: FIX) | monotonic commit watermark (`GovernedState::commit_time`); Clock은 lock 밖 — `ports.rs` doc, `hardening_memory_epochs.rs` |
| Q21 | README queue 의미 | [H16-021](H16-021-quality-and-doc-migration.md) | **FIXED** (계획: FIX) | README §5: `queued`는 class queue, outstanding = `queued + inflight`; admission 앞 대기 공간 없음 |
| Q22 | root/queued exact invariants | [H16-003](H16-003-terminal-ticket-lifecycle.md), [H16-004](H16-004-exact-resource-accounting.md), [H16-013](H16-013-observability-and-invariants.md) | **FIXED** (계획: FIX) | `Snapshot::conservation_violation` + 독립 oracle — `hardening_exact_accounting.rs`, `hardening_snapshot_projection.rs` |
| Q23 | PM path containment | [H16-020](H16-020-pm-validated-render-plan.md) | **DECIDED** (계획: DECIDE) | D11: output은 repo root, template/section은 `tools/pm` 내부; 절대경로·`..`·symlink escape 거절 — `test_render_plan.py` |
| Q24 | fixed sleep fixture | [H16-014](H16-014-production-regression-proof.md), [H16-015](H16-015-benchmark-measurement.md) | **HARDENED** (계획: HARDEN) | 신규 host regression은 barrier/oneshot handshake + bounded drain 대기; 고정 sleep 단독 단언 없음 |
| Q25 | USL no peak/undefined tail | [H16-016](H16-016-workload-and-latency-model.md), [H16-021](H16-021-quality-and-doc-migration.md) | **FIXED** (계획: FIX) | `usl_probe.rs`: β≤0은 "no finite peak"로 보고, near-linear 자동 표기 제거 |
| Q26 | abandon-without-promote 후보 | [H16-007](H16-007-fairness-reference-and-credit.md) | **VERIFIED_RETAIN** (계획: VERIFY_RETAIN) | 독립 DRR reference와 순서 일치 — `drr_selection_matches_the_reference_for_mixed_quanta_and_costs`; abandon-without-promote는 결함 아님 |
| Q27 | custom waker panic | [H16-006](H16-006-effects-and-bounded-promotion.md) | **HARDENED** (계획: HARDEN) | `apply_effects`가 모든 waiter를 깨운 뒤 첫 panic을 `resume_unwind` — `every_waiter_in_one_pass_is_woken_even_if_an_earlier_one_panics`, mutation `panicking-waker-aborts-the-wake-loop` |
| Q28 | O(Q²) cancellation/batch scale | [H16-006](H16-006-effects-and-bounded-promotion.md), [H16-007](H16-007-fairness-reference-and-credit.md) | **MEASURE_BOUND** (계획: MEASURE_BOUND) | cancellation rebase는 O(queue) ≤ `max_queue_depth`; promotion은 `PROMOTION_BUDGET`/pass; DRR select O(classes) — 성능 수치 주장 없음 |
| Q29 | Loom/Shuttle model realism | [H16-014](H16-014-production-regression-proof.md) | **FIX_PROOF** (계획: FIX_PROOF) | 감사(A2-P0-1)에서 toy replica로 판정 → engine `src/sync.rs` seam으로 production `Governor`를 checker 위에서 실행: loom 5 (전수) / shuttle 4 (10k schedules). ADR 0003 D13 |
| Q30 | unknown serde fields | [H16-001](H16-001-contracts-and-compatibility.md), [H16-021](H16-021-quality-and-doc-migration.md) | **RETAINED** (계획: RETAIN_OR_DECIDE) | `deny_unknown_fields` 미도입 — 호환성 계약 없이 강제하지 않음 |
| Q31 | LocalSet affinity/non-Send | [H16-014](H16-014-production-regression-proof.md), [H16-021](H16-021-quality-and-doc-migration.md) | **RETAIN_TEST** (계획: RETAIN_TEST) | `run_local` non-Send future는 caller task 유지 — `hardening_dispatch_resolution.rs`, `runtime_local.rs` |
| Q32 | measurement qualification gaps | [H16-017](H16-017-performance-gates.md), [H16-022](H16-022-qualification-and-rollout.md) | **QUALIFY** (계획: QUALIFY) | Linux IAI baseline은 macOS에서 미실행 → receipt에 `SKIPPED_PLATFORM`/NOT_QUALIFIED로 기록; fault injection은 mutation gate 43건(42 defects + control) |
| Q33 | external activation evidence | [H16-022](H16-022-qualification-and-rollout.md) | **EXTERNAL** (계획: EXTERNAL) | 외부 consumer/activation 미확인 → UNVERIFIED |

## 누락 방지

- source finding 40개 ID는 파일 목록/sha256으로 freeze한다. original file 추가/삭제/변경 시 mapping을 명시적으로 재검토한다.
- primary owner 변경 시 plan.json 및 표·각 티켓 source links를 함께 갱신한다.
- 새 설계 위험은 acceptance/decision으로 먼저 관리한다. reachable source defect가 입증될 때만 별도 bug ticket으로 승격한다.
- H16-022는 dependency closure로 나머지 21개를 포함한다. validator가 cycle·미매핑·중복 primary·누락된 파일/acceptance를 검사한다.

