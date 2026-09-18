# Sep-16 structural hardening — 실행 계획

- 기준: 2026-09-16, HEAD `9ae9547216c70458f54f37368a67661321060886`, branch `main`.
- 최초 관찰: tracked 변경 없음, 기존 `docs/bugbash/` 전체 untracked. 기존 audit 산출물은 수정하지 않는다.
- 범위: 최종 정적 audit + 실행 계획 문서. production 코드/설정/의존성/실제 PM target 수정, commit/push/deploy 없음.
- 상태 (2026-09-18): **22개 티켓 IMPLEMENTED, branch `hardening/sep-16`에 commit (0.2.0)**. 원본 40개 finding 각각 corrected
  regression 연결, 품질 항목 33개 중 32개 처분 완료·1개 EXTERNAL(Q33). local(macOS, clean tree) qualification은
  **NOT_QUALIFIED**이며 사유는 하나, Linux-only `bench-iai` 미실행(SKIPPED_PLATFORM). 사유는
  [H16-022](H16-022-qualification-and-rollout.md)와 receipt(`../receipts/local-2026-09-18.json`)에 명시되어 있다;
  최종 qualification은 CI `qualification` job(Linux clean checkout)이 낸다.
- 구현된 계약: [ADR 0003](../../../adr/0003-sep-16-hardening-contracts.md). 로컬 증거:
  `just gate` green(baseline에서는 clippy/semgrep/deny red), `cargo test --workspace` 415 green (contract 38 · engine 180 · host 138 · rayon 3 · bench 55 · doc-examples 1),
  loom 5 / shuttle 6 green **on the production `Governor`**, mutation gate 65/65
  (Python 도구 10건·shuttle 모델 2건·differential 모델 1건·객관 sweep 갭 13건 포함), consumer MSRV 1.81 PASS(default·rayon), allocation gate 3.0 allocs/op
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

## 읽는 순서

1. [최종 audit와 증거 한계](FINAL-AUDIT.md)
2. [목표 구조와 불변식](ARCHITECTURE.md), [승인이 필요한 12개 결정](DECISIONS.md)
3. [병렬 lane·배타적 파일 소유권·실행 순서](EXECUTION.md)
4. [40개 finding 및 33개 품질 항목 추적](COVERAGE.md)
5. [실행·negative proof·qualification 계약](VERIFICATION.md)
6. [예외 대장 — 미체크 acceptance 10건의 owner·사유·재검토 조건](EXCEPTIONS.md) (`validate_plan.py`가 티켓의 모든 `[ ]`이 이 대장에 있음을 강제; `just py-test`에 포함)

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

## 남은 작업 (구현 밖)

- **commit/push는 하지 않았다.** working tree는 dirty이며 review·merge·hosted CI는 UNVERIFIED다.
- Linux에서 `just bench-iai`(instruction-count)를 실행해 QUALIFIED baseline을 만들어야 한다.
- breaking 변경 3건(`claim`·`release_stage_memory` 반환 타입, snapshot schema v2)의 외부 소비자 영향은
  UNKNOWN이다 — in-repo 소비자만 inventory했다.
- `tools/pm`은 target을 소유하지 않는다(AGENTS.md는 사용자 소유). 생성 agent 문서를 원하면 owner가
  source를 작성하고 target을 추가한다.
- 실제 assignee, consumer repo/SHA, activation owner는 여전히 미지정이다.

## 계획 자체 검증

```sh
python3 docs/plans/sep-16-hardening/tickets/validate_plan.py
```

[plan.json](plan.json)은 ID/dependency/source mapping의 기계 판독 inventory다. ticket Markdown은 구현 액션의 본문이다. 둘을 함께 변경해야 한다. validator PASS는 문서 무결성만 의미하며 production 테스트 PASS가 아니다.

