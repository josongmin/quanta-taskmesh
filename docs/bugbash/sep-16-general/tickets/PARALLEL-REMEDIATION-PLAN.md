# Parallel remediation plan

- Baseline: `9ae9547216c70458f54f37368a67661321060886`.
- 이 문서는 보완 작업 계획이다. 구현/commit/merge/activation 완료가 아니다.
- 원인이 같은 ingress 문제(TM16-001의 queue/validation/hoarding/fairness)는 한 owner가 설계한다.
- 동일 `runtime.rs`, `governor.rs`, `state.rs`, contract DTO, Cargo.lock을 여러 lane이 동시에 수정하지 않는다.

## 0. 공통 계약 / interface freeze

먼저 main integration owner가 다음을 짧은 design note + typed interfaces + regression expectations로 고정한다.

1. pending/queued/promoted/unclaimed/running/reclaimed의 ownership과 terminal ticket outcome.
2. worker slot을 실행하지 않는 semantic waiter가 보유하지 않으면서 ingress가 bounded되는 reservation 정책.
3. resource total의 exact numeric 폭 및 기존 public snapshot/API compatibility.
4. RunFor의 시작 시점, unsupported blocking 옵션 처리(TM16-002), 요청 결과 전달과 실제 child cleanup의 구분.
5. TM16-005의 trusted stage release policy 및 fallback class가 resource-only인지 full policy 변경인지.

계약 검토가 필요한 항목은 전체 안전 개선을 막지 않는다. TM16-010 ledger 정합성, checked arithmetic, topology panic 제거, 명백한 gate 실패 전파는 해당 계약에 종속되지 않는 부분부터 진행한다.

## 1. 병렬 실행 / disjoint write sets

| Lane | Tickets | Owned write set | 의존 / 제약 |
| --- | --- | --- | --- |
| E — engine ledger/lifecycle | 004,008,009,010,011,026,030 | engine/shared/mod.rs; engine/engine/governor.rs, state.rs; features/memory, inventory; 해당 engine tests | ledger/terminal-ticket 및 commit-time timestamp contract owner. port final-drop은 lock 밖에서 실행. contract snapshot 변경은 integration owner에게 요청 |
| R — runtime worker/control | 002,003,015,022,023,024,031,032,040 | taskmesh/src/runtime.rs,builder.rs; executor control; taskmesh runtime tests; contract/topology.rs; Rayon topology constructor/tests | 한 owner가 serial patches로 처리. 성공 completion fence와 pending cleanup의 lease를 혼동하지 않음. timed admission interface는 E와 고정. soak의 전건 거부를 검출하는 outcome/실제 실행 assertion 추가 |
| F — fairness | 012,013,014,037 | features/fairness/scheduler.rs,retry_after.rs; fairness/retry tests | E의 state fields 직접 수정 금지. empty-queue deficit reset hook과 class eligibility/counter interface를 freeze하고 사용 |
| G — structural validation | 016,017,033 | tools/arch; tools/semgrep/rules,tests; rule enrollment config | Q와 phantom command 주장을 정리. 신규 gate 채택 여부는 C에게 요청. strict Clippy production 코드 수정은 R에게 전달. shared Justfile/workflows 직접 수정 금지 |
| B — benchmarks | 018,019,020,027,028,029,034,036,038 | alloc probe, iai/criterion benchmark source; loadgen.rs,workload.rs; tools/bench-gate.sh,bench-iai.sh; bench tests | op/input-teardown 및 actual completed/iteration 단위를 고정. malformed metric은 parser에서 거부. .github/workflows/bench.yml patch는 C owner에게 전달. bench manifest dependency 변경은 D에게 요청 |
| D — dependency/toolchain | 007,021 | root/crate Cargo.toml dependency declarations; Cargo.lock; config/deny.toml; toolchain docs | Cargo.lock 유일 owner. iai package/runner 버전 변경 시 B/C와 exact version 맞춤 |
| Q — docs/PM cleanup | 005,025,035,039 및 QUALITY backlog | docs; tools/pm sources/templates/targets; pm.py/tests | duplicate YAML key는 dict 변환 시점에 거부. render template identity를 loader 기준으로 보존. AGENTS generated output 직접 overwrite 금지. canonical instructions 보존 여부 확인; gate recipe 수정은 C에 전달 |

C — CI integration owner는 TM16-006과 다른 lane의 recipe/workflow 변경을 받아 `Justfile`, `.github/workflows/ci.yml`, `bench.yml`을 유일하게 수정한다. D의 toolchain/dependency 결정과 B/G의 runnable front doors가 완료되면 통합한다. R과 E의 production lint error는 해당 source owner가 수정한다.

## 2. Ingress integration — TM16-001

R/E/F 기반 수정 이후 ingress integration owner가 semantic pending reservation과 worker selection을 연결한다.

- terminal class/spec validation을 physical wait 전에 수행.
- per-class + capability pool의 pending 상한 및 typed overload outcome을 고정.
- class scheduler가 worker eligibility를 보도록 연결하고 FIFO semaphore의 hidden fairness arbitration을 제거.
- cancel/deadline/drop의 reservation/permit/gate custody를 한 번만 해제.
- worker가 실제로 실행하지 않는 semantic queued request는 idle physical execution slot을 점유하지 않음.
- snapshot/diagnostics에서 모든 admitted/pending 범위를 명확히 드러냄.

이 단계는 `runtime.rs`와 engine admission/claim 양쪽을 변경할 수 있으므로 R/E/F의 겹치는 write set을 잠근 뒤 한 owner가 통합한다. 빠른 'gate를 admission 뒤로 옮기기'만으로 무제한 executor queue를 만드는 수정은 허용하지 않는다.

## 3. 검증 / closure

1. ticket마다 corrected behavior assertion을 production integration/unit tests에 추가한다. 보존된 observation harness의 green을 remediation green으로 사용하지 않는다.
2. engine: 독립 u64 accounting oracle, reclaim/claim/abandon 순열, Estimated release/reconcile 합성, clock-read/state-commit 역순 interleaving 및 monotonic activity, reentrant wake/Drop와 blocking port retirement.
3. fairness: 소규모 reference algorithm과 order/credit 비교, idle/reentry·last-pending abandon·capacity-blocked queue의 credit 차이, 큰 numeric boundary의 bounded operation count.
4. host: hold gate + bounded pending; runnable class progress; best-effort/primary; stack-hint dispatch; success fence; terminal cleanup 중 worker lifetime; thread-name input/setup failure; synchronous admission contention의 budget 초과와 job 미시작.
5. tools: unknown metadata/real test paths/Reject performance fixture/failing shell pipeline/duplicate PM target을 actual front door에서 반드시 red로 확인. benchmark는 invalid event input·zero/nonfinite dwell 거부, CTMC rate/occupancy oracle, raw histogram population도 검증. R의 soak 검증은 all-disabled mutation을 반드시 검출한다.
6. 동일 immutable source에서 `just gate`, default/Rayon matrix, rustdoc, loom/shuttle와 실제 production-transition tests를 검증한다. 독립 model의 green만으로 production transition을 qualify하지 않는다.
7. Linux IAI는 compatible baseline 및 explicit regression config를 사용한다. consumer MSRV와 developer floor를 별도 matrix로 check한다.
8. source digest/HEAD, feature set, exact command/result를 ticket에 receipt로 붙인다. source fix와 hosted required-check/release/runtime integration의 상태를 별도로 기록한다.

## 통합 위험

- pending gate와 permit을 따로 획득하는 구조에서 새 deadlock/누수/late cancellation이 생기기 쉽다.
- reclaim은 ticket mapping을 지우는 것뿐 아니라 awaiter terminal notification이 필요하다.
- deadline 결과를 빨리 전달하면서 live child가 남으면 lease는 cleanup owner가 보존해야 한다.
- wide totals/snapshot 타입 변경은 semver와 external consumer에 영향이 있다.
- WFQ cancellation debt와 numeric precision 수정은 서비스 순서를 바꾼다. 기존 test fixture의 문자열 순서를 답으로 삼지 말고 정해진 credit semantics와 비교한다.
- 새로운 gate를 CI에 연결하면 이미 존재하는 위반이 red가 된다. blanket allow/ignore 또는 skipped enrollment로 녹색화하지 않는다.
