# SEP-21 structural remediation tickets

## 기준과 판정

- 기준일: 2026-09-21
- source: branch `hardening/sep-16`, HEAD
  `1f04ccb245fc631507df780b97768f8d2f0e5e1a`, tree
  `65152e1f97e5b691df7ffe946c43a25bc7c61ee5`
- 입력: [`../findings.md`](../findings.md)의 확정 finding 23건
- 재현성: `plan.json`이 HEAD/tree뿐 아니라 tracked dirty diff, untracked audit 입력,
  체크리스트와 finding SHA-256을 고정한다. `docs/bugbash/sep-21/`만 audit 산출물로 제외한다.
- 상태: **PLANNED / implementation NOT_STARTED / release NO-GO**
- 목표: finding별 국소 패치가 아니라 authority와 불변식 단위로 원인을 제거한다.
- 비목표: 이 문서 작성, focused test 또는 ticket 체크만으로 qualification을 주장하지 않는다.

## 공통 RCA

23건은 다음 여섯 구조적 원인으로 수렴한다.

1. **결정 authority가 여러 projection으로 분열됐다.** admission은 첫 blocker 하나,
   cycle detector는 root-scoped permit 일부, scheduler는 queue head, timeout은 intake snapshot을
   각각 진실로 사용한다. 동일 요청에 대해 서로 다른 결론이 나온다.
2. **fallible boundary가 state commit 양쪽에 흩어졌다.** validation, cancel/deadline 확인,
   external drop/wake, executor submit이 하나의 명시적 transaction에 속하지 않아 commit 뒤
   복구 불가능한 gap이 생긴다.
3. **semantic capacity와 physical executor capacity의 owner가 다르다.** capability metadata는
   선언일 뿐 실제 submit/pool concurrency를 강제하지 않는다.
4. **monotonic identity가 소진 상태를 표현하지 못한다.** memory epoch가 wrapping integer라
   terminal value 이후 stale 판정이 영구 고착된다.
5. **public contract가 validated domain을 표현하지 못한다.** product-specific provenance,
   중복 stage, 불완전 parent lineage, panicking constructor가 raw public surface에 남아 있다.
6. **proof가 artifact 대신 proxy marker를 신뢰한다.** stamp, 출력 문자열, aggregate count,
   mutable action tag가 실제 비교·실행·도구 identity를 대신한다.

## 목표 구조

- contract는 `ValidatedTaskPlan`, product-neutral provenance, explicit parent-wait identity를
  소유한다.
- engine은 한 번 계산한 `CapacityAssessment`를 admission, promotion, cycle 판정, timeout
  diagnostics가 공유한다.
- host는 `preflight -> admit/claim -> dispatch commit -> caller wait` transaction을 사용한다.
- external effect와 executor submit은 state lock 밖에서 실행하되, 실패/Drop panic의 compensating
  transition이 typed하고 exactly-once다.
- physical executor capacity는 build-time validated authority 하나만 가진다.
- proof gate는 source/tool/config/artifact/result를 digest로 묶고 anti-vacuity witness가 없으면
  PASS를 만들지 않는다.

## 티켓 목록

| ID | 목적 | 포함 finding | 선행 | write lane |
| --- | --- | --- | --- | --- |
| [SEP21-C01](SEP21-C01-task-plan-contract.md) | Task plan·provenance·lineage contract 정상화 | 010 | — | contract-task |
| [SEP21-E01](SEP21-E01-capability-authority.md) | fail-closed registered capability authority | 008 | — | engine-core |
| [SEP21-E02](SEP21-E02-effect-custody.md) | external effect panic과 state commit 원자성 | 004 | — | engine-core |
| [SEP21-E03](SEP21-E03-memory-epoch.md) | non-wrapping memory measurement sequence | 016 | — | engine-core |
| [SEP21-E04](SEP21-E04-pending-resolver.md) | validated plan·blocker set·wait graph·eligible promotion 단일 resolver | 001, 002, 009, 023 | C01,E01,E02 | engine-core |
| [SEP21-H01](SEP21-H01-validated-dispatch-plan.md) | stack/substrate dispatch preflight를 admission 앞으로 이동 | 007 | C01 | host |
| [SEP21-H02](SEP21-H02-acquisition-arbiter.md) | cancel/deadline/admission boundary와 timeout cause 단일 선형화 | 005, 006, 019 | H03 | host |
| [SEP21-H03](SEP21-H03-executor-domain.md) | physical executor capacity와 fallible submit authority | 003, 015, 020 | E01,E04,H01 | host |
| [SEP21-V01](SEP21-V01-trusted-evidence-envelope.md) | IAI artifact, CI trust root, tool provenance 통합 | 011, 014, 022 | — | proof-envelope |
| [SEP21-V02](SEP21-V02-mutation-evidence.md) | isolated mutation campaign과 exact outcome truth | 012, 017 | V01 | mutation-proof |
| [SEP21-V03](SEP21-V03-fuzz-model-evidence.md) | fuzz anti-vacuity와 model replay evidence | 018, 021 | V01 | concurrency-proof |
| [SEP21-R01](SEP21-R01-release-qualification.md) | semver 및 exact-source release closure | 013 | C01,E01,E02,E03,E04,H01,H02,H03,V01,V02,V03 | release |

`plan.json`이 finding 23건의 exact-once mapping과 dependency/write-lane metadata를 제공한다.
`validate_plan.py`는 source fingerprint, 122개 `path:line` 근거 범위, acceptance ID,
lane serialization과 dependency cycle까지 fail-closed 검증한다.
실제 agent dispatch에는 [`../prompts/README.md`](../prompts/README.md)의 7개 execution packet과
orchestrator prompt를 사용한다. 12개 ticket이 packet에 정확히 한 번 매핑된다.

## 상태 전이와 closure evidence

각 ticket은 다음 상태만 사용한다.

```text
PLANNED -> IN_PROGRESS -> IMPLEMENTED_UNQUALIFIED -> LOCALLY_VERIFIED
R01 only: LOCALLY_VERIFIED -> HOSTED_QUALIFIED
```

- `IMPLEMENTED_UNQUALIFIED`: source와 deterministic regression은 작성됐지만 full proof 미완료.
- `LOCALLY_VERIFIED`: ticket DoD와 named negative fixture가 frozen local source에서 통과.
- `HOSTED_QUALIFIED`: R01 release receipt만 사용할 수 있다. 개별 ticket은 이 상태를 발급하지 않는다.
- source/contract가 이후 바뀌면 관련 ticket은 `IN_PROGRESS`로 되돌리고 evidence를 폐기한다.
- 상태 변경에는 HEAD/tree, dirty digest, 변경 파일, 실행 명령, exit code, selected/executed count,
  artifact digest, 미실행 항목을 함께 기록한다.

## patch-on-patch 방지 규칙

1. **같은 lane은 한 writer가 최종 API까지 소유한다.** `engine-core`와 `host`는 각각 한
   integration branch/worktree에서 수행한다. 중간 상태를 main에 merge한 뒤 다음 티켓이
   되돌려 고치는 방식은 금지한다.
2. **C01이 task-plan contract shape를 먼저 고정한다.** E04/H01은 임시 field, string shim,
   old/new dual representation을 만들지 않고 C01의 최종 타입만 소비한다. capability contract는
   독립적인 E01 owner가 고정한다.
3. **host lane은 H01 → H03 → H02 순서로 한 번 통합한다.** H01이 final preflight type을,
   H03이 physical requirement/dispatch domain을 고정한 뒤 H02가 cancellation과 permit handoff를
   포함한 최종 acquisition transaction을 완성한다. 중간 adapter를 main에 따로 merge하지 않는다.
4. **proof lane은 production semantic을 바꾸지 않는다.** verifier를 통과시키기 위한 production
   branch, sleep, output marker 추가를 금지한다.
5. **R01은 선행 ticket을 수정하지 않는다.** 빠진 acceptance가 발견되면 원 owner ticket을
   reopen하고 새 receipt를 만든다.
6. 임시 compatibility shim은 제거 commit을 별도로 약속하지 않는다. 필요한 migration은
   C01/H01/H03 안에서 before/after fixture와 함께 완결한다.
7. **shared 문서는 R01 단일 owner다.** product ticket은 closure evidence에 documentation delta와
   before/after를 남기고 README, CHANGELOG, external interface, library spec, ADR을 병렬 수정하지
   않는다. source doc comment와 machine-readable runtime inventory는 해당 code lane이 소유한다.

## 실행 wave와 안전한 병렬성

| wave | 병렬 실행 가능 | 직렬/통합 제약 |
| --- | --- | --- |
| W0 | C01, V01, engine-core(E01→E02→E03) | 세 lane은 병렬, engine-core 내부는 한 writer가 직렬 |
| W1 | E04, H01, V02, V03 | E04는 C01/E01/E02, H01은 C01, proof producers는 V01을 소비 |
| W2 | H03→H02 | host 한 writer가 physical domain 뒤 final acquisition transaction 통합 |
| W3 | R01 | 모든 선행 source와 proof tool이 frozen 뒤 실행 |

병렬 worker는 시작 전에 HEAD, dirty paths, 담당 write lane을 다시 기록한다. 다른 lane의 파일이
필요하면 직접 수정하지 않고 owner에게 interface 요청을 보낸다.

## 배타적 write ownership

| lane | exclusive paths/symbols | 병렬 작업 규칙 |
| --- | --- | --- |
| contract-task | `taskmesh-contract/src/task.rs`, `validation.rs`, contract-crate provenance/lineage export | C01만 수정. `taskmesh` facade export는 H03이 통합 |
| engine-core | `engine/state.rs`, `engine/governor.rs`, admission/composite/fairness/memory features | E01–E04 한 writer. 다른 lane은 interface 요청만 가능 |
| host | `runtime.rs`, `execution_plan.rs`, `builder.rs`, executor adapters, `taskmesh-rayon`, executor 관련 contract 파일, `taskmesh` facade | H01→H03→H02 한 writer가 최종 transaction으로 통합 |
| proof-envelope | `receipt.py`, gate runner/inventory 공통 schema, workflows의 공통 permissions/action pins | V01 owner가 schema와 CI trust root를 먼저 고정 |
| mutation-proof | `tools/verification/`, mutation subreceipts | V01 schema 소비. Justfile/inventory/workflow/receipt 직접 수정 금지 |
| concurrency-proof | `tools/fuzz/`, `fuzz/`, Loom/Shuttle runner와 model artifacts | V01 schema 소비. shared orchestration과 production semantic 수정 금지 |
| release | release required set, semver runner, release receipt/checklist, README, CHANGELOG, external interface, library spec, ADR | R01만 수정. 선행 ticket source 수정 금지 |

`runtime.rs`, `governor.rs`, workflow처럼 물리적으로 공유되는 파일은 lane 밖 병렬 편집을
허용하지 않는다. 별도 worktree에서 작업하더라도 merge 전에 owner가 한 번에 통합하고,
임시 adapter/shim을 main에 먼저 넣지 않는다.

`plan.json`의 `lane_serialization`은 같은 lane의 실행 순서다. `depends_on`은 cross-lane semantic
선행 조건이고, 둘을 합친 그래프가 cycle-free여야 한다. C01/E01/E02는 다른 crate/lane에서
동시에 시작할 수 있지만 E04는 세 contract가 고정되기 전 시작하지 않는다.

V02/V03은 producer와 producer-owned manifest만 구현한다. `Justfile`, gate inventory,
qualification receipt, CI workflow 등록은 V01 owner가 두 producer contract가 고정된 뒤 한 번에
통합한다. R01은 모든 proof lane이 frozen된 뒤 release 전용 파일을 추가한다.

## campaign 공통 DoD

- `findings.md` 23건이 `plan.json`과 ticket Markdown에 정확히 한 번씩 매핑된다.
- public breaking change는 CHANGELOG, facade compile fixture, MSRV default/rayon consumer,
  `cargo semver-checks` human adjudication을 갖는다.
- 각 product ticket은 최소 하나의 deterministic regression과 intentional-defect/negative
  oracle을 가진다.
- panic/cancel/deadline/reject path 뒤 ticket, permit, phase, class, CPU, memory, capability,
  root attribution이 quiescent snapshot에서 보존된다.
- full workspace, feature matrix, Loom/Shuttle/TSan, isolated mutation, non-vacuous fuzz,
  performance comparison을 하나의 frozen clean source에서 실행한다.
- receipt는 exact source/tool/config/action/artifact identity와 selected/executed counts를
  포함한다. required NOT_RUN/SKIPPED/TIMEOUT은 `NOT_QUALIFIED`다.
- hosted qualification, review/merge, consumer qualification, activation은 서로 다른 상태로
  보고한다.

## 계획 자체 검증

```sh
# campaign 시작 전 baseline/source까지 strict 검증
uv run python docs/bugbash/sep-21/tickets/validate_plan.py

# production edit 시작 후 구조/status/mapping만 검증; source qualification이 아님
uv run python docs/bugbash/sep-21/tickets/validate_plan.py --structure-only
git diff --check
```

기본 모드는 frozen audit baseline과 현재 working tree가 같은지까지 확인한다. `--structure-only`는
의도적으로 source가 바뀐 뒤 mapping/lane/status/link만 검사하며 source 또는 release qualification
증거가 아니다.
