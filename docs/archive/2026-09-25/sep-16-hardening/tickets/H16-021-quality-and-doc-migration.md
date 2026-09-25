# H16-021 — 중복·dead data·계약 문서 정리

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P2 (원본 bug severity 변경 아님)
- 책임 역할: Q — PM/문서 owner (실제 assignee 미지정)
- 선행 완료: [H16-007](H16-007-fairness-reference-and-credit.md), [H16-012](H16-012-deadlines-and-cleanup.md), [H16-015](H16-015-benchmark-measurement.md), [H16-016](H16-016-workload-and-latency-model.md), [H16-018](H16-018-gate-inventory-and-ci.md), [H16-020](H16-020-pm-validated-render-plan.md)
- 원본 finding: 통합·계약·품질 작업; 독립 신규 결함 수에 가산하지 않음
- 배타적 write lease: `engine`, `host`, `docs`; [적용 순서](EXECUTION.md) 준수

## 목적

구조 변경 뒤 중복 구현·죽은 데이터·오해하는 문서를 정리하고 의도된 확장면은 결함과 구분한다.

## 변경 범위

- 기존: [README.md](../../../../../README.md)
- 기존: [docs/taskmesh-library-spec.md](../../../../taskmesh-library-spec.md)
- 기존: [docs/taskmesh-external-interface.md](../../../../taskmesh-external-interface.md)
- 기존: [docs/runtime-inventory-baseline.md](../../../../runtime-inventory-baseline.md)
- 기존: [docs/release-checklist.md](../../../../release-checklist.md)


## 구현 액션

- [x] COVERAGE의 Q01–Q33을 각각 fixed/retained-by-contract/deferred-with-owner로 처분한다. source-linked rationale 없이 '품질 완료'로 묶지 않는다.
- [x] control lookup·worker handoff·built-in inventory 중복은 H16-002/008/011의 단일 권위로 통합한다. 단지 모양이 비슷한 서로 다른 semantic 함수를 generic helper로 합치지 않는다.
- [x] RequestKey/미사용 pending fields를 public/API/serde/test 소비자까지 검색하고 내부 dead data만 제거한다. reserved burst/DropBestEffort, DAG/reduce/checkpoint, root dedupe는 지원 계약을 명시하고 신규 엔진 기능으로 확장하지 않는다.
- [x] error handling은 expected receiver gone, optional ambient lookup, infrastructure fault를 나눠 annotation/error/metric을 선택한다. 무조건 로그·unwrap 제거를 품질 지표로 쓰지 않는다.
- [x] queue depth/admission phase/deadline start/cleanup/sweep/soft measured memory/retry hint/LocalSet affinity/unsupported nested wait를 구현 계약과 맞춘다.
- [x] generator/scheduler/gate 변경의 사용자 migration 예제와 실제 존재하는 command를 docs에 반영한다. PM 사용자 문서는 H16-020 승인 결과와 맞춘다.
- [x] unknown serde fields·빈 문자열/Unicode class identity 등 입력 호환성은 D01 및 별도 승인 결정대로 유지/변경한다. 증거 없는 신규 결함 수를 늘리지 않는다.

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- Q01–Q33 disposition을 [COVERAGE.md](COVERAGE.md)에 source/test 링크와 함께 기록. Q27은 DEFERRED가
  아니라 HARDENED로 닫았다(모든 waiter를 깨운 뒤 panic 전파). 미완료는 Q33(EXTERNAL) 1건.
- 중복 통합: `execution_plan.rs`(control lookup), `run_detached_job`(worker handoff),
  `builtin_records` + `validate_inventory`(registry).
- dead data: unused `policies` 파라미터 제거; `diagnostics_dropped`(항상 0) 필드 제거; ledger/pending
  audit 필드는 `PermitLedgerView`/`PendingView`로 노출.
- 문서: README §4/§5/§6/§7·집행 의미, library-spec, external-interface, runtime-inventory-baseline,
  release-checklist를 실제 계약(SubstrateSaturated, ClaimOutcome, StageReleaseOutcome, snapshot v2,
  response≠custody, blocking RunFor, 측정 계약, gate inventory)으로 갱신. ADR 0003 링크.
- 고정 sleep: 신규 regression은 handshake/bounded drain; 기존 `deadline_cancel.rs`의 즉시 0 단언은
  `assert_drains`로 교체.
- serde unknown fields·class identity는 RETAIN/DECIDE로 유지(변경 없음).

## 검증 / 완료 조건

- [x] `H16-021-A01` Q01–Q33 모두 disposition + 근거
- [x] `H16-021-A02` 단일 registry/plan/worker protocol; intentional variant(run_local) 근거
- [x] `H16-021-A03` 문서 예제가 실제 API/command와 일치 (README 코드 블록은 실제 API 사용)
- [x] `H16-021-A04` sleep 의존 단언을 handshake/bounded wait로 교체; semgrep test-quality 규칙이
      `#[tokio::test]`를 인식한 뒤 드러난 25건(`let _`·sleep)을 수정 — 남은 `thread::sleep` 3건은 작업이
      wall-clock budget을 초과해야 하는 test로 same-line reason을 가진다
- [x] `H16-021-A05` public symbol/serde diff는 ADR 0003 소비자 표면 inventory

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
cargo test --doc -p taskmesh
just test-architecture
just py-test
```

## 호환성 / 실패 모드

- 품질 정리가 광범위한 unrelated refactor로 번지지 않도록 변환별 행동 보존 테스트를 둔다.
- reserved public API 삭제와 unknown-field 거부는 자동 정리 대상이 아니다.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-19.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)

