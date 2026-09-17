# September 16 general audit — ticket index

- Source HEAD: `9ae9547216c70458f54f37368a67661321060886`.
- 감사 시작 시 clean main. 문서 및 standalone observation harness만 추가했다.
- 4개 병렬 audit lane 결과를 main이 소스/재현으로 통합하고 중복 원인의 증상은 합쳤다.
- 40 records: P1 4건, P2 29건, P3 7건. TM16-005는 CONTRACT_DECISION이며 확정 runtime bug 수에 넣지 않는다.
- 추가 감사에서 TM16-026–029를 등록했다. 기존 25건과 원인이 다른 lease timestamp 및 benchmark/replay 결함이다.
- 두 번째 추가 감사에서 TM16-030–033을 등록했다: port destructor deadlock, thread-name setup panic, synchronous admission timeout 누락 및 phantom verification command 안내.
- 세 번째 추가 감사에서 TM16-034–036을 등록했다: IAI input teardown 혼입, PM nested template identity 소실, Criterion iteration 분모 불일치. Linux instruction-count/timing qualification은 미실행이다.
- 네 번째 추가 감사에서 TM16-037–038을 등록했다: DRR idle credit 누적과 allocation gate의 malformed-number false green. 과거 hardening 후보였던 DRR은 public-engine 재현으로 승격했다.
- 다섯 번째 추가 감사에서 TM16-039–040을 등록했다: PM duplicate key로 인한 검증 대상 유실과 mixed-substrate soak의 전건 거부 false green. Burst zero-dwell hang은 기존 TM16-028의 validation 보완 범위에 합쳤다.
- 구현 보완은 아직 수행하지 않았다. 보존된 Rust observation/diagnostic은 26개(runtime 20 + benchmark 6)다. 이번에는 양쪽을 debug/release에서 검증하고 실제 PM duplicate-target lint를 실행했다. 이전 allocation/nested-template 및 workspace gate 실행은 각 감사 receipt와 구분한다. Green probe는 수정 완료 증거가 아니다.

## 우선 처리

1. TM16-010: leak sweep의 dead ticket claim / 무계상 실행.
2. TM16-001: bounded admission 우회, terminal reject 지연, idle slot 점유, host fairness 우회.
3. TM16-023: 실제 requested-stack worker cap 우회.
4. TM16-024: cooperative root deadline 응답이 blocking child cleanup에 묶임.
5. TM16-008/003/011/012/013/014: 자원·설정·메모리·fairness 경계값과 합성 동작.
6. TM16-006/007/016–021: 실패하는 정책 게이트 및 false-green 검증 경로.

## 전체 inventory

| ID | Priority | Problem | Evidence |
| --- | --- | --- | --- |
| [TM16-001](TM16-001-substrate-waiters-bypass-bounded-admission.md) | P1 | Substrate 대기열이 class queue/Reject 정책을 우회한다 | retained host probe + integration source |
| [TM16-002](TM16-002-blocking-runfor-deadline-is-inert.md) | P2 | Blocking 경로가 CooperativeWithDeadline의 RunFor를 집행하지 않는다 | retained host probe |
| [TM16-003](TM16-003-invalid-topology-panics.md) | P2 | Fallible Builder가 잘못된 topology에서 panic한다 | retained build-panic probe |
| [TM16-004](TM16-004-policyset-default-inventory-bypass.md) | P2 | PolicySet의 public Default/fields가 builtin inventory 권위를 우회한다 | retained engine probe |
| [TM16-005](TM16-005-memory-release-policy-not-enforced.md) | P3 / decision | Stage 조기 반환의 release policy 계약이 불분명하다 | retained observation / contract decision |
| [TM16-006](TM16-006-local-gates-not-enforced-by-ci.md) | P2 | CI와 just gate의 정책 집행 표면이 다르고 현재 local gate가 실패한다 | exact local commands + workflow source |
| [TM16-007](TM16-007-dependency-advisories.md) | P2 | Locked dependency supply-chain gate가 red다 | cargo-deny + RustSec |
| [TM16-008](TM16-008-resource-counter-saturation.md) | P2 | Resource counter saturation이 MAX budget의 초과 admission을 숨긴다 | retained engine numeric probe |
| [TM16-009](TM16-009-stage-activity-does-not-touch-leak-lease.md) | P2 | Stage memory activity가 leak lease를 갱신하지 않아 최근 작업을 stale로 회수한다 | retained clock probe |
| [TM16-010](TM16-010-leak-sweep-leaves-dead-claim.md) | P1 | Leak sweep 후 reclaimed permit의 ticket이 claim 가능하다 | retained debug/release engine probe |
| [TM16-011](TM16-011-estimated-reconcile-undoes-stage-release.md) | P2 | Estimated reconcile이 반환된 stage reservation을 다시 늘린다 | retained engine probe |
| [TM16-012](TM16-012-drr-unbounded-work-under-lock.md) | P2 | DRR 선택의 작업량이 cost/quantum에 비례해 mutex를 장시간 점유한다 | algorithm bound / small subagent probe |
| [TM16-013](TM16-013-wfq-zero-increment.md) | P2 | WFQ weight precision 소실로 weighted share가 FIFO로 퇴화한다 | retained weight-ratio probe |
| [TM16-014](TM16-014-wfq-cancelled-service-debt.md) | P2 | Abandoned WFQ 요청이 phantom service debt를 남긴다 | retained cancellation-debt probe |
| [TM16-015](TM16-015-requested-stack-completion-before-release.md) | P2 | Requested-stack 결과 반환에 lease release fence가 없다 | source interleaving / forced schedule pending |
| [TM16-016](TM16-016-architecture-checker-false-green.md) | P2 | Architecture checker가 미등록 crate/외부 의존성과 optionality를 누락한다 | main negative metadata diagnostic |
| [TM16-017](TM16-017-semgrep-test-enrollment-gap.md) | P2 | Semgrep test-quality 규칙이 실제 integration tests를 스캔하지 않는다 | actual verbose scanner enrollment |
| [TM16-018](TM16-018-benchmarks-skip-intended-work.md) | P2 | Allocation/IAI benchmark가 non-admission을 성공 비용처럼 측정한다 | source branch / forced gate pending |
| [TM16-019](TM16-019-benchmark-workflow-swallowed-failures.md) | P2 | Benchmark trend workflow가 pipeline/branch lookup 실패를 삼킨다 | shell diagnostic + workflow source |
| [TM16-020](TM16-020-iai-proof-config-and-baseline-mismatch.md) | P2 | Local IAI proof threshold와 CI baseline compatibility가 일치하지 않는다 | config/source / Linux execution pending |
| [TM16-021](TM16-021-developer-toolchain-floor.md) | P2 | Declared Rust floor와 locked developer proof graph가 호환되지 않는다 | Cargo 1.83 failure / manifest requirements |
| [TM16-022](TM16-022-cpu-relative-deadline-starts-after-spawn.md) | P2 | CpuExecutor::spawn 완료 뒤 상대 deadline을 시작해 늦은 결과를 성공 처리한다 | retained inline executor probe |
| [TM16-023](TM16-023-stack-dispatch-bypasses-capability.md) | P1 | Blocking stack request가 실제 dedicated worker의 large-stack cap을 우회한다 | retained actual worker concurrency probe |
| [TM16-024](TM16-024-requested-stack-shutdown-delays-deadline.md) | P1 | Requested-stack owned runtime teardown이 deadline 응답을 blocking child 종료까지 지연한다 | retained owned-runtime shutdown probe |
| [TM16-025](TM16-025-prompt-manager-real-target-drift.md) | P3 | Prompt-manager 실제 target drift | actual PM lint |
| [TM16-026](TM16-026-lease-timestamp-before-state-commit.md) | P2 | State commit 이전 clock capture가 신규 lease/최신 heartbeat를 조기 stale로 만든다 | retained controlled-interleaving probe |
| [TM16-027](TM16-027-invalid-trace-times-not-rejected.md) | P2 | Invalid trace time이 replay를 왜곡하고 debug overflow/release wrap을 만든다 | retained debug/release benchmark probe |
| [TM16-028](TM16-028-mmpp-crosses-phases-with-old-rate.md) | P2 | MMPP generator가 old-phase interval로 high-rate phase를 건너뛴다 | retained seeded rate probe |
| [TM16-029](TM16-029-open-loop-double-corrects-latency.md) | P2 | Omission 없는 open-loop latency를 backfill해 모집단을 중복 집계한다 | retained deterministic histogram probe |
| [TM16-030](TM16-030-waker-destructor-under-governor-lock.md) | P2 | Queued waker의 소멸자가 governor lock에서 재진입하여 deadlock한다 | retained isolated-process probe |
| [TM16-031](TM16-031-class-nul-panics-thread-construction.md) | P2 | Class의 NUL이 requested-stack thread construction을 panic시킨다 | retained host panic probe |
| [TM16-032](TM16-032-acquire-timeout-ignores-admission-lock-wait.md) | P2 | Synchronous admission lock wait가 acquire budget을 넘겨도 작업을 시작한다 | retained controlled-contention host probe |
| [TM16-033](TM16-033-test-quality-refers-to-missing-gates.md) | P3 | Test-quality 정책이 존재하지 않는 mutation/coverage gate를 안내한다 | actual recipe lookup + repository inventory |
| [TM16-034](TM16-034-iai-input-teardown-in-measurement.md) | P2 | IAI op 측정 구간에 input governor teardown이 들어온다 | source-extracted portable body probe + default entry-point source |
| [TM16-035](TM16-035-pm-template-path-collapses-to-basename.md) | P3 | PM이 nested template identity를 잃어 다른 template으로 lint green을 만든다 | retained hermetic actual CLI fixture |
| [TM16-036](TM16-036-contention-bench-iteration-denominator.md) | P3 | Contention batch 작업 수와 Criterion iteration 분모가 다르다 | source-guarded arithmetic + Criterion analysis source |
| [TM16-037](TM16-037-drr-idle-credit-accumulation.md) | P2 | DRR empty queue의 residual credit이 누적되어 이후 peer service를 지연한다 | retained deterministic busy-period reentry probe |
| [TM16-038](TM16-038-allocation-gate-malformed-number-passes.md) | P3 | Allocation gate가 malformed metric을 숫자 0/prefix로 처리하여 통과시킨다 | actual shell gate / controlled producer stdout |
| [TM16-039](TM16-039-pm-duplicate-target-key-hides-output.md) | P3 | PM duplicate YAML key가 output을 lint 대상에서 제거한다 | retained actual CLI fixture |
| [TM16-040](TM16-040-mixed-soak-discards-all-run-errors.md) | P2 | Mixed soak가 360건 전부 거부되어도 기존 assertion을 통과한다 | source-extracted disabled-policy mutation + unmodified control |

## 작업 문서

- [병렬 보완 계획](PARALLEL-REMEDIATION-PLAN.md): write set, 의존 순서, 통합 DoD.
- [코드 품질·중복·의도적 한계](QUALITY-AND-SCOPE.md): 확정 결함과 분리한 cleanup 및 계약 검토.
- [검증 receipt](evidence/verification.md): 실행 명령, 결과, 이전 증거의 범위.
- [보존된 observation harness](evidence/repro/src/lib.rs): 현재 동작의 재현; 수정용 regression assertion과 다름.
- [Benchmark observation harness](evidence/repro-bench/src/lib.rs): invalid trace, MMPP rate, histogram population의 재현.

## 기존 감사의 최종 조정

- plain Cooperative/PreSubmitOnly의 deadline ignoring은 class policy상 의도된 동작이다. 독립 fail-open bug로 세지 않는다.
- blocking RunFor는 TM16-002에서 옵션/문서 계약 불일치로 남겼다. class-policy ignoring과 unsupported substrate의 처리를 구분한다.
- OnTaskCompletion에서 explicit stage release 가능한 관찰은 trusted host hook 계약이 불분명하므로 TM16-005로 낮췄다.
- empty root/operation identity는 문서상 caller의 unique-root 책임과 함께 QUALITY-AND-SCOPE에 기록했다.
- known inert knobs(burst, DropBestEffort) 및 caller-owned stage/reduce/checkpoints를 누락 구현으로 단정하지 않았다.
- duplicate loom/default doctest/Rayon execution은 성능/maintainability backlog이며 기능 defect로 중복 세지 않았다.
- dependency inclusion은 application exploit proof가 아니다. proc-macro maintenance advisory는 bench/dev 범위다.

## 판정 범위

핵심 admission/lifecycle/accounting/host 구현과 테스트는 존재한다. 전체 workspace tests, baseline Clippy와 architecture front door는 통과하지만, 후자는 negative inventory fixture를 놓친다. strict Clippy/Semgrep/deny와 실제 PM lint는 실패한다. 일반적인 실행 성공과 bounded topology·deadline·complete registry·true proof-gate enforcement는 같은 결론이 아니다.

이 감사는 exhaustive formal proof, hosted CI/current branch protection, 실제 외부 소비자 통합/배포, Linux IAI qualification을 완료한 증거가 아니다. 최대 DRR configuration wall-clock 실행 및 requested-stack release-fence 강제 interleaving은 미실행이며 각 티켓에 표시했다.
