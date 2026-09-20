# SEP21-E03 — Non-wrapping memory measurement sequence

- 상태: LOCALLY_VERIFIED
- 우선순위: P1
- 포함 finding: TM21-016
- 선행: 없음
- write lane: `engine-core`

## 목적

memory measurement ordering을 wrap 가능한 숫자 관례가 아니라 소진 상태를 포함한 monotonic
protocol로 만든다.

## RCA

- caller-provided explicit epoch와 engine-generated implicit epoch가 같은 `u64` domain을 쓴다.
- terminal value 정책이 없고 implicit path가 `wrapping_add(1)`을 사용한다.
- stale/exhausted/unknown outcome이 충분히 분리되지 않으면 fail-closed 처리가 불가능하다.

## 확정 근거

- unrestricted explicit epoch와 implicit `wrapping_add(1)`:
  `crates/taskmesh-engine/src/engine/governor.rs:559-590`.
- `epoch <= current`를 모두 stale로 처리하는 ledger:
  `crates/taskmesh-engine/src/features/memory/mod.rs:100-121`.

## 목표 구조와 불변식

- accepted epoch는 permit lifetime 동안 strictly increasing이다.
- exhaustion은 ledger/activity clock/aggregate 변경 전 typed outcome이다.
- implicit next는 checked increment만 사용한다.
- stale, exhausted, unknown permit, conversion failure는 서로 다른 outcome이다.

## 작업 플랜

1. `crates/taskmesh-engine/src/features/memory/mod.rs`
   - `MeasurementSequence` 또는 동등한 checked domain과 `EpochExhausted`를 정의한다.
   - validation을 aggregate update보다 먼저 수행한다.
2. `crates/taskmesh-engine/src/engine/governor.rs`
   - `wrapping_add`를 제거하고 explicit/implicit path를 한 validator로 합친다.
3. `crates/taskmesh-engine/src/lib.rs`
   - advanced API의 typed outcome export를 갱신한다.
4. closure evidence에 MAX 처리, reporter ownership, retry 불가 조건의 문서 delta를 남긴다.
   shared library spec/external interface 반영은 R01이 통합한다.

## 테스트 플랜

- `crates/taskmesh-engine/tests/hardening_memory_epochs.rs`
  - `MAX-1 -> implicit MAX -> next implicit exhausted`.
  - explicit MAX reject 정책을 택하면 apply 전 state 동일성.
  - terminal epoch 뒤 low/high measurement가 under/overcharge를 고착시키지 않음.
  - stale/exhausted/unknown/conversion failure exact variant.
- `loom_governance.rs` 또는 focused concurrency test에서 explicit/implicit reporter race.

## DoD

- `SEP21-E03-A01`: production epoch 증가 경로에 wrapping/saturating reset이 없다.
- `SEP21-E03-A02`: reject 전후 permit/class/root/global memory와 `last_touched_ms`가 동일하다.
- `SEP21-E03-A03`: concurrent reporters의 winner와 loser outcome이 deterministic하다.
- `SEP21-E03-A04`: permit 종료 전에도 terminal sequence가 capacity를 stale value로 영구
  고정하지 않는다.

## 금지되는 임시방편

- `saturating_add`만 적용.
- MAX에서 0 또는 1로 reset.
- exhaustion을 bool false로 합침.
- ledger와 aggregate 중 하나만 보정.

## 검증 명령 후보

```sh
cargo test --locked -p taskmesh-engine --test hardening_memory_epochs
just loom
```

## Closure evidence (2026-09-21)

- source: `main@32189ab0bbd44aad83921167eeb076223fbfe230`
- sequence API: `MeasurementSequence::checked_next`; no production epoch path uses wrapping or
  saturating reset. Explicit and implicit reporters converge on `memory::reconcile`.
- typed outcomes: `Applied`, `UnknownPermit`, `StaleEpoch`, `EpochExhausted`, and
  `ConversionFailed`.

| Acceptance | Local evidence |
| --- | --- |
| SEP21-E03-A01 | Source audit plus clippy confirms the implicit path uses `checked_next`; `terminal_measurement_sequence_never_wraps_or_mutates_on_exhaustion` covers `MAX-1`, implicit `MAX`, and subsequent explicit/implicit attempts. |
| SEP21-E03-A02 | The terminal-sequence fixture compares the full permit ledger and snapshot before/after each exhausted update; conversion/stale/unknown tests likewise apply no aggregate or activity change. |
| SEP21-E03-A03 | Loom `unordered_concurrent_reconciles_both_apply` exhaustively checks two unordered reporters; both apply and the committed ledger reaches epoch 2. |
| SEP21-E03-A04 | Exhaustion cannot wrap into a newer low epoch or change the held charge; normal release still clears the terminal-sequence permit and all memory accounting. |
| SEP21-E03-A05 | Stale, exhausted, unknown-permit, and conversion failure use exact distinct variants. |
| SEP21-E03-A06 | `last_touched_ms` advances only on an applied measurement and remains monotonic under late samples. |

R01 documentation delta: `reconcile_memory` now returns `ReconcileOutcome` rather than `bool`;
`MeasurementSequence` and `EpochExhausted` are public engine API; epoch exhaustion is non-retryable
for that permit and release remains required to end its capacity custody.
