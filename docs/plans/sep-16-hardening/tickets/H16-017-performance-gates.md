# H16-017 — Metric schema·실패 전파·baseline qualification

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P1 (원본 bug severity 변경 아님)
- 책임 역할: B — Benchmark owner (실제 assignee 미지정)
- 선행 완료: [H16-016](H16-016-workload-and-latency-model.md), [H16-019](H16-019-dependency-and-toolchain.md)
- 원본 finding: [TM16-019](../../../bugbash/sep-16-general/tickets/TM16-019-benchmark-workflow-swallowed-failures.md), [TM16-020](../../../bugbash/sep-16-general/tickets/TM16-020-iai-proof-config-and-baseline-mismatch.md), [TM16-038](../../../bugbash/sep-16-general/tickets/TM16-038-allocation-gate-malformed-number-passes.md)
- 배타적 write lease: `bench`, `gate-tools`, `ci`; [적용 순서](EXECUTION.md) 준수

## 목적

명령·parser·baseline 실패가 PASS로 변환되지 않는 단일 performance qualification 경로를 만든다.

## 변경 범위

- 기존: [tools/bench-gate.sh](../../../../tools/bench-gate.sh)
- 기존: [tools/bench-iai.sh](../../../../tools/bench-iai.sh)
- 기존: [.github/workflows/bench.yml](../../../../.github/workflows/bench.yml)
- 기존: [Justfile](../../../../Justfile)
- 제안 경로: `tools/tests/test_bench_gates.py` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음
- 제안 경로: `tools/bench/baseline-schema.json` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [x] B는 metric/baseline schema와 gate 구현을 소유하고 C는 Justfile·workflow 패치를 적용한다. 공유 파일 병렬 수정을 금지한다.
  → 단일 작업자 — 직렬 적용
- [x] shell pipeline 전체에 실패 전파를 적용하고 producer 실패·tee 실패·parser 실패를 구별한다. remote baseline의 명시적 not-found와 인증·network·format 실패를 구분한다.
  → fixture: `tools/bench/tests/test_history_branch.py`(ls-remote 0/2/128), `test_validate_bencher_output.py`(partial capture)
- [x] allocation 수치와 MAX_ALLOCS_PER_OP threshold 모두 전체 문자열을 엄격히 parse하고 malformed/NaN/inf/negative를 거절한다. metric 중복/누락, 단위·required metrics·schema version도 검증한다.
- [x] IAI threshold·comparison policy를 단일 config로 만들고 local/CI가 같은 config를 읽게 한다. benchmark id, harness/helper schema, config, lock digest, toolchain, target, runner, Valgrind compatibility fingerprint를 저장한다.
- [x] 초기 baseline 생성은 BASELINE_CREATED / NOT_QUALIFIED로 기록한다. incompatible baseline을 자동 PASS나 silent reset으로 바꾸지 않는다.
- [x] baseline과 candidate의 source identity를 각각 보존하고 Linux 지원 환경에서 정상 control·허용 경계·5% 초과 악화 negative를 실제 gate에 통과시킨다.
  → Linux 실행은 BLOCKED(EXCEPTIONS H16-017-A05); fingerprint·source identity 보존은 구현·테스트됨
- [x] H16-015/016 측정 모델 변경 전후 수치를 동일 population의 성능 추이로 비교하지 않는다. baseline 재승인과 artifact retention을 명시한다.
  → `MEASUREMENT_SCHEMA = 2`로 이전 baseline 단절; baseline은 fingerprint별 CI cache, 재승인은 첫 Linux run(BASELINE_CREATED)

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local); Linux 실행은 BLOCKED.**

- `tools/bench/allocation_gate.py`: 앵커된 grammar로 metric·threshold 모두 strict parse; marker 정확히
  1개; unknown field 거절; `completed==attempted`·`final_inflight==0`·schema 일치 검증; producer
  nonzero는 자기 exit code로 전파. `bench-gate.sh`는 얇은 front door. set-but-empty threshold도 거절.
- `tools/bench-iai.sh`: `perf-gate.json`에서 threshold/runner version 읽음(env가 다르면 config 우선 +
  경고), baseline fingerprint(bench source·deps·config·schema·runner·valgrind·rustc) 저장·비교,
  불일치 baseline은 폐기. `status=QUALIFIED|BASELINE_CREATED` 출력, `$GITHUB_OUTPUT` 기록.
- `bench.yml`: `defaults.run.shell: bash` + `pipefail`; `ls-remote` exit 2(missing)만 skip, 그 외는 실패;
  bencher output validator; main bootstrap step; cache key에서 prefix restore-keys 제거.
- `ci.yml`: `just` recipe만 호출(TM16-006).

Regression: `tools/bench/tests/test_allocation_gate.py` (35) — TM16-038 표의 모든 false-green row가
nonzero.

## 검증 / 완료 조건

- [x] `H16-017-A01` producer nonzero/malformed/missing metric 각각 nonzero + 원인
- [x] `H16-017-A02` `3.0.0`/NaN/inf/-1/duplicate/invalid threshold 거절; below/equal/above control
- [x] `H16-017-A03` local/CI 동일 config; fingerprint mismatch → baseline 폐기 + BASELINE_CREATED
      (PATH shim end-to-end test `tools/bench/tests/test_iai_gate.py`; CI cache key = script의
      fingerprint 그 자체)
- [x] `H16-017-A04` baseline 없는 최초 실행은 NOT_QUALIFIED 명시; stamp는 bench 성공 후에만 기록.
      감사(A3-P0-1): `iai-callgrind-runner --version`은 존재하지 않아 script가 무진단으로 죽었다 →
      `tools/bench/iai_gate.py`가 `cargo install --list`/runner self-report로 버전을 읽고 모든 exit가
      진단을 낸다. allocation gate threshold는 측정값 3.0과 같다(4가 아님).
- [ ] `H16-017-A05` Linux 실제 >5% injection FAIL / control PASS는 **미실행** (macOS) → BLOCKED
  → 예외 대장: [EXCEPTIONS.md](EXCEPTIONS.md)

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
just bench-gate
just bench-iai
```

## 호환성 / 실패 모드

- 없는 baseline과 고장 난 baseline fetch를 같은 fallback으로 처리하면 false green이 재발한다.
- 성능 한계값은 노이즈·환경을 포함한 승인 정책이며 단일 laptop 수치로 설정하지 않는다.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-18.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)
