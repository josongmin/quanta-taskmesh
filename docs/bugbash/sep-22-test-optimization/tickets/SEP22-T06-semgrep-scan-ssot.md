# SEP22-T06 Semgrep scan SSOT

- 상태: PLANNED
- finding: TO-06
- priority: P1
- write lane: `semgrep-proof`
- 선행 티켓: 없음
- 소유 경로: `tools/semgrep/tests/test_rules_fire.py`

## 목적

실제 `crates` target-set을 확인하는 두 테스트가 동일한 Semgrep process 결과를 공유하게 한다.
스캔 횟수만 줄이고 enrolled-path 및 every-crate-directory oracle은 그대로 유지한다.

## RCA

두 테스트가 같은 config, flags, target으로 `_semgrep_json`을 각각 호출한다. 반환 JSON의
`paths.scanned`만 소비하므로 한 module-scoped immutable receipt로 충분하다. 현재 구조는 비싼
실제 scan을 두 번 수행하면서 두 번째 실행이 추가 의미를 제공하지 않는다.

## 확정 근거

- `_semgrep_json`은 subprocess를 매 호출 새로 실행한다:
  `tools/semgrep/tests/test_rules_fire.py:332-342`.
- fixture trigger/clean scan은 별도 module-scoped fixture이며 목적과 target이 다르다:
  `tools/semgrep/tests/test_rules_fire.py:396-419`.
- 두 real-inventory 테스트는 동일한 command를 호출해 `paths.scanned`를 읽는다:
  `tools/semgrep/tests/test_rules_fire.py:459-488`.

## 목표 불변식

- real `crates` scan command는 module session에서 정확히 한 번 실행된다.
- `ENROLLED_PATHS` 누락 검사는 그대로 fail-closed다.
- 모든 crate `tests` directory가 scanned set과 교차하는지 검사하는 oracle도 그대로다.
- trigger/clean synthetic fixture scans는 real inventory receipt와 합치지 않는다.
- Semgrep 부재의 CI hard failure 정책은 변하지 않는다.

## 구현 플랜

1. module-scoped `real_crates_scan` fixture를 만들고 기존 exact command를 한 번 호출한다.
2. fixture는 mutable raw JSON 대신 normalized `frozenset[str]` scanned paths를 반환한다.
3. 두 inventory 테스트가 fixture를 parameter로 받아 각각 enrolled paths와 directory coverage를
   독립적으로 assert하게 한다.
4. fixture 내부에서 empty scanned set을 즉시 실패시켜 두 consumer가 공통 전제 실패를 명확히
   공유하게 한다.
5. subprocess call counter를 monkeypatch하는 unit-level intentional negative를 추가하거나 기존
   module 실행에 verbose instrumentation을 사용해 호출 1회를 증명한다.
6. 변경 전후 동일 pytest node pair를 최소 5회 측정하고 median/worst를 기록한다.
7. rule YAML, ignore policy, target path, Semgrep flags는 변경하지 않는다.

## 테스트와 intentional negative

- positive: 두 consumer가 동일한 normalized scan receipt로 각자의 oracle을 통과한다.
- negative: enrolled file 하나를 receipt에서 제거하면 enrolled-path test가 실패한다.
- negative: crate test directory 전체를 receipt에서 제거하면 directory test가 실패한다.
- execution negative: 두 consumer 실행 중 real command call count가 2가 되면 실패한다.
- absence negative: CI에서 Semgrep executable이 없으면 skip이 아니라 collection failure다.

## DoD

- SEP22-T06-A01 identical real `crates` scan이 module당 1회만 실행됨이 자동 검증된다.
- SEP22-T06-A02 enrolled-path와 every-crate-directory oracle이 별도 테스트로 유지된다.
- SEP22-T06-A03 trigger/clean fixture scans 및 CI missing-binary policy에 semantic diff가 없다.
- SEP22-T06-A04 before/after median, worst, subprocess count, executed test count가 기록된다.

## 금지되는 임시방편

- 두 테스트를 하나로 합쳐 실패 attribution을 잃는 것.
- static glob만으로 Semgrep의 실제 `paths.scanned`를 대체하는 것.
- cache를 process 간 영구 저장해 source/config 변경 후 stale receipt를 재사용하는 것.
- rule/ignore 설정을 완화하거나 slow marker로 기본 gate에서 제외하는 것.

## 검증 명령

```sh
uv run pytest tools/semgrep/tests/test_rules_fire.py::test_real_integration_tests_are_inside_the_scanned_target_set tools/semgrep/tests/test_rules_fire.py::test_the_scan_reaches_every_crate_test_directory -q --durations=10
uv run pytest tools/semgrep/tests/test_rules_fire.py -q --durations=20
uv run ruff check tools/semgrep/tests/test_rules_fire.py
```

Semgrep version, command arguments, source SHA, run별 duration을 함께 기록한다.

## Stop/reopen 조건

- 두 테스트의 command line 또는 environment가 실제로 다르면 공유하지 말고 finding을 재평가한다.
- fixture 공유로 test isolation이나 path normalization 의미가 변하면 cache 범위를 축소하고 다시
  측정한다.
- 최적화 후 실제 scan receipt가 source/config 변경을 반영하지 않으면 즉시 되돌리고 cache key
  설계를 별도 티켓으로 연다.
