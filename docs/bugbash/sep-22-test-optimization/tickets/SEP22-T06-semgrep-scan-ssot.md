# SEP22-T06 Semgrep scan SSOT

- 상태: IMPLEMENTED_UNQUALIFIED
- finding: TO-06
- priority: P1
- write lane: `semgrep-proof`
- 선행 티켓: 없음
- 소유 경로: `tools/semgrep/tests/test_rules_fire.py`

## 목적

실제 `crates` scan을 canonical gate 한 곳에서 실행하고, 테스트는 target-set 정책의 독립적인
negative oracle을 검증한다. 중복 real-tree subprocess를 되살리지 않는다.

## RCA

2026-09-22 baseline에서는 두 테스트가 같은 config, flags, target으로 `_semgrep_json`을
각각 호출했다. 현재 소스는 그 두 real-tree subprocess 호출을 제거하고, canonical
`tools/semgrep/check.py`가 실제 source scan을 소유한다. Synthetic rule trigger/clean scan은
다른 불변식을 검사하므로 유지한다.

## 확정 근거

- `_semgrep_json`은 synthetic fixture를 검사할 때만 subprocess를 실행한다:
  `tools/semgrep/tests/test_rules_fire.py:364-375,398-425`.
- fixture trigger/clean scan은 별도 module-scoped fixture이며 목적과 target이 다르다:
  `tools/semgrep/tests/test_rules_fire.py:396-419`.
- baseline의 두 real-inventory 테스트는 현재 소스에 없다. 동일한 target-set 불변식은
  `tools/semgrep/check.py:39-55,107-133`의 production gate와 test-local negative에서 검증한다.

## 목표 불변식

- real `crates` scan은 `just semgrep`의 `check.py` 한 곳에서 실행된다.
- `ENROLLED_PATHS` 누락 검사는 그대로 fail-closed다.
- 모든 crate `tests` directory가 scanned set과 교차하는지 검사하는 oracle도 그대로다.
- trigger/clean synthetic fixture scans는 real inventory receipt와 합치지 않는다.
- Semgrep 부재의 CI hard failure 정책은 변하지 않는다.

## 구현 플랜

1. Synthetic trigger/clean scan을 실제 source scan과 합치지 않는다.
2. 실제 source scan은 canonical `just semgrep` 한 곳에서 실행하고 empty/missing enrollment,
   crate tree 누락은 `target_set_problems`가 fail-closed로 거부하게 한다.
3. 동일 source/host 조건의 이전 timing을 확보하지 못하면 절감률을 주장하지 않는다.

## 테스트와 intentional negative

- positive: gate는 실제 scanned-path receipt로 enrollment와 crate-directory를 검증한다.
- negative: empty/missing enrollment와 unscanned crate tree를 policy tests가 거부한다.
- execution negative: test module에 real-`crates` subprocess가 재도입되면 중복 scan으로 재개한다.
- absence negative: CI에서 Semgrep executable이 없으면 skip이 아니라 collection failure다.

## DoD

- SEP22-T06-A01 real `crates` scan은 canonical gate 한 곳에 있고 test module은 중복 실행하지 않는다.
- SEP22-T06-A02 enrolled-path와 every-crate-directory negative oracle이 별도 테스트로 유지된다.
- SEP22-T06-A03 trigger/clean fixture scans 및 CI missing-binary policy에 semantic diff가 없다.
- SEP22-T06-A04 동일 조건의 before/after median·worst를 확보하지 못하면 비용 절감을 미주장한다.

## 금지되는 임시방편

- gate의 target-set 검사와 test-local negative oracle을 합쳐 실패 attribution을 잃는 것.
- static glob만으로 Semgrep의 실제 `paths.scanned`를 대체하는 것.
- cache를 process 간 영구 저장해 source/config 변경 후 stale receipt를 재사용하는 것.
- rule/ignore 설정을 완화하거나 slow marker로 기본 gate에서 제외하는 것.

## 검증 명령

```sh
uv run pytest tools/semgrep/tests/test_rules_fire.py -q --durations=20
uv run ruff check tools/semgrep/tests/test_rules_fire.py
just semgrep
```

Semgrep version, command arguments, source SHA, run별 duration을 함께 기록한다.

## Stop/reopen 조건

- synthetic fixture와 real source scan의 command/target 의미가 다르면 계속 분리한다.
- 최적화 후 실제 scan receipt가 source/config 변경을 반영하지 않으면 즉시 되돌리고 cache key
  설계를 별도 티켓으로 연다.

## 2026-09-23 현재 소스 재검증

- baseline의 중복 real-`crates` 테스트 두 개는 현재 `test_rules_fire.py`에 없다. 현재 독립
  `target_set_problems` negative tests는 empty, missing enrollment, missed crate tree를
  검사하고, 실제 source scan은 `Justfile`의 `semgrep` front door → `tools/semgrep/check.py`
  한 곳에서 수행한다. Synthetic trigger/clean fixture scan 두 개는 의미가 달라 유지한다.
- `uv run pytest tools/semgrep/tests/test_rules_fire.py -q`: 47/47 통과.
  `just semgrep`: exit 0, pinned Semgrep 1.157.0, 133 files scanned, 0 findings.
- 현재 source에는 공유해야 할 중복 real scan이 없어 추가 fixture를 만들지 않는다.
  변경 전후 동일 조건의 timing/call-count 비교와 W3 receipt는 없어 비용 절감 수치는 주장하지 않는다.
