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
- source HEAD/tree/path digest
- status: OPEN, PASS, NOT_RUN, or OUT_OF_SCOPE with reason

PASS is invalid if the target/case does not exist, is filtered out, is feature-disabled, or the receipt is bound to another source. A previously K row cannot disappear merely because no implementation ticket owned it.

## Current baseline

- Historical CI receipt: bound to audit HEAD/tree `76559c4`/`215a968`, 16/16 PASS, 561 default workspace tests reported. It does not qualify current source.
- 2026-09-25 replan baseline: HEAD `63bc5fd`/tree `c3310bf`; `just dev` passed 506/506 tests, but the clean-source CI-profile receipt is `qualified=false` because Clippy rejected an unreadable numeric literal in `strict_ingress.rs` and nine subsequent gates were NOT_RUN. A one-line local correction and this documentation are uncommitted; rerun on a new clean unchanged HEAD before claiming CI-profile qualification.
- No current nightly/modelcheck/TSan/fuzz/mutation qualification artifact.

The plan validator checks document mapping only. Its PASS is not product verification.
