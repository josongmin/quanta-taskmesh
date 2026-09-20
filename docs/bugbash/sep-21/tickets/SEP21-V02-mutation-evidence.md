# SEP21-V02 — Transactional mutation campaign authority

- 상태: PLANNED
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

## 검증 명령 후보

```sh
uv run pytest -q tools/verification/tests tools/qualification/tests/test_receipt.py
just mutants-critical --require-clean
# generated runner의 exact command는 ticket 구현 시 manifest에 고정
```
