# Code quality, duplication, scope and decision backlog

Baseline: `9ae9547216c70458f54f37368a67661321060886`. 아래 항목은 티켓의 확정 동작 결함과 구분한 품질/계약 audit 결과다. 단순 style 취향은 blocker로 세지 않는다.

## 에러 처리

- task error는 `RunError::Task(E)`로 보존되고 governor/runtime failure와 분리된다. ordinary CPU/blocking worker panic은 typed governor error로 반환된다.
- requested-stack `let _ = tx.send(outcome)` 2건은 policy gate 위반(TM16-006)이다. receiver가 drop된 후 send 실패는 예상된 동작이고, 살아 있는 caller의 task error를 삼키는 재현은 없다.
- CPU의 `let _delivered = tx.send((outcome,lease))`도 결과를 검사하지 않지만 failed payload drop이 lease를 worker에서 회수한다. generic Semgrep regex가 named underscore binding을 검사하지 않는다는 이유만으로 이를 runtime error loss라고 하지 않는다.
- `Handle::try_current().ok()`는 requested-stack blocking의 optional ambient runtime context다. 현재 의미상 intentional optional lookup이며 blanket error swallowing으로 분류하지 않는다.
- architecture source read `except OSError: continue`는 실제 fail-open scanner 경로(TM16-016).
- benchmark `if let Admitted`의 non-admission 무시(TM16-018)와 `cargo bench | tee` exit loss/remote lookup error→absent(TM16-019)는 실제 verification failure swallowing.
- panic payload가 generic message로 축약되고 반환 error에 operation/class/permit context가 적어 운영 진단력이 제한된다 (`runtime.rs:110-130,516-518,598-625`). task error 보존은 유지하며 typed runtime error context/telemetry를 추가할 수 있다. panic catching을 모두 'swallowing'으로 세지 않는다.
- 추가 감사 (2): thread-name construction의 NUL panic은 worker 내부 catch 이전에 발생하여 typed error mapping을 벗어난다(TM16-031). queued waker의 destructor는 mutex 아래 호출되어 재진입 deadlock을 만든다(TM16-030). 단순 panic context 손실/custom waker panic 후보와 구분한 실제 재현 경로다.
- 추가 감사 (5): mixed-substrate soak의 `let _ = run_*.await`는 실제 outcome 검증을 누락한다(TM16-040). source-extracted fixture에서 360건 모두 ClassDisabled인 것을 추가 단언해도 원래 test assertions가 통과했다. JoinHandle 성공/zero accounting은 workload 성공이 아니다.

## 중복 / maintenance

| Surface | 사실 / 위험 | 보완 후보 |
| --- | --- | --- |
| Clock/deadline policy lookup | runtime.rs:405-440에서 CooperativeWithDeadline 판정을 두 번 수행 | resolved submission controls/capability를 한 번 계산; TM16-022/024 lifetime contract와 함께 정리 |
| host workers | ordinary blocking, CPU, requested-stack blocking/async마다 result/panic/lease handoff가 다름 | ownership state machine 공유; 각각 executor 의미는 유지. TM16-015/023/024 해결 전 blanket wrapper 합치기 금지 |
| queued RequestKey | PendingRequest의 request_key는 root_operation_id에서 새 String을 만들고 동일 root도 따로 저장; request_key field는 suppression된 dead code이고 실제 decision은 안 읽음 | unused audit field retire 또는 immutable root identity 공유. derive-count 테스트가 field의 business value를 증명하지 않음 |
| PendingRequest.class | per-class queue key와 class field가 중복이고 field는 dead-code suppressed | 진단 소비자가 필요하면 명시적으로 사용; 아니면 구조 축소. duplicate governor/config class maps는 immutable config snapshot 복제이므로 그 자체 defect는 아님 |
| workflow loom | ci.yml과 bench.yml 모두 same loom rail 실행 | required-check 이름/coverage를 보존하며 하나로 정리 |
| repeat doctest/Rayon | workspace test 후 taskmesh doctest/taskmesh-rayon default check 반복 | feature-specific check와 기본 중복을 분리하고 단일 proof matrix로 관리 |
| architecture policy | Python substring scanner와 Semgrep가 일부 같은 boundaries를 표현하지만 inventory 범위가 다름 | 공통 crate/dependency inventory + 각 도구의 actual negative enrollment. 둘 다 있다는 이유로 어느 하나를 무검증 제거하지 않음 |
| source instructions | PM scaffold는 logq 명칭과 다중 generated target 선언; 실제 target drift | TM16-025에서 owner/retirement 결정. AGENTS를 blind sync하지 않음 |
| gate policy comments | test-quality.yml이 없는 mutants-critical/cov-gate와 cheese detector section을 안내 | TM16-033에서 실제 채택 여부 또는 phantom coverage 주장 제거. 다른 gate의 green으로 대체하지 않음 |
| builtin inventory | BUILTIN_SUBSTRATES names와 builtin_records constructor가 같은 5개 이름을 별도 선언 | 현재 drift는 없음. production 정의를 단일화할 수 있으나 독립 expected inventory fixture는 제거하지 않음 |
| benchmark fixture lifetime | IAI setup을 제외해도 owned input의 destructor가 benchmark 함수 안에 존재 | TM16-034의 op-only/input-teardown 단위 분리. snapshot output cleanup 포함 여부도 명시 |
| PM template resolution | 존재를 검사한 template path와 loader가 읽는 basename path가 다름 | TM16-035. nested/custom target 입력 범위이며 기본 flat targets의 손상 증거는 없음 |

## 의도적 제한 / inert settings

- `WeightedFairQueue::burst`는 명시적 reserved no-op (`policy.rs:14-16`, fairness_weighted test). 현재 구현 누락 bug로 세지 않음.
- `DropBestEffort`는 현재 reject 동치가 명시됨 (`policy.rs:134-136`). distinct shedding을 제품이 요구한다면 별도 기능 work다.
- `stages`는 governance declaration이며 실행 graph가 아니다 (`task.rs:91-103`). runtime은 후속 stage closure를 자동 schedule하지 않는다.
- deterministic reduce policy는 shape/required-key declaration만 검증한다. 결과 정렬/merge/tie/error aggregation을 실행하거나 caller reducer의 determinism을 증명하지 않는다.
- checkpoint metadata는 보존/조회되고 hook 실행은 caller/host의 책임이다.
- occupancy recursion은 same root/parent_stage의 sibling breadth와 ancestor depth를 구분하지 못해 두 번째 admission을 거부한다 (`composite/mod.rs:65-75`). unique stage 사용/단일 fan-out declaration의 기존 scope와 일치한다.
- 같은 operation/root 중복 admission dedupe는 startup scope 밖이고 caller의 unique root ID를 전제한다 (`release-checklist.md:60-61`).

## 별도 contract decisions / hardening

1. **Empty identity:** TaskSpec::base의 operation/root는 빈 문자열, class/stage wrapper도 empty string을 허용한다. validate_shape는 이를 검사하지 않는다. 식별자 충돌에 대한 caller 책임을 명확히 하고 `root_operation_id`를 display operation과 분리하는 타입/검증을 검토한다. 현재 문서의 unique-root 전제 때문에 독립 runtime bug로 확정하지 않았다.
2. **Fallback policy split:** engine `admit_class`는 resource permit을 fallback class ledger로 기록하지만 host cancel/deadline은 original spec.class로 결정한다 (`admission/mod.rs:112-114`, `runtime.rs:405-440`). caller가 cheaper job을 실행하도록 바꾸는 callback도 없고 resolved class는 admission result에 없다. resource-only reclassification인지 full policy transition인지 고정하고 host에 resolved authority를 노출한다. same closure의 physical memory를 자동 줄이는 구현으로 설명하지 않는다.
3. **Live-worker leak reclaim:** LeakDetecting sweep은 stale permit을 강제로 reclaim하므로 실제 worker가 살아 있어도 semantic permit이 사라질 수 있다. physical ExecutionLease gate는 worker가 보존하지만 0/unlimited gate에서는 resource governance를 대신 못 한다. timeout은 termination proof가 아니므로 explicit opt-in sweep의 운영 전제와 heartbeat/termination custody를 문서화한다. stale forced reclaim 자체는 intentional이라 별도 확정 bug count에서 제외한다.
4. **Per-request measured limits:** per-request memory cap은 configured estimate를 build 때 검사한다. reconcile은 measured overage를 기록하고 신규 admission을 차단하며 기존 task를 kill하지 않는다. 실제 allocation hard cap을 제공한다고 설명하면 안 된다.
5. **CPU topology/executor alignment:** injected executor의 capacity/queue metadata를 runtime이 확인할 port는 없다. default topology gate는 ingress를 cap하지만 external pool과 실 worker 수가 동일함을 증명하지 않는다. shared executor를 여러 runtime instance가 사용하면 per-runtime gates는 global pool budget이 아니다.
6. **Numeric policy clarity:** weight/quantum 0은 scheduler에서 1로 normalize되고 retry heuristic은 원값을 읽는다. adaptive retry의 0ms 및 fairness relief는 service-time measurement가 아닌 deterministic heuristic이다. positive-parameter 조건/0ms 의미를 명시한다.
7. **Clock:** default Clock은 SystemTime wall millis이며 NTP/clock adjustment를 견디는 monotonic lease clock이 아니다 (`ports.rs:6-22`). staleness/DeadlineAware logical time의 operational assumption을 정리한다. 이번에는 clock-jump production reproduction을 하지 않았다.
8. **README queue samples:** max_queue_depth를 지정해도 default overflow Reject이면 queue하지 않는다. README의 구성 예시가 설정한 depth를 실제 queue promise로 읽히지 않도록 QueueWithinDepth를 보여주거나 inert 조건을 설명한다.
9. **Actual source invariants:** assert_consistent는 class/global/permits/granted subset만 검사하고 root attribution/active_recursion/queued ownership의 전체 equivalence를 검사하지 않는다. accounting bug를 catch하려면 exact wide oracle와 독립 ledger projection이 필요하다.
10. **PM output path boundary:** `pm.py:114-123`는 output을 repo_root와 join하고 write_text하므로 `../`/absolute path 및 output symlink를 따라갈 수 있다. agent의 임시 sentinel fixture에서 root 밖 write를 관측했고 main은 해당 source를 확인했다. 하지만 `--repo-root` help는 resolution base를 설명할 뿐 명시적인 sandbox/containment 계약이 없으므로 별도 확정 보안 결함으로 승격하지 않았다. repo-contained output만 지원할지 결정하고, 필요하면 resolved path/ancestor symlink 검사와 명시적 external-output opt-in을 설계한다. 실제 기본 target의 escape나 사용자 파일 손상을 주장하지 않는다.

## Scalability / proof limits

- 추가 감사 (3): TM16-034/036은 측정 경계·분모 결함이다. portable function-body Drop probe와 source-guarded integer diagnostic을 actual Linux instruction count/host scalability qualification으로 승격하지 않았다.
- governance_tax.rs 및 host_edge_paths.rs의 holder 준비는 fixed 50ms sleep에 의존한다. actual holder/gate readiness acknowledgement가 아니므로 느린 CI에서 timeout fixture가 아직 준비되지 않을 수 있다. started signal로 대체할 품질 후보이며 실제 slow-CI failure 강제 재현 없이 별도 confirmed runtime ticket으로 세지 않았다.
- USL의 finite peak 부재는 near-linear scaling과 동치가 아니다. beta=0, alpha>0에서는 peak 없이 saturation으로 수렴한다. `usl_probe.rs`의 near-linear 문구와 `metrics.rs`의 unbounded-scaling 표현을 계수/empirical curve 기준으로 정리할 수 있다. fit/predict 함수의 동일 수학 오류로 단정하지 않고 report semantics 후보로 남겼다.

- 추가 감사에서 timestamp read/commit 순서를 분리한 lease 결함(TM16-026), replay 입력의 invalid float/time 허용(TM16-027), MMPP phase scheduling 오류(TM16-028), raw/synthetic latency 모집단 혼동(TM16-029)을 별도 티켓으로 승격했다. request conservation과 CV/determinism 테스트가 이 의미적 오류를 검출하지 못한다.
- `metrics::tail_amplification(0, positive_p99)`는 1을 반환한다 (`metrics.rs:27-31`). 현재 headline path에서 호출되지 않으므로 독립 production blocker로 세지 않았다. undefined/infinite ratio와 flat tail을 같은 값으로 표시하지 않도록 Option/명시적 상태를 검토한다.
- `Governor::abandon`의 queued removal 후 promote 미호출도 재검토했다. 현재 유효 정책에서는 per-class queued cost가 동일하고 queue removal이 capacity를 늘리지 않는다. 실제 runnable head가 새로 생기는 재현 없이 별도 starvation ticket으로 등록하지 않았다.
- 추가 감사 (4): DRR drained class의 residual credit은 20회 empty/reentry 이후 A×10이 B보다 먼저 실행되는 deterministic 재현으로 TM16-037에 승격했다. 단순 standard algorithm 선호가 아니라 idle history가 peer service bound를 바꾸는 결함이다. custom PermitWaker panic 격리는 계속 contract/hardening 후보로 남긴다.
- 추가 감사 (4): allocation gate의 malformed 숫자 `...`/`3.0.0` 통과를 TM16-038에 기록했다. 정상 producer 출력이 실제로 깨졌다는 증거는 없으며 missing metric/producer failure는 현재도 거부한다. blanket shell error swallowing으로 확대하지 않는다.
- 추가 감사 (5): PM YAML duplicate key의 inventory 소실은 TM16-039로 기록했다. zero-dwell BurstConfig가 phase boundary를 진전시키지 못하는 격리 재현은 TM16-028에 합쳐 중복 집계를 피했다. Composite/reduce, recursion/root attribution, retry arithmetic, host cancellation/lease 계약을 재검토했지만 기존 티켓과 다른 runtime 결함은 추가 확정하지 못했다.
- 추가 감사 (2): synchronous admission mutex 대기가 budget을 넘겨도 immediate Admitted를 성공 처리하는 TM16-032를 재현했다. worker code 선점/실시간 응답 보장을 구현했다고 해석하지 않으며, 최소한 expired admission 이후 job을 시작하지 않는 경계를 요구한다.
- arbitrary identity의 empty/Unicode 자체와 OS thread label의 NUL 제약을 분리했다. serde unknown-field admission은 forward-compatibility 결정 없이 일괄 bug로 세지 않았다. `run_local`의 LocalSet은 non-Send lifetime으로 보존되므로 thread-affinity가 자동으로 깨진다는 후보도 등록하지 않았다.

- promote는 batch를 global mutex 안에서 실행한다. selection은 retained class 전체를 훑고 capacity lookup이 O(log C)다.
- abandon은 class queues를 순회/선형 검색/VecDeque removal하므로 큰 queue의 adverse cancellation order에서 합계 O(Q²)가 가능하다. class/inventory 수와 queue budget의 운영 범위가 필요하다. measured timing 없이 즉시 rewrite blocker로 올리지는 않았다.
- Loom/Shuttle tests는 parking_lot/std production engine 대신 별도 작은 model을 사용한다 (`loom_governance.rs:1-15,35`, `shuttle_governance.rs:26`). model green은 synchronization design 증거이며 실제 memory/reclaim/fairness/host 전이가 instrument된 formal proof는 아니다.
- 현재 focused/workspace greens가 TM16-010 leak-promoted ownership, numeric overflow, host physical ingress bound를 덮지 못했다.
- Linux instruction regression, actual consumer Rust 1.81, hosted required checks, deployment/runtime integration은 남은 qualification work다.
