# Sep-16 structural hardening — 실행 계획

- 기준: 2026-09-16, HEAD `9ae9547216c70458f54f37368a67661321060886`, branch `main`.
- 최초 관찰: tracked 변경 없음, 기존 `docs/bugbash/` 전체 untracked. 기존 audit 산출물은 수정하지 않는다.
- 범위: 최종 정적 audit + 실행 계획 문서. production 코드/설정/의존성/실제 PM target 수정, commit/push/deploy 없음.
- 상태 (2026-09-21): **22개 티켓 IMPLEMENTED, branch `hardening/sep-16` (0.2.0)**; 4차(마무리)에서 예외 대장 결정 항목 2건(drain D17, nested wait D12)을 구현으로 닫았다. 원본 40개 finding 각각 corrected
  regression 연결, 품질 항목 33개 중 32개 처분 완료·1개 EXTERNAL(Q33). local(macOS, clean tree) qualification은
  historical schema-v1 receipt는 **NOT_QUALIFIED**다. Linux-only `bench-iai` 미실행 외에도 gate
  summary splice 불일치가 사후 발견되어 current validator가 거절한다. [H16-022](H16-022-qualification-and-rollout.md)와
  receipt(`../receipts/local-2026-09-19.json`)에 범위를 명시했다. 최종 qualification은 수정된 schema-v2
  collector를 commit한 뒤 CI `qualification` job(Linux clean checkout)이 새로 내야 한다.
- 구현된 계약: [ADR 0003](../../../adr/0003-sep-16-hardening-contracts.md). 로컬 증거:
  `just gate` green(baseline에서는 clippy/semgrep/deny red), `cargo test --workspace` 461 green (contract 42 · engine 207 · host 151 · rayon 3 · bench 55 · doc-examples 3),
  loom 5 / shuttle 7 green **on the production `Governor`**, curated mutation inventory
  102 defect probes + 1 CONTROL_GREEN(cargo runner 90·pytest runner 13; cargo-mutants score 아님),
  libFuzzer 3 target(`just fuzz`), consumer MSRV 1.81 PASS(default·rayon), allocation gate 3.0 allocs/op
  (threshold = 측정값).
- 구현 직후 3-track 적대적 감사(engine/runtime · 증명 강도 · tooling/CI/docs)를 실행했고 P0 3건·P1 12건·
  P2 다수를 모두 처리했다; 그 처리 위에 다시 2-track 감사(코드/주장 · 증명 표면)를 돌려 P1 4건·P2 8건을
  추가로 처리했다. breaking change가 승인된 뒤 3차로 release 0.2.0(semver 감사·CHANGELOG·실행되는
  migration fixture), TSan/coverage/CI qualification rail, `cargo mutants` 전수 sweep(생존자 전원
  triage, HIGH 갭 13건은 inventory entry), shuttle gap 모델·differential 명세, 소비자 시선·증명 표면
  적대 감사를 처리했다: [AUDIT-2026-09-16](../AUDIT-2026-09-16.md). 계약 변경은 ADR 0003
  D03/D05/D08/D09/D10 개정과 D13–D16.
- 원본 분류: P1 4 / P2 29 / P3 7. TM16-005는 계약 결정이며 독립적으로 확인된 runtime defect로 계산하지 않는다.
- 계획의 P0/P1/P2는 실행 순서다. 원본 severity를 상향한 것이 아니다.
- “SOTA++”는 정확한 계약·유한한 소유권·독립 검증을 목표로 한 설계안의 이름이다. 최신 업계 대비 우월성이나 성능 개선을 측정한 결과가 아니다.

## 현재 상태 — 2026-09-23

- 위 2026-09-21 상태와 실행 기록은 당시 snapshot의 증거다. hardening 구현 commit은 현재
  `origin/main@e55d3aa`의 조상이며, `hardening/sep-16` 전용 branch 상태는 아니다.
  `plan.json`은 21개 `IMPLEMENTED`와 H16-020의 `RETIRED`를 구분한다. H16-020 PM renderer는
  실제 생성 target이 0임을 확인해 2026-09-23 제거했다.
- `ci`, `bench-gates`, `release-qualification` GitHub workflow는 수동 비활성화되어 있다.
  현재 검증 경로는 [release checklist](../../../release-checklist.md)의 clean-source
  `just verify-macos-full`과 Linux `just qualify-local`이다. Linux `bench-iai`는 같은
  fingerprint의 baseline 생성 뒤 별도 qualified run이 필요하다.
- 2026-09-23 IAI owner-local 감사에서는 누락된 `Ir` 값, 일부 case만 실행된 비교,
  case별 raw `.out` 누락, symlink된 `target` 상위 경로를 재현하고 gate를 보강했다.
  fixture 테스트는 Linux Valgrind 실측과 clean-source qualification이 아니다.
- clean 후보 `86d8f4d`의 macOS required gate는 generated mutation 전까지 모두 PASS였으나,
  package-wide `cargo test` 변형 2건의 timeout으로 생성 mutation sweep을 중단했다.
  이 receipt는 `NOT_QUALIFIED`다. 같은 4개 변형의 nextest owner-local 실행은
  4/4 caught였고 생성 runner를 nextest로 변경했지만, 새 전체 receipt는 아직 없다.
- 2026-09-24 `main@95c1b6d`까지 test-oracle 정리 commit 3개가 추가됐다. 이전 clean
  후보의 receipt는 그 HEAD에 적용되지 않는다. 현재 owner-local Rust 35건과 Python
  50건, 실제 crate-boundary checker는 PASS지만 새 exact-source 전체 receipt는 없다.
- 보관된 2026-09-19 schema-v1 receipt는 `NOT_QUALIFIED`인 역사적 실행 증거다. 현재
  collector/validator의 receipt schema는 v4다. 현재 HEAD에 대한 ordinary qualification,
  외부 consumer 실행, deployment, activation 및 rollback owner 승인은 아직 이 문서에
  입증되지 않았다. Semantica의 추적된 optional path dependency는 발견했으나 frozen
  두 소스의 consumer rail은 미실행이다. [예외 대장](EXCEPTIONS.md)을 따른다.

## 읽는 순서

1. [최종 audit와 증거 한계](FINAL-AUDIT.md)
2. [목표 구조와 불변식](ARCHITECTURE.md), [승인이 필요한 12개 결정](DECISIONS.md)
3. [병렬 lane·배타적 파일 소유권·실행 순서](EXECUTION.md)
4. [40개 finding 및 33개 품질 항목 추적](COVERAGE.md)
5. [실행·negative proof·qualification 계약](VERIFICATION.md)
6. [예외 대장 — 미체크 acceptance 8건의 owner·사유·재검토 조건](EXCEPTIONS.md) (`validate_plan.py`가 티켓의 모든 `[ ]`이 이 대장에 있음을 강제; `just py-test`에 포함)

## 실행 티켓

| ID | 작업 | 역할 | 우선순위 | 선행 |
|---|---|---|---|---|
| [H16-001](H16-001-contracts-and-compatibility.md) | 계약·호환성 결정과 migration 경계 | I | P0 | — |
| [H16-002](H16-002-validated-policy-topology.md) | Validated policy·canonical inventory·topology 검증 | E | P0 | H16-001 |
| [H16-003](H16-003-terminal-ticket-lifecycle.md) | 단일 ticket lifecycle과 terminal claim | E | P0 | H16-002 |
| [H16-004](H16-004-exact-resource-accounting.md) | 정확한 resource accounting과 snapshot 폭 | E | P0 | H16-003 |
| [H16-005](H16-005-memory-epochs-and-lease-clock.md) | Memory reservation·measurement epoch·lease clock | E | P1 | H16-004 |
| [H16-006](H16-006-effects-and-bounded-promotion.md) | Lock 밖 effect retirement와 bounded promotion | E | P0 | H16-005 |
| [H16-007](H16-007-fairness-reference-and-credit.md) | Fairness reference·bounded cost·credit 정합성 | F | P1 | H16-006 |
| [H16-008](H16-008-resolved-execution-plan.md) | Resolved execution plan과 실제 capability 일치 | R | P0 | H16-002 |
| [H16-009](H16-009-bounded-intake.md) | 대기 이전 bounded intake와 pending accounting | R | P0 | H16-006, H16-008 |
| [H16-010](H16-010-dispatch-integration.md) | 단일 dispatch authority 통합 | I | P0 | H16-007, H16-009, H16-012 |
| [H16-011](H16-011-executor-protocol-and-setup.md) | Executor ownership protocol·worker setup 오류 | R | P1 | H16-009 |
| [H16-012](H16-012-deadlines-and-cleanup.md) | Deadline·completion fence·cleanup custody | R | P0 | H16-011 |
| [H16-013](H16-013-observability-and-invariants.md) | Authoritative snapshot·진단·보존식 | E | P1 | H16-010 |
| [H16-014](H16-014-production-regression-proof.md) | Production regression·mutation·concurrency proof | V | P1 | H16-013 |
| [H16-015](H16-015-benchmark-measurement.md) | Benchmark 실제 작업량·측정 경계·분모 | B | P1 | — |
| [H16-016](H16-016-workload-and-latency-model.md) | Validated workload·MMPP·raw latency | B | P1 | H16-015 |
| [H16-017](H16-017-performance-gates.md) | Metric schema·실패 전파·baseline qualification | B | P1 | H16-016, H16-019 |
| [H16-018](H16-018-gate-inventory-and-ci.md) | 검증 inventory·scanner enrollment·local/CI parity | C | P1 | H16-014, H16-017, H16-019, H16-020 |
| [H16-019](H16-019-dependency-and-toolchain.md) | Dependency advisory·consumer/dev toolchain 분리 | D | P1 | — |
| [H16-020](H16-020-pm-validated-render-plan.md) | PM validated render plan·safe apply | Q | P2 | H16-001 |
| [H16-021](H16-021-quality-and-doc-migration.md) | 중복·dead data·계약 문서 정리 | Q | P2 | H16-007, H16-012, H16-015, H16-016, H16-018, H16-020 |
| [H16-022](H16-022-qualification-and-rollout.md) | Exact-head qualification·rollout·rollback | I | P1 | H16-021 |

## 남은 작업 (2026-09-21 기록; 현재 상태는 위 절 참조)

- **commit/push는 하지 않았다.** working tree는 dirty이며 review·merge·hosted CI는 UNVERIFIED다.
- Linux에서 `just bench-iai`(instruction-count)를 실행해 QUALIFIED baseline을 만들어야 한다.
- breaking 변경 3건(`claim`·`release_stage_memory` 반환 타입, snapshot schema v2)의 외부 소비자 영향은
  UNKNOWN이다 — in-repo 소비자만 inventory했다.
- PM subsystem은 owner target이 0이라 제거했다. 생성 agent 문서가 필요해지면 owner source와 target
  계약을 먼저 정의한 뒤 새 gate로 재도입한다.
- 실제 assignee, consumer repo/SHA, activation owner는 여전히 미지정이다.

## 계획 자체 검증

```sh
python3 docs/plans/sep-16-hardening/tickets/validate_plan.py
```

[plan.json](plan.json)은 ID/dependency/source mapping의 기계 판독 inventory다. ticket Markdown은 구현 액션의 본문이다. 둘을 함께 변경해야 한다. validator PASS는 문서 무결성만 의미하며 production 테스트 PASS가 아니다.
