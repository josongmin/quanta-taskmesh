# SEP22-T05 Python floor

- 상태: PLANNED
- finding: TO-05
- priority: P0
- write lane: `python-compat`
- 선행 티켓: 없음
- 소유 경로: `pyproject.toml`, `uv.lock`, `tools/verification/run_generated_mutants.py`, `tools/verification/tests/test_run_generated_mutants.py`

## 목적

선언된 Python 지원 범위와 실제 표준 라이브러리 사용을 일치시킨다. 기본 결정은 현재 구현을
사실대로 반영해 Python floor를 3.11로 올리는 것이며, 3.9 지원 요구가 확인될 때만 explicit
backport dependency 경로로 재설계한다.

## RCA

프로젝트 metadata와 Ruff는 Python 3.9를 지원한다고 선언하지만 generated-mutants runner는
3.11에 추가된 stdlib `tomllib`를 unconditional import한다. 이 때문에 3.9/3.10 환경은 테스트
수집 또는 도구 시작 단계에서 실패하며 declared support가 거짓이다.

## 확정 근거

- metadata는 `requires-python = ">=3.9"`, Ruff는 `py39`를 선언한다: `pyproject.toml:1-19`.
- runner는 fallback 없이 `tomllib`를 import한다:
  `tools/verification/run_generated_mutants.py:1-18`.
- 테스트는 module import 시 runner를 즉시 execute하므로 낮은 Python에서는 collection 전에
  실패한다: `tools/verification/tests/test_run_generated_mutants.py:13-20`.

## 목표 불변식

- metadata, lockfile, lint target, CI/interpreter inventory가 하나의 Python floor를 말한다.
- 지원된 최저 버전에서 runner import와 focused tests가 실제 실행된다.
- 지원하지 않는 버전은 resolver 단계에서 명확히 거부된다.
- compatibility를 위해 import error를 skip하거나 mutation proof를 NOT_RUN으로 만들지 않는다.

## 구현 플랜

1. workflows, setup scripts, devcontainer/toolchain inventory에서 실제 Python 버전을 검색해 3.11
   미만 consumer가 있는지 기록한다.
2. 3.9/3.10 의무 consumer가 없으면 `requires-python`을 `>=3.11`, Ruff target을 `py311`로 맞춘다.
3. `uv lock`으로 lock metadata를 재생성하고 diff에서 package graph가 불필요하게 변하지 않았는지
   확인한다.
4. 현재 interpreter와 별도로 Python 3.11 환경에서 runner import 및 focused tests를 실행한다.
5. resolver negative로 Python 3.10이 새 metadata를 지원 환경으로 오인하지 않는지 확인한다.
6. inventory가 3.9 지원을 요구하면 이 변경을 중단한다. 그 경우 `[project]` dependency에
   conditional `tomli; python_version < '3.11'`를 명시하고 guarded import를 사용하는 별도 계획을
   승인받는다.
7. 어느 경로든 `uv.lock`, declared floor, lint target이 atomic하게 변경되어야 한다.

## 테스트와 intentional negative

- positive: declared minimum Python에서 runner import와 generated-mutants tests가 통과한다.
- negative: unsupported Python은 resolver가 명시적으로 거부하며 runtime `ModuleNotFoundError`까지
  진행하지 않는다.
- lock negative: `uv lock --check`가 metadata/lock 불일치를 잡아야 한다.
- mutation negative: unconditional `tomllib`를 유지한 채 floor만 3.9로 되돌리면 compatibility
  check가 실패해야 한다.

## DoD

- SEP22-T05-A01 repo inventory 근거와 선택한 floor 또는 backport 결정이 handoff에 기록된다.
- SEP22-T05-A02 metadata, Ruff target, lockfile, import 경계가 동일한 지원 범위를 표현한다.
- SEP22-T05-A03 최저 지원 버전 positive와 바로 아래 버전 negative가 실제 interpreter로 실행된다.
- SEP22-T05-A04 전체 Python suite에서 skip/NOT_RUN 증가 없이 exact test count가 기록된다.

## 금지되는 임시방편

- `try/except ImportError` 후 기능을 skip하거나 빈 결과를 반환하는 것.
- lockfile을 갱신하지 않은 metadata-only 변경.
- CI만 최신 Python으로 바꾸고 public `requires-python`을 그대로 두는 것.
- 확인되지 않은 3.9 consumer를 추정해 dependency를 영구 추가하는 것.

## 검증 명령

```sh
rg -n "python-version|requires-python|target-version|3\\.(9|10|11|12|13)" .github tools pyproject.toml uv.lock
uv lock --check
uv run python --version
uv run pytest tools/verification/tests/test_run_generated_mutants.py -q
uv run pytest tools -q --durations=40
uv run ruff check tools pyproject.toml
```

최저 지원 interpreter 검증은 설치된 toolchain의 명시적 실행 명령과 버전 출력까지 receipt에 남긴다.

## Stop/reopen 조건

- inventory에서 production 또는 CI가 Python 3.9/3.10을 요구하면 floor 인상을 중단하고 backport
  dependency 설계로 재개한다.
- lock 재생성이 관련 없는 대규모 dependency churn을 만들면 uv 버전과 lock procedure를 먼저
  고정한다.
- 최저 버전 실행 환경을 확보하지 못하면 static metadata 변경만으로 완료 처리하지 않는다.
