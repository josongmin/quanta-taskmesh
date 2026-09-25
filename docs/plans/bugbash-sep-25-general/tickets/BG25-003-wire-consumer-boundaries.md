# BG25-003 — wire·consumer·오류 경계

- 구현 상태: IMPLEMENTED
- 증명 상태: STATIC_MAPPED
- 외부 상태: OPEN
- 우선순위: P1
- 선행: BG25-001 D9
- 소유: contract/consumer owner

## 2026-09-25 재감사 잔여

- **저장소 코드:** B18/B24/D03/D13/D14/D17/D23의 library-side fixture가 CI `test`에서 실행됐다. 새 확정 결함 없음.
- **필수 외부 인계:** classifier가 provenance의 사실성을 어디서 보장하는지, serialized Snapshot/enum을 읽는 소비자가 있는지 배포 owner가 확인한다. 실제 wire가 있으면 unknown version/variant의 실패·협상 경로를 그 소비자의 fixture와 receipt로 입증한다. 없으면 확인한 topology를 근거로 N/A를 기록한다.
- **증거 경계:** raw roundtrip과 library consumer fixture만으로 외부 classifier·wire adoption을 닫지 않는다. 문서 변경 후 CI 재검증은 BG25-012가 소유한다.

## 목적

식별자, provenance, legacy alias, Snapshot version, enum evolution, close/preflight precedence를 consumer가 오해하지 않도록 현재 동작과 목표 오류 계약을 고정한다.

## 근거

- Snapshot만 schema version을 갖고 derived enum Deserialize는 unknown variant를 거절한다.
- `conservation_violation()`은 wire snapshot과 live permit ledger의 완전한 일치를 증명하지 않는다.
- D17 closed-admission preflight 우선순위는 `governor.rs` rustdoc과 `hardening_close_admission.rs`의 무부작용 fixture에 정렬됐다.
- B18/B24/D03/D13/D14/D17/D23은 104행 정적 목록에 target/case가 연결돼 있다. 외부 wire consumer의 버전 협상과 오류 처리는 별도 채택 증거가 필요하다.

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

- contract/public facade test와 doctest/rustdoc는 최종 committed HEAD의 BG25-012 receipt에서 판정한다.
- 외부 consumer가 있으면 해당 버전 조합의 receipt를 별도로 요구한다.

## 최종 감사

- 라이브러리 wire/version/closed-preflight fixture는 정적 목록에 연결돼 있다. 배포 consumer가 Snapshot version·미래 enum decode 실패·opaque handle migration을 어떻게 처리하는지는 이 저장소의 fixture로 검증할 수 없다. 외부 owner의 호출점과 실행 증거가 OPEN이다.
- D13은 미래 enum decode/Snapshot 버전 case에 TaskSpec, RuntimeConfig, Snapshot, legacy PlanSource 왕복 supporting cases를 연결했다. D14는 허용된 특이 class의 thread label과 NUL·비ASCII·길이 초과 class 거절 case를 함께 연결한다. 외부 consumer의 버전 협상은 여전히 별도 채택 증거가 필요하다.

## 인계 및 중단 조건

- Tolerant envelope나 enum fallback을 추가하면 새 versioned contract로 설계한다.
- Snapshot helper를 내부 ledger oracle로 승격하지 않는다.
