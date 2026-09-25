# BG25-012 — scenario evidence and exact-source gate integration

- 구현 상태: IMPLEMENTED
- 증명 상태: RECEIPT_REQUIRED
- 외부 상태: NOT_APPLICABLE
- 우선순위: P1
- 선행: BG25-002–BG25-011
- 소유: integration/CI owner

## 2026-09-25 재감사 잔여

- **직전 clean source:** `da5356b253fc69450e98869a2c94e4a3e770747e`의 `target/verification/macos-gates.json`을 `--validate-receipt --expected-head`로 재검증했다. macOS CI profile 필수 16/16 PASS, required NOT_RUN/FAIL 0, HEAD/tree/path digest 전후 일치. 이는 해당 commit에만 유효하다.
- **필수 마지막 단계:** `scenario-evidence.json`과 `hardening_effect_retirement.rs`는 `605070d`에 별도 커밋했고 B27 focused 1/1 및 두 plan validator가 PASS였다. 남은 tracked 티켓 문서를 검토·커밋해 clean HEAD를 고정한 뒤 `just dev`와 `just verify-macos-ci`를 다시 실행하고 저장된 영수증을 그 HEAD로 재검증한다. 그 전까지 `RECEIPT_REQUIRED`를 유지한다. 104 `MAPPED`는 정적 후보이며 새 영수증과도 의미상 동치가 아니다.
- **별도 범위:** 외부 D1–D9 채택은 BG25-001~003; full nightly/modelcheck·TSan·fuzz·mutation 및 release는 명시적 자격 요청이 있을 때만 실행한다. 의도적으로 꺼진 hosted workflow를 이 티켓의 미완료 게이트로 세지 않는다.

## 목적

전체 104개 audit scenario의 contract와 fixture를 actual test discovery, feature/platform selectors, gate inventory, and exact-source receipts에 연결한다. 파일 존재나 coverage percentage를 closure로 사용하지 않는다. 51개 기존 K는 regression baseline으로 보존하고 53개 P/G는 owner ticket과 연결한다.

## 근거

- Historical receipts validate only their recorded committed source. The final ignored artifact must be produced and validated after every tracked change is committed.
- `test-rayon` declares exact host lib, Rayon, direct/host reservation, and public wire roundtrip selectors required by H18. Actual selected count and exit status come from the final receipt.
- `fuzz-check` compiles/lints; `bench-gate` is allocation-only.
- Modelcheck/TSan/fuzz campaign/coverage/IAI/mutation remain NOT_RUN and are not implied by this CI profile.
- Static mapping, recipe selectors, and nightly evidence metadata validation fail closed. Compound oracles use explicit supporting cases where one test cannot honestly prove the whole claim. The 104 MAPPED rows still assert source/case existence only; actual Cargo collection and execution status come from the exact-source receipt.

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
5. Run owner-local → `just dev` → clean `just verify-macos-ci` in that order; repeat after the ticket/document source is committed.
6. Run nightly only when explicitly authorized; retain every NOT_RUN/failure honestly.

## DoD

- Every one of the 104 scenario rows is MAPPED, OPEN, NOT_RUN, or OUT_OF_SCOPE; MAPPED asserts only a candidate target/case exists, so the tracked file cannot fabricate an execution PASS.
- Every completed implementation ticket has an actually selected deterministic test and accepted contract reference.
- Gate inventory, required set, recipes, test catalog, and documentation agree.
- A clean unchanged-HEAD CI-profile receipt separately records gate execution and binds HEAD/tree/path digest; it is never copied into the tracked static map.
- Nightly/release status is reported separately and never inferred from CI profile.

## 검증

- Run `python3 validate_plan.py` and `python3 validate_scenario_evidence.py`.
- Execute changed gate/tooling owner tests, actual test collection, and clean `just verify-macos-ci`.
- Validate saved receipt again after the run; do not use a receipt from another HEAD.

## 인계 및 중단 조건

- Missing external consumer/ingress ownership leaves that row OPEN.
- Any required-gate or workflow-trigger policy change requires separate review.
- Mutation, TSan, fuzz campaign, coverage, and modelcheck are not run without explicit authorization.
- Freeze all owners' changes, commit the final source, and issue/validate one clean final-HEAD CI-profile receipt. Never attach a receipt from another HEAD/tree/path digest.
