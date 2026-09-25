# BG25-012 — scenario evidence and exact-source gate integration

- 상태: IMPLEMENTED
- 우선순위: P1
- 선행: BG25-002–BG25-011
- 소유: integration/CI owner

## 목적

전체 104개 audit scenario의 contract와 fixture를 actual test discovery, feature/platform selectors, gate inventory, and exact-source receipts에 연결한다. 파일 존재나 coverage percentage를 closure로 사용하지 않는다. 51개 기존 K는 regression baseline으로 보존하고 53개 P/G는 owner ticket과 연결한다.

## 근거

- The historical CI-profile receipt is source-bound to its older audit HEAD and is not reused for this implementation.
- `test-rayon` executes host lib plus one focused integration test.
- `fuzz-check` compiles/lints; `bench-gate` is allocation-only.
- no current modelcheck/TSan/fuzz/mutation receipt exists.

## 변경 파일

- `scenario-evidence.json` and `validate_scenario_evidence.py`
- `Justfile`, `tools/gates/{inventory,required}.json`, gate tests and target catalog
- only if policy approves: `.github/workflows/**`
- `docs/release-checklist.md` and final ticket statuses

## 작업 계획

1. 104 scenarios each record origin(K/P/G), exact target/case/oracle/feature/platform/gate/source; P/G rows also record owner ticket.
2. Query actual Cargo test collection; reject missing, filtered, zero-test, cfg-disabled cases.
3. Expand Rayon selector to execute new feature integration cases without duplicating default tests unnecessarily.
4. Update modelcheck producer manifest only for new bounded models.
5. Run owner-local → `just dev` → clean isolated `just verify-macos-ci` in that order.
6. Run nightly only when explicitly authorized; retain every NOT_RUN/failure honestly.

## DoD

- Every 104 scenario row is PASS, OPEN, NOT_RUN, or OUT_OF_SCOPE with evidence; no fabricated PASS and no dropped K baseline.
- Every completed implementation ticket has an actually selected deterministic test and accepted contract reference.
- Gate inventory, required set, recipes, test catalog, and documentation agree.
- A clean unchanged-HEAD CI-profile receipt passes and binds HEAD/tree/path digest.
- Nightly/release status is reported separately and never inferred from CI profile.

## 검증

- Run `python3 validate_plan.py` and `python3 validate_scenario_evidence.py`.
- Execute changed gate/tooling owner tests, actual test collection, and clean `just verify-macos-ci`.
- Validate saved receipt again after the run; do not use a receipt from another HEAD.

## 인계 및 중단 조건

- Missing external consumer/ingress ownership leaves that row OPEN.
- Any required-gate or workflow-trigger policy change requires separate review.
- Mutation, TSan, fuzz campaign, coverage, and modelcheck are not run without explicit authorization.
