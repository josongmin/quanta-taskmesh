# BG25-003 — wire·consumer·오류 경계

- 구현 상태: IMPLEMENTED
- 증명 상태: STATIC_MAPPED
- 외부 상태: OPEN
- 우선순위: P1
- 선행: BG25-001 D9
- 소유: contract/consumer owner

## 목적

식별자, provenance, legacy alias, Snapshot version, enum evolution, close/preflight precedence를 consumer가 오해하지 않도록 현재 동작과 목표 오류 계약을 고정한다.

## 근거

- Snapshot만 schema version을 갖고 derived enum Deserialize는 unknown variant를 거절한다.
- `conservation_violation()`은 wire snapshot과 live permit ledger의 완전한 일치를 증명하지 않는다.
- close 전에 malformed/capability preflight가 실행될 수 있는데 rustdoc 표현과 외부 문서가 완전히 정렬되지 않았다.

## 변경 파일

- `crates/taskmesh-contract/src/{task,validation,snapshot,verdict}.rs`
- `crates/taskmesh-engine/src/engine/governor.rs` rustdoc; precedence 변경은 계약 승인 시에만
- tests: `contract_roundtrip.rs`, `snapshot_oracle.rs`, `error_surface.rs`
- `docs/taskmesh-{library-spec,external-interface}.md`

## 작업 계획

1. wire fixtures와 runtime-only error fixtures를 분리한다.
2. Snapshot version mismatch와 future enum decode error를 consumer entrypoint에서 판정한다.
3. close + valid/malformed/foreign-capability precedence를 고정하고 side-effect 0을 확인한다.
4. identifier byte boundaries와 thread-label sanitization을 분리한다.

## DoD

- `BG25-003-B18`: custom/legacy `PlanSource`와 source-key boundary가 raw/validated 경로에서 일관된다.
- `BG25-003-B24`: metadata truth는 classifier 책임이고 engine은 exact provenance preservation만 보장한다고 문서·fixture가 일치한다.
- `BG25-003-D03`: identifier byte limit ±1, NUL, non-ASCII와 정확한 오류 위치를 표로 검증한다.
- `BG25-003-D13`: Snapshot version/u128 string/derived validation 범위와 consumer reject를 구분한다.
- `BG25-003-D14`: 유효 class label 정제와 금지 class의 spawn-before reject를 독립 관측한다.
- `BG25-003-D17`: closed valid/malformed/foreign capability precedence와 state/waker 0 변화를 고정한다.
- `BG25-003-D23`: future enum variant와 Snapshot version mismatch가 consumer의 명시적 오류/negotiation 경로로 끝난다.

## 검증

- Contract tests + public facade consumer test + doctest/rustdoc.
- 외부 consumer가 있으면 해당 버전 조합의 receipt를 별도로 요구한다.

## 인계 및 중단 조건

- Tolerant envelope나 enum fallback을 추가하면 새 versioned contract로 설계한다.
- Snapshot helper를 내부 ledger oracle로 승격하지 않는다.
