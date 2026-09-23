# SEP22-T05 Python floor

- 상태: LOCALLY_VERIFIED
- finding: TO-05
- priority: P0
- write lane: `python-compat`
- 선행 티켓: 없음
- 소유 경로: `pyproject.toml`, `uv.lock`, `tools/verification/run_generated_mutants.py`, `tools/verification/tests/test_run_generated_mutants.py`

## 목적

선언된 Python 3.9 지원 범위와 실제 import를 일치시킨다. 현재 소스는 3.9 경로에 explicit
`tomli` dependency와 guarded import를 제공하므로 floor 인상은 필요하지 않다.

## RCA

2026-09-22 baseline은 Python 3.9 지원을 선언하면서 generated-mutants runner가 3.11의
stdlib `tomllib`를 unconditional import했다. 현재 소스는 conditional `tomli` backport와
guarded import로 이 불일치를 해소했다.

## 확정 근거

- metadata는 `requires-python = ">=3.9"`, Ruff는 `py39`를 선언한다: `pyproject.toml:1-19`.
- baseline runner는 fallback 없이 `tomllib`를 import했다. 현재 runner는 guarded import다:
  `tools/verification/run_generated_mutants.py:1-18`.
- 테스트는 module import 시 runner를 즉시 execute하므로 낮은 Python에서는 collection 전에
  실패한다: `tools/verification/tests/test_run_generated_mutants.py:13-20`.

## 목표 불변식

- metadata, lockfile, lint target, CI/interpreter inventory가 하나의 Python floor를 말한다.
- 지원된 최저 버전에서 runner import와 focused tests가 실제 실행된다.
- 지원하지 않는 버전은 resolver 단계에서 명확히 거부된다.
- compatibility를 위해 import error를 skip하거나 mutation proof를 NOT_RUN으로 만들지 않는다.

## 구현 플랜

1. 선언된 3.9 floor와 `tools/arch/tests`의 3.9 import 경계를 보존한다.
2. Python `<3.11`에만 `tomli`를 설치하고 runner는 guarded import로 stdlib/backport를 선택한다.
3. `uv.lock`과 Ruff target을 metadata에 맞게 유지하고 `uv lock --check`로 검증한다.
4. 실제 Python 3.9에서 focused 및 전체 도구 suite를 실행하고 3.8 resolver 거부를 확인한다.

## 테스트와 intentional negative

- positive: declared minimum Python에서 runner import와 generated-mutants tests가 통과한다.
- negative: Python 3.8은 resolver가 명시적으로 거부하며 runtime `ModuleNotFoundError`까지
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

## 2026-09-23 현재 소스 재검증

- `main@146233665942d75b73e2b724f781be7e105fd7c4`의 `pyproject.toml`/`uv.lock`은
  `>=3.9`, Ruff `py39`, Python `<3.11`의 `tomli>=2`를 일치시킨다.
  `run_generated_mutants.py`는 3.11+의 `tomllib`와 3.9/3.10의 `tomli`를 선택한다.
- `uv lock --check` exit 0. `uv run --isolated --python 3.9 pytest
  tools/verification/tests/test_run_generated_mutants.py -q`는 Python 3.9.25에서 27/27 통과.
  같은 focused rail은 Python 3.10.19에서 28/28, 3.11에서 28/28 통과했다.
  동일 interpreter의 `uv run --isolated --python 3.9 pytest tools -q --durations=10`은
  564/564 통과, skip 0, exit 0. 3.8.20의 `uv run --isolated --python 3.8 python --version`은
  `requires-python >=3.9` 위반으로 exit 2 거부한다.
- 이는 dirty shared worktree의 로컬 proof다. W3 exact-source receipt가 없으므로 캠페인
  qualification으로 승격하지 않는다. 전체 suite duration은 동시 호스트 부하 때문에 성능 증거가 아니다.
