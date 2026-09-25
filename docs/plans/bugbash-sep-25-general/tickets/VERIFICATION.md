# Verification contract

## Evidence levels

1. **Reproduction:** deterministic negative fixture against the pre-fix behavior; exact observable and side-effect ledger.
2. **Owner-local:** changed crate plus direct consumers; diagnostic, not qualification.
3. **Cross-package dev:** `just dev`; still not a qualification verdict.
4. **CI profile:** clean unchanged HEAD via `just verify-macos-ci`; all applicable non-nightly required gates pass.
5. **Nightly/release:** modelcheck, TSan, fuzz, coverage, IAI, and mutation only when explicitly requested. Partial output never qualifies.

Exact ticket commands are centralized in [COMMANDS](COMMANDS.md) to avoid divergent copies in twelve tickets.

## Fixture rules

- Use barriers, injected clocks, and bounded channels; do not prove ordering with sleep alone.
- Derive expected ownership/accounting from input events, not from the production snapshot or `permit_ledgers()` being tested.
- Assert zero side effects on preflight rejection: closure calls, permits, tickets, queue depth, role pool, and physical-domain occupancy.
- Assert response, root completion, cleanup, worker termination, lease release, second admission, and drain as separate events.
- Preserve normal controls beside every negative fixture.
- A modelcheck change must update the producer manifest and obtain a source-bound model receipt; direct `just loom` or `just shuttle` is debugging evidence only.

## Integration evidence inventory

BG25-012 adds `scenario-evidence.json` with one row for every one of the 104 audit scenarios. The 51 existing K rows are baseline-owned; the 53 P/G rows also name their BG25 implementation ticket:

- scenario ID, baseline/P/G origin, and optional owner ticket
- accepted contract decision
- exact test target and case
- independent oracle summary
- required feature/platform
- selecting gate
- static status: MAPPED, OPEN, NOT_RUN, or OUT_OF_SCOPE

MAPPED means that candidate test declarations exist, are not ignored or disabled for the row's feature, and any Rayon case has an exact selected recipe. It does not assert semantic sufficiency, Cargo execution, or a passing result. Exact HEAD/tree/path digest and gate PASS/FAIL/NOT_RUN live only in the generated CI receipt. A previously K row cannot disappear merely because no implementation ticket owned it.

## Current baseline

- Historical receipts remain valid only for their recorded HEAD/tree/path digest and never qualify later source.
- The current source adds fail-closed mapping validation, executable Rayon selectors, bounded async fixtures, and direct compound oracles for overload, capacity, policy, concurrency, child lifetime, and host/simulator accounting.
- `test-architecture` runs both the plan validator and 104-row mapping validator, so cardinality, ownership, missing-case, inventory, and exact Rayon recipe-selector drift fail the normal CI profile.
- Clean `a23dcda`의 `verify-macos-ci` receipt는 16/16 필수 gate PASS이며 `--validate-receipt --expected-head`로 재검증됐다. 그 뒤의 tracked 변경에는 그 영수증을 재사용하지 않는다. 최종 clean HEAD의 자격은 외부 영수증을 해당 HEAD로 재검증해서만 판정한다.
- No nightly/modelcheck/TSan/fuzz/coverage/IAI/mutation qualification is inferred from deterministic or CI-profile success.

Both plan validators check static mapping only. Their PASS is not scenario execution or product qualification.
