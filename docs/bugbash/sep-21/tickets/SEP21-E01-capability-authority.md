# SEP21-E01 — Fail-closed capability authority

- 상태: PLANNED
- 우선순위: P1
- 포함 finding: TM21-008
- 선행: 없음
- write lane: `engine-core`

## 목적

`ungated`와 `registered capability`를 타입으로 분리하고, admission이 raw pool name 또는
missing limit을 unlimited capacity로 해석하지 못하게 한다.

## RCA

- `Option<&str>`가 no-capability와 named capability를 동시에 표현한다.
- `intern_capability`가 admission 중 unknown name을 생성한다.
- registry membership, configured limit, `0 == unlimited` 의미가 서로 다른 owner에 있어
  missing authority가 정상 ungated path로 합쳐진다.

## 확정 근거

- unknown pool을 admission 중 intern하는 경로:
  `crates/taskmesh-engine/src/engine/governor.rs:144-152,174-187`.
- missing limit을 0으로 읽고 0을 ungated로 처리하는 capacity 판정:
  `crates/taskmesh-engine/src/engine/state.rs:498-504`.

## 목표 구조와 불변식

- build/config 단계가 `ResolvedCapability::Ungated | Registered(CapabilityId)`를 만든다.
- `CapabilityId`는 validated registry만 발급하고 raw string에서 admission 중 생성할 수 없다.
- registered pool은 bounded 또는 explicitly-unbounded 정책을 명시한다. missing은 error다.
- engine capacity/snapshot/diagnostic은 동일 registry record를 사용한다.

## 작업 플랜

1. `crates/taskmesh-engine/src/shared/mod.rs`
   - typed resolved capability와 stable registry ID를 추가한다.
2. `crates/taskmesh-engine/src/features/inventory/mod.rs`
   - name→ID resolution과 explicit capacity policy를 단일화한다.
3. `crates/taskmesh-engine/src/engine/governor.rs`
   - `admit_resolved`가 validated handle만 받게 하고 dynamic intern을 제거한다.
4. `crates/taskmesh-engine/src/engine/state.rs`
   - missing record를 limit 0으로 해석하는 분기를 제거한다.
5. `crates/taskmesh-engine/src/lib.rs`
   - engine advanced API가 raw name 대신 validated handle만 노출하도록 정리한다.
6. host plan resolution, facade, consumer migration은 H03이 E01의 final handle을 소비해 한 번만
   통합한다. E01은 `crates/taskmesh/**`를 수정하지 않는다.

## 테스트 플랜

- `crates/taskmesh-engine/tests/substrate_inventory.rs`: typo, unknown, duplicate registration.
- `crates/taskmesh-engine/tests/config_validation.rs`: registry-without-authority와
  authority-without-registry를 구분.
- engine integration fixture: built-in resolved path 전부 registry-backed이고 explicit ungated만
  ungated admission 가능. host/facade fixture는 H03 acceptance다.
- intentional mutant: missing record를 다시 0/unlimited로 바꾸면 named negative가 실패.

## DoD

- `SEP21-E01-A01`: unknown pool은 state 변화 없이 typed reject한다.
- `SEP21-E01-A02`: `None`, empty string, unknown string이 서로 암묵 변환되지 않는다.
- `SEP21-E01-A03`: snapshot의 pool record와 capacity decision이 동일 ID/limit authority를 가진다.
- `SEP21-E01-A04`: queued request가 promotion 때 다른 pool/limit으로 재-resolve되지 않는다.
- `SEP21-E01-A05`: facade에서 raw-string fail-open entrypoint가 제거되거나 unchecked surface로도
  남지 않는다.

## 금지되는 임시방편

- unknown pool에 임의 limit 1 적용.
- warning만 남기고 admission 허용.
- host allowlist만 고치고 public engine API 유지.
- registry와 limit map을 별도로 dual-write.

## 검증 명령 후보

```sh
cargo test --locked -p taskmesh-engine --test substrate_inventory --test config_validation
```
