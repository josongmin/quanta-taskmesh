# H16-018 — 검증 inventory·scanner enrollment·local/CI parity

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P1 (원본 bug severity 변경 아님)
- 책임 역할: C — CI/gate owner (실제 assignee 미지정)
- 선행 완료: [H16-014](H16-014-production-regression-proof.md), [H16-017](H16-017-performance-gates.md), [H16-019](H16-019-dependency-and-toolchain.md), [H16-020](H16-020-pm-validated-render-plan.md)
- 원본 finding: [TM16-006](../../../bugbash/sep-16-general/tickets/TM16-006-local-gates-not-enforced-by-ci.md), [TM16-016](../../../bugbash/sep-16-general/tickets/TM16-016-architecture-checker-false-green.md), [TM16-017](../../../bugbash/sep-16-general/tickets/TM16-017-semgrep-test-enrollment-gap.md), [TM16-033](../../../bugbash/sep-16-general/tickets/TM16-033-test-quality-refers-to-missing-gates.md)
- 배타적 write lease: `ci`, `gate-tools`, `engine-tests`, `host-tests`; [적용 순서](EXECUTION.md) 준수

## 목적

필수 검증의 실행 집합을 inventory로 고정하고 scanner와 local/CI 양쪽의 누락을 fail-closed로 검출한다.

## 변경 범위

- 기존: [Justfile](../../../../Justfile)
- 기존: [.github/workflows/ci.yml](../../../../.github/workflows/ci.yml)
- 기존: [.github/workflows/bench.yml](../../../../.github/workflows/bench.yml)
- 기존: [tools/arch/check_crate_boundaries.py](../../../../tools/arch/check_crate_boundaries.py)
- 기존: [tools/semgrep/tests/test_rules_fire.py](../../../../tools/semgrep/tests/test_rules_fire.py)
- 기존: [tools/semgrep/README.md](../../../../tools/semgrep/README.md)
- 제안 경로: `tools/gates/inventory.json` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음
- 제안 경로: `tools/gates/run.py` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음
- 제안 경로: `tools/gates/validate_inventory.py` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음
- 제안 경로: `tools/gates/tests/test_inventory.py` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [x] 현재 Justfile·CI·bench workflow에서 command/cwd/env/toolchain/features/platform/prerequisites/required 여부/timeout/artifact를 추출하여 stable gate id를 부여한다. 현행 명령을 임의 rename하지 않는다.
- [x] 필수 gate 집합은 별도 승인 목록과 비교한다. inventory 삭제만으로 required gate를 없앨 수 없고 unknown/duplicate/missing/skipped-required는 qualification 실패다.
- [x] local-only strict deny/Semgrep/Python 검사와 CI-only Rayon/rustdoc/bench smoke를 모두 분류한다. 플랫폼별 필수 집합은 달라도 qualification matrix의 합집합은 누락되지 않아야 한다.
- [x] architecture scanner는 workspace metadata에서 모든 package/dependency edge를 읽고 unknown package/rule/optional dependency/read error를 실패시킨다. 제품 의미 정책과 worker governance 경계를 별도 규칙으로 유지한다.
- [x] Semgrep 실제 integration fixtures를 test collection에 등록하고 fixture 양성·음성 control, rule id별 expected hit를 검사한다. 성공한 pytest 프로세스만으로 collection을 입증하지 않는다.
- [x] 기존 중복 Loom rail은 feature·model coverage가 같을 때만 합친다. PM 실제 target lint와 원본 mixed-soak negative, bench helper tests를 inventory에 등록한다.
- [x] 존재하지 않는 mutants-critical/cov-gate 문서 참조는 실제 gate 구현 및 등록 후 연결하거나 삭제한다. 문자열 grep·장난감 모델을 실행 증거로 승격하지 않는다.
- [x] runner는 gate별 exit/status/test count/환경/receipt를 기록하고 required gate 미실행을 실패 처리한다. branch protection 실제 적용 여부는 H16-022에서 별도 확인한다.
  → gate별 exit/status/duration/status_line(self-report verdict) 기록; test count는 mutation receipt와 output tail에만 있음

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- `tools/gates/inventory.json`(20 gates) + **별도 파일** `required.json`(20 required) +
  `validate_inventory.py`: required id 누락/unknown/duplicate/recipe 부재/CI-only/local-only/
  `just gate` fast-tier drift 모두 실패. `just gates-inventory`로 wired, fast gate에 포함.
- `tools/gates/run.py`: gate별 exit/status/duration receipt; required 미실행·SKIPPED_PLATFORM·NOT_RUN은
  NOT_QUALIFIED.
- `check_crate_boundaries.py` 재작성(TM16-016): `workspace_members` id 기반, 외부 edge 검사, optional-only
  edge 강제, read failure/missing dir 실패. 원본 checker에 대해 11개 negative fixture FAIL, 신규 15 PASS.
- Semgrep enrollment(TM16-017): 저장소 `.semgrepignore`로 `crates/*/tests/` 포함(50→118 files);
  `test_real_integration_tests_are_inside_the_scanned_target_set`가 target set을 단언; test-quality
  fixture는 `tests/` 경로에서 실행. 새로 스캔된 test에서 실제 finding 5건을 수정.
- TM16-033: `cov-gate`/phantom 참조 제거, `mutants-critical` 실제 recipe화, recipe drift test.
- `ci.yml`이 `just` recipe만 호출; loom 중복 job 제거(Q08).

## 검증 / 완료 조건

- [x] `H16-018-A01` required 삭제/unknown/duplicate/drift 각각 실패 (`tools/gates/tests`)
- [x] `H16-018-A02` unknown crate/engine→tokio/engine→bench/nonoptional rayon/missing dir/unreadable/
      metadata 실패 각각 nonzero; control PASS (`tools/arch/tests`)
- [x] `H16-018-A03` `.semgrepignore` 제거 시 enrollment test가 자기 assertion으로 FAIL (확인함)
- [x] `H16-018-A04` PM lint·Rayon·rustdoc·bench smoke·consumer-msrv·bench-iai 모두 inventory에 등록;
      platform-conditional 명시
- [x] `H16-018-A05` workflow가 inventory에 없는 recipe를 호출하거나 그 반대면 parity 실패 — 감사(A3-P2-5)
      후 workflow `run:` block은 YAML로 parse되어 multi-line/`&&`/flag 형태를 모두 본다. 감사(A3-P1-2):
      `just proof`가 required 4개를 건너뛰던 gap → `just matrix` 추가, `proof` expansion == `required.json`을
      validator가 강제 (`test_the_real_proof_recipe_expands_to_exactly_the_required_set`; mutation
      `proof-parity-not-checked`). 추가 범위: inventory 24 gates — `tsan`(ThreadSanitizer, nightly+rust-src,
      NOT_RUN 가능), `coverage-report`(cargo-llvm-cov 수치 기록, threshold 아님), `fuzz`(libFuzzer, nightly+cargo-fuzz,
      NOT_RUN 가능; `taskmesh-fuzz status=CLEAN`만 PASS)와 fast tier의 `fuzz-check`(target을 stable에서 type-check); `just proof`가 전부 포함;
      CI `tsan`·`coverage` job + clean checkout에서 receipt를 만드는 `qualification` job; parity는 집행까지
      검사(`continue-on-error`/`if:`/`|| true`/`set +e`/`exit 0` 거절).

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
just gate
just proof
just test-architecture
just semgrep
just py-test
just py-lint
```

## 호환성 / 실패 모드

- inventory와 required 목록을 동일 생성 입력 하나로 만들면 동시 누락을 감지하지 못한다.
- 기존 strict gate reds를 원인 수정 없이 완화하거나 baseline reset하여 통과시키지 않는다.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-19.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)
