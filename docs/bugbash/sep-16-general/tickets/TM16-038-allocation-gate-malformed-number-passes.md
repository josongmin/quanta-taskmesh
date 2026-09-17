# TM16-038 — Allocation gate가 malformed 숫자를 0/숫자 prefix로 변환하여 통과시킨다

- Severity: P3
- Status: OPEN / actual shell front-door negative-input reproduction
- Lane: B — benchmark gate input validation
- Baseline: `9ae9547216c70458f54f37368a67661321060886`, 2026-09-16

## 근거

- `tools/bench-gate.sh:23-31`: sed의 `[0-9.][0-9.]*`는 `...`, `3.0.0`도 허용한다. nonempty 여부만 검사하고 awk의 `v + 0`으로 비교한다.
- `crates/taskmesh-bench/examples/alloc_probe.rs:66`: 현재 producer는 고정 소수점 `{per_op:.3}` 한 줄을 출력한다. 이번에는 정상 producer에서 malformed output이 자연 발생했다는 증거가 없다.
- [Retained shell diagnostic](evidence/allocation-gate-input.sh): subprocess에 export한 cargo 함수가 stdout/status만 제어하고 실제 `tools/bench-gate.sh`를 실행한다. production script 수정이나 dependency 실행은 없다.

## Trigger / 관찰

`MAX_ALLOCS_PER_OP=4`에서 actual gate 결과:

| Probe output / status | Gate exit | 판정 |
| --- | --- | --- |
| `allocations/op = 3.000` | 0 | 정상 below-limit control |
| `allocations/op = 5.000` | 1 | 정상 above-limit rejection |
| `allocations/op = ...` | 0 | invalid metric을 0으로 비교하는 false green |
| `allocations/op = 3.0.0` | 0 | invalid metric의 숫자 prefix 3만 비교하는 false green |
| metric marker 없음 | 1 | missing input은 거부함 |
| cargo exit 17 | 17 | producer process failure는 전파함 |

실행: `bash docs/bugbash/sep-16-general/tickets/evidence/allocation-gate-input.sh`.

## 원인 / 영향 / 범위

측정값 validation과 숫자 변환을 구분하지 않아 stdout format drift/corruption이 성공 metric으로 바뀐다. 일반 cargo failure swallowing은 아니다. TM16-018은 producer가 실제 admit/release를 수행했는지 검증하지 않는 문제이고, 이 티켓은 별도의 consumer parser boundary다.

현재 정상 probe를 그대로 실행하여 regression이 은폐되었다는 주장은 하지 않는다. producer-controlled malformed input이 필요하므로 P3 검증 도구 결함으로 분류한다. 여러 metric 줄도 탐색했지만 현재 macOS awk에서는 exit 2로 실패했으므로 multiple-line false green으로 기록하지 않는다.

## 보완 계획

- 허용하는 출력 문법을 한 개의 완전한 finite/nonnegative 숫자로 검증하고 trailing junk를 거부한다. marker count도 명시한다.
- threshold도 동일한 숫자 validation을 적용한다. producer/gate 계약을 구조화된 output으로 바꾸는 경우 stderr 진단과 metric을 분리한다.
- parse 실패를 0 또는 default baseline으로 대체하지 않는다.

## Acceptance / 회귀 검증

- malformed dots, partial number, trailing junk, missing/duplicate marker, invalid threshold는 actual gate에서 nonzero다.
- below/equal/above-limit controls 및 producer nonzero 전파를 보존한다.
- supported macOS/Linux shell 도구에서 동일한 fail-closed 결과를 검증한다. 이번 실행은 현재 macOS 도구만 확인했다.
