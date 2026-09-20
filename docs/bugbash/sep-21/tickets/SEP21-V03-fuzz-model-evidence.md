# SEP21-V03 — Search-budget and semantic-checkpoint authority

- 상태: PLANNED
- 우선순위: P1 release blocker
- 포함 finding: TM21-018, TM21-021
- 선행: V01
- write lane: `concurrency-proof`

## 목적

fuzz/model gate의 process success를 실제 탐색과 동일시하지 않는다. target별 semantic
checkpoint, model bound, seed, schedule count와 replay token을 receipt에 보존한다.

## RCA

- fuzz gate는 aggregate run summary만 보고 valid decode/state transition 실행을 모른다.
- tracked seed가 없어 wire-format oracle이 한 번도 실행되지 않을 수 있다.
- Loom/Shuttle recipe는 PASS만 남기고 bounds, explored count, random seed/replay를 잃는다.
- random failure를 clean environment에서 재생할 artifact contract가 없다.

## 확정 근거

- target별 positive count/checkpoint 없이 aggregate만 출력:
  `tools/fuzz/run.sh:41-71`.
- corpus ignored와 CI empty-corpus start: `.gitignore:5-10`,
  `.github/workflows/ci.yml:121-124`.
- decode 성공 때만 oracle이 실행되는 target:
  `fuzz/fuzz_targets/wire_formats.rs:19-52`.
- unseeded `check_random` 호출:
  `crates/taskmesh-engine/tests/shuttle_governance.rs:115,166,211,276,347,597,714`.
- model parameters를 보존하지 않는 recipes: `Justfile:118-144`.

## 목표 구조와 불변식

- required target별 runs, valid inputs, semantic checkpoints가 모두 양수여야 한다.
- 최소 valid/boundary seed corpus와 digest를 version-control한다.
- model receipt는 model ID, source, bound, requested/completed schedules, seed를 기록한다.
- Shuttle failure는 exact replay token/schedule을 보존한다.
- missing target/count/bound/replay는 PASS가 아니라 NOT_RUN/FAIL이다.

## 작업 플랜

1. `tools/fuzz/run.sh`, `check.sh`
   - duration > 0, target별 runs > 0, required target exact set을 강제한다.
   - aggregate 한 줄 대신 `fuzz-receipt.json`을 발행한다.
2. `fuzz/fuzz_targets/wire_formats.rs`
   - Snapshot/Topology/ClassPolicy valid-path checkpoint를 분리해 집계한다.
3. tracked `fuzz/corpus/<target>/`과 corpus manifest
   - minimal/boundary valid inputs를 보존한다.
4. 신규 `tools/modelcheck/run.py`
   - Loom builder bounds와 completed permutations, Shuttle seeded scheduler/count를 수집한다.
5. `loom_governance.rs`, `shuttle_governance.rs`
   - explicit model IDs와 replayable configuration을 사용한다.
6. producer-owned fuzz/model gate manifest를 추가한다. Justfile, inventory, CI, receipt의 shared
   wiring은 직접 수정하지 않고 V01 owner가 두 manifest를 한 번에 등록한다.

## negative fixture

- duration 0, target 누락, target runs 0이 aggregate로 숨겨짐.
- valid seed 제거, decode 전부 실패, checkpoint branch no-op.
- requested schedule보다 적게 실행하고 exit 0.
- Shuttle seed 누락/매 run 변경.
- Loom model ID/bound/completed count 누락.
- failure schedule artifact가 없거나 replay 결과가 다름.

## DoD

- `SEP21-V03-A01`: 모든 required fuzz target이 target별 positive witness를 가진다.
- `SEP21-V03-A02`: wire 3종과 core transition checkpoint가 최소 한 번 실행된다.
- `SEP21-V03-A03`: corpus/source/tool/target digest가 V01 envelope에 결합된다.
- `SEP21-V03-A04`: Loom/Shuttle PASS가 실제 탐색 수와 bounds를 구조화해 보고한다.
- `SEP21-V03-A05`: failure artifact 하나로 clean environment에서 동일 failure를 재생한다.
- `SEP21-V03-A06`: no-op harness와 zero-schedule model이 CLEAN/PASS를 얻지 못한다.

## 금지되는 임시방편

- wall-clock duration만 늘림.
- total runs 하나로 target별 0을 숨김.
- log line만 receipt에 복사.
- random seed를 출력하지만 replay하지 않음.

## 검증 명령 후보

```sh
just fuzz-check
FUZZ_SECONDS=60 just fuzz
just loom
just shuttle
```
