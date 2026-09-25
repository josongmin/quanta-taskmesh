# BG25-001 — 계약과 신뢰 경계 고정

- 구현 상태: IMPLEMENTED
- 증명 상태: REVIEW_REQUIRED
- 외부 상태: OPEN
- 우선순위: P0
- 선행: 없음
- 소유: contract/API 통합 owner

## 목적

[DECISIONS](DECISIONS.md)의 D1–D9를 실제 consumer와 대조해 승인한다. raw DTO, strict ingress, direct Governor API, Tokio host, external planner, wire consumer의 보장을 분리하고 후속 티켓이 서로 다른 의미의 안전·완료·소유권을 구현하지 않게 한다.

## 근거

- Raw DTO Serde는 호환성 경로로 남아 unknown/default를 허용한다. Untrusted bytes는 `taskmesh::{parse_task_spec, parse_runtime_config}`를 명시적으로 호출해야 한다.
- `PermitId`/`Ticket`은 Governor authority를 포함하는 opaque handle로 이전됐다. Runtime은 build 시 검증한 executor descriptor를 사용한다.
- `CompleteBy` caller 응답과 owned-runtime teardown/worker custody는 분리됐다. `RunFor`는 worker 실행 예산이다.
- D1–D9는 [DECISIONS](DECISIONS.md)에 라이브러리 결정으로 accepted다. 실제 외부 ingress/planner/wire consumer의 호출점과 채택 증거는 이 저장소에서 확인되지 않았다.

## 변경 파일

- `docs/taskmesh-library-spec.md`
- `docs/taskmesh-external-interface.md`
- 필요 시 새 ADR; accepted ADR은 조용히 재해석하지 않는다.
- 공개 API 영향 조사: `crates/taskmesh-contract/src/{task,ports,verdict}.rs`, `crates/taskmesh-engine/src/shared/mod.rs`, `crates/taskmesh/src/{builder,runtime}.rs`

## 작업 계획

1. 실제 bytes→DTO→validated plan/config→runtime 경로와 외부 consumer를 inventory한다.
2. D1–D9마다 current/target behavior, typed failure, owner, migration, semver를 기록한다.
3. H30/H34/B25/B28의 목표 계약을 승인한 뒤 후속 코드 티켓을 unblock한다.
4. out-of-scope인 자동 fan-out/reduce, undeclared wait 추론, ambient executor 전체 제어를 명시한다.

## DoD

- D1–D9 각각 승인 상태, source anchor, consumer owner, migration이 있다.
- `RunFor`/`CompleteBy`, caller response, cleanup, custody, drain 시점이 구분된다.
- raw ID same-Governor 제한 또는 opaque handle migration이 결정된다.
- 외부 owner가 없으면 해당 integration 항목은 OPEN으로 남는다.

## 검증

- 문서 링크와 실제 공개 타입명을 대조한다.
- doctest/rustdoc/consumer-MSRV는 최종 committed HEAD에서 BG25-012의 clean macOS CI-profile receipt로 다시 증명한다. 과거 receipt는 현재 소스 자격으로 재사용하지 않는다.
- 이 티켓의 문서 PASS를 product PASS로 기록하지 않는다.

## 최종 감사

- D1–D9 라이브러리 결정은 문서에 수용됐지만 사람 검토와 외부 consumer별 compatibility/owner 승인은 증명되지 않았다. 확인한 로컬 `code-new` Rust 소스에서 strict parser 호출은 Taskmesh 라이브러리와 테스트에만 있었다. 저장소 밖 배포는 이 검색으로 부재를 증명할 수 없다.
- 이 티켓의 남은 작업은 실제 bytes ingress, parent planner, wire consumer의 저장소·호출점·담당자와 migration 결정을 기록하는 것이다. 새 production 결함을 전제하지 않는다.

## 인계 및 중단 조건

- Breaking API 또는 raw wire 의미 변경은 semver/migration 승인 전 중단한다.
- 결정 후 BG25-002~007에 타입·오류·호환성 계약을 인계한다.
