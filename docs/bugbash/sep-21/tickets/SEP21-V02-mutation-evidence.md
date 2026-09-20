# SEP21-V02 — Transactional mutation campaign authority

- 상태: IMPLEMENTED_UNQUALIFIED
- producer state: runner와 integrated focused campaign 검증 완료; full current-source generated
  sweep는 `NOT_RUN`, bounded `taskmesh-rayon` generated subset은 `FAIL`
- 우선순위: P1 release blocker
- 포함 finding: TM21-012, TM21-017
- 선행: V01
- write lane: `mutation-proof`

## 목적

curated mutation과 generated cargo-mutants campaign을 isolated exact-source transaction으로
실행하고, process truth·baseline·failure set·denominator를 보존한다.

## RCA

- classifier가 subprocess return code와 unrelated failure를 무시한다.
- 동일 command/env/profile의 unmutated baseline이 없다.
- live source를 제자리 변경하고 shared target을 사용한다.
- `finally` 복원은 concurrent reader와 SIGKILL을 보호하지 못한다.
- curated kill count와 generated sweep score의 authority가 과거 문구에서 섞였다.

## 확정 근거

- return code를 사용하지 않는 classifier:
  `tools/verification/run_mutations.py:237-288`.
- live source in-place mutation과 `finally` 복원:
  `tools/verification/run_mutations.py:339-369`.
- dirty tree 허용과 isolation/target lock 없는 campaign loop:
  `tools/verification/run_mutations.py:386-402,522-531`.
- per-mutant receipt에 process exit가 없는 결과 shape:
  `tools/verification/run_mutations.py:453-465`.
- generated sweep는 현재 별도 `NOT_RUN` disclosure만 가짐:
  `tools/qualification/receipt.py:359-383`.

## 목표 구조와 불변식

- campaign은 immutable source identity에서 isolated copy/worktree와 dedicated target을 만든다.
- command/env/profile fingerprint별 baseline PASS가 mutant 실행의 prerequisite다.
- control은 exit 0 + completion marker + failures 0 + tests > 0을 모두 만족한다.
- mutant kill은 expected nonzero exit + declared exact failure set/reason을 만족한다.
- unrelated failure, timeout, signal, harness error, unviable는 별도 status다.
- curated와 generated schema/denominator/status를 절대 합산하지 않는다.

## 작업 플랜

1. `tools/verification/run_mutations.py`
   - source copy/worktree lifecycle, exclusive campaign identity, dedicated `CARGO_TARGET_DIR`.
   - preflight baseline cache와 strict exit/failure/completion classifier.
   - source-before/after digest 및 abrupt-exit cleanup protocol.
2. `tools/verification/mutations.json`
   - expected exact/co-failure set, command/env/profile identity를 명시한다.
3. 신규 `tools/verification/run_generated_mutants.py`
   - cargo-mutants raw output을 보존하고 caught/missed/unviable/timeout denominator를 구조화한다.
4. producer-owned `tools/verification/mutation-gate.json`을 추가해 V01 envelope에 필요한
   command/status/artifact contract를 선언한다. shared receipt/inventory/Justfile/CI는 직접
   수정하지 않고 V01 owner가 한 번에 등록한다.
5. `tools/verification/tests/test_run_mutations.py`와 신규 generated runner tests
   - process/isolation/classification negative matrix를 추가한다.

## 산출 artifact

- `receipt.mutations.json`
- `receipt.generated-mutations.json`
- `mutation-baseline-manifest.json`
- cargo-mutants raw JSON/log
- source-before/source-after, isolation-root, target-dir identity
- equivalent 판정의 mutant ID, source reachability, reviewer

## negative fixture

- exit 101 + parsed failure 0이 CONTROL_GREEN이 되지 않음.
- exit 0 + expected failure text가 KILLED가 되지 않음.
- expected + unrelated failure가 KILLED가 되지 않음.
- baseline red/zero-test/partial completion.
- 두 campaign과 normal cargo command 병렬 실행.
- SIGKILL 뒤 원본 digest 변화.
- generated PASS지만 raw denominator/digest 없음.
- curated count를 generated result로 복사.

## DoD

- `SEP21-V02-A01`: campaign process는 원본 source를 쓰지 않는다.
- `SEP21-V02-A02`: 각 outcome에 exit/signal/command/env/profile/test count/baseline digest가 있다.
- `SEP21-V02-A03`: concurrent/abrupt-exit test에서 source와 artifact가 오염되지 않는다.
- `SEP21-V02-A04`: current-source generated sweep을 실제 실행해 모든 survivor를
  CAUGHT/MISSED/UNVIABLE/TIMEOUT/EQUIVALENT로 보존한다.
- `SEP21-V02-A05`: equivalent는 자동 PASS가 아니며 source reachability와 reviewer가 필요하다.
- `SEP21-V02-A06`: R01은 curated 100/100을 generated score로 표현할 수 없다.

## 금지되는 임시방편

- shared checkout에 lock만 추가하고 mutant source를 계속 노출.
- output substring만으로 exit truth 대체.
- unrelated failures allow-all.
- generated survivor를 수기 표 한 줄로만 waive.

## Producer evidence (2026-09-21)

- integrated base: `main@ba643a130dfdaf091099bb11099962198d0e5981` + producer-owned V02
  changes. Ticket 상태는 full generated sweep가 없고 실제 subset에 survivor/unviable가 있으므로
  `IMPLEMENTED_UNQUALIFIED`를 유지한다.
- producer manifest: `tools/verification/mutation-gate.json` v1,
  `sha256:c861ef1a959715f1de71f5442204b4b45aae49bd40ddb532b21f0164cec2b8bd`.
- curated inventory: 105 entries, exact failure-set policy
  `primary_plus_declared_cofailures_exact`; H02 authority 이동으로 실제 drift한 6개만 refresh했고
  current integrated source에서 anchor 105/105가 정확히 한 번 일치한다. Inventory digest는
  `00255106d5a238459d283558b420b2933822d68a8d4c7917c08c11c57371a52d`.
- runner tests: `uv run pytest -q tools/verification/tests` — 55 passed.
- lint/format: `uv run ruff check tools/verification` 및
  `uv run ruff format --check tools/verification` — passed.
- isolation negatives: exact git-visible path만 snapshot하고 ignored/unbound file은 복사하지 않음,
  symlink 자체 보존, copy 중 path-set race fail-closed, concurrent campaign root/target 분리, normal
  target sentinel 보존, isolated child `SIGKILL` 뒤 original digest 보존이 모두 passed.
- manifest safety negatives: absolute/`..`/비정규 file path, parent symlink escape, manifest의
  `CARGO_TARGET_DIR`/runner isolation env override를 모두 거부한다.
- classifier negatives: exit 0 + failure text, nonzero control, signal, timeout, zero tests, partial
  completion, expected + unrelated failure, red baseline이 모두 green으로 승격되지 않는다.
- generated parser는 cargo-mutants 27.0의 real-shape fixture와 실제 local v27 artifact에서
  `mutants.json` planned names, category txt names, `outcomes.json` executed names/summary를 모두
  exact-set 비교한다. Equal-count identity mismatch, duplicate identity, category-summary mismatch,
  red baseline, zero denominator, partial outcome, all-caught/nonzero-process가 모두 fail-closed다.
  Equivalent는 mutant ID, reachability evidence, reviewer가 있어도 자동 PASS가 아니다.
- prescribed integrated focused campaign (6 selected): baseline 4/4 `PASS`, mutation 6/6 `KILLED`,
  source before/snapshot/after
  `6192de0fdd022f0a295f142f15b5204dbf9b17da23f46e79bbd9a0322cb48a2b`, envelope problems 0.
  Full curated inventory는 `NOT_RUN`이다. 이 focused result를 full curated score로 확대하지 않는다.
- refreshed H02-authority severe campaign (6 selected): baseline 4/4 `PASS`, mutation 6/6
  `KILLED`, 같은 source identity, envelope problems 0. 중앙 authority mutation이 여러 oracle을
  깨뜨리는 경우 exact `expect_cofailures`만 선언했다.
- 첫 integrated focused 시도는 mutation 6/6이 `KILLED`였지만 V03 동시 edit로 source digest가
  변해 receipt 전체가 `FAIL`했다. 이 artifact는 non-final이며 mixed-source 증거로 재사용하지
  않았다.
- real generated subset (`taskmesh-rayon`, cargo-mutants 27.0.0, jobs=1): planned/executed/
  categorized 10/10/10, `caught=4`, `missed=1`, `unviable=5`, `timeout=0`, `equivalent=0`, process
  exit 2, signal null, timeout false, parse error null, source before/snapshot/after
  `d163fc0538a20064f541eeed2dd209f35b1de7037143ff6bccc35f654dac3328`, envelope problems 0.
  Receipt digest는 `700eba79928dc2fe66322226368280778e58fd83f1b38d710daa8fcf325837a6`이며
  semantic status는 `FAIL`이다.
- full workspace generated cargo-mutants sweep: `NOT_RUN`. Package subset/runner 검증과 full
  generated denominator를 분리하며 R01 전 실제 full sweep와 survivor/unviable 처리가 필요하다.

### V01 shared registration request

- producer id/version: `mutation-campaign` / `1`.
- envelope schema: `tools/qualification/evidence-envelope-v1.schema.json`.
- config: `tools/verification/mutation-gate.json`과 그 digest.
- curated command: `uv run python tools/verification/run_mutations.py`; required summary
  `receipt.mutations.json`, baseline manifest `mutation-baseline-manifest.json`, raw stdout/stderr logs.
- generated command: `uv run python tools/verification/run_generated_mutants.py --jobs 1`; required summary
  `receipt.generated-mutations.json`, raw `mutants.out/**`, runner stdout/stderr.
- source fields: HEAD, dirty flag, current-source tree digest, snapshot digest, original after digest,
  isolation root, dedicated target directory.
- command fields: argv, recorded environment, profile, command digest; tool fields: exact Python/Cargo/
  rustc and cargo-mutants identity as applicable.
- result rule: curated PASS requires every selected entry `KILLED` or `CONTROL_GREEN`, every command
  baseline PASS, selected/executed equality, and unchanged original source. Generated PASS requires a
  successful raw baseline, complete planned/categorized/outcome denominator, and zero missed,
  unviable, timeout, and equivalent outcomes.
- shared files requested from V01 owner only: Justfile recipe, gate inventory record, receipt linkage,
  workflow execution/upload. V02 does not modify those files.

## 검증 명령 후보

```sh
uv run pytest -q tools/verification/tests tools/qualification/tests/test_receipt.py
just mutants-critical --require-clean
# generated runner의 exact command는 ticket 구현 시 manifest에 고정
```
