# TM16-039 — PM의 중복 YAML target key가 출력 검증 대상을 조용히 제거한다

- Severity: P3
- Status: OPEN / actual CLI false-green reproduction
- Lane: Q — PM configuration validation
- Baseline: `9ae9547216c70458f54f37368a67661321060886`, 2026-09-16

## 근거

- `tools/pm/pm.py:53-78`: `yaml.safe_load`가 중복 mapping key를 마지막 값으로 덮어쓴 다음, 이미 축소된 dict만 검증한다.
- `tools/pm/pm.py:145-169`: sync/lint가 축소된 target 집합을 순회한다. 사라진 첫 target은 missing/drift 검사 자체를 받지 않는다.
- [Retained duplicate-key fixture](evidence/pm-duplicate-target/targets.yaml): 동일 `demo` 이름으로 `MISSING.md`, `KEEP.md` 두 output을 선언한다. KEEP만 정상 내용으로 존재한다.

## Trigger / 관찰

```sh
PYTHONDONTWRITEBYTECODE=1 .venv/bin/python tools/pm/pm.py \
  --targets docs/bugbash/sep-16-general/tickets/evidence/pm-duplicate-target/targets.yaml \
  --pm-dir docs/bugbash/sep-16-general/tickets/evidence/pm-duplicate-target \
  --repo-root docs/bugbash/sep-16-general/tickets/evidence/pm-duplicate-target lint
test ! -e docs/bugbash/sep-16-general/tickets/evidence/pm-duplicate-target/MISSING.md
```

두 command 모두 exit 0. 실제 lint는 `pm lint OK: 1 target(s) up to date`를 출력한다. 첫 선언의 output은 없는데 invalid duplicate-name configuration도, 누락된 output도 보고하지 않는다. Main은 retained fixture에 lint만 실행했으며 실제 repo의 generated instructions를 sync하지 않았다. 별도 agent의 임시 fixture sync에서도 첫 output이 생성되지 않았다.

## 원인 / 영향 / 범위

복사/merge로 target key가 중복되면 YAML decode 단계에서 inventory 일부가 소실된다. downstream의 올바른 drift check로는 복구할 수 없다. 실제 기본 targets.yaml에 현재 중복이 있다는 주장은 아니며 malformed configuration을 허용하는 P3 도구 결함이다.

현재 target drift(TM16-025), nested template lookup(TM16-035)과 다른 원인이다. output path가 같고 target 이름은 다른 경우의 write collision도 별도 설계 검토 대상이며 이번 재현과 혼동하지 않는다.

## 보완 계획

- YAML mapping을 만드는 단계에서 중복 key를 감지하고 offending key/line을 포함한 PmError로 거부한다. dict 생성 이후 set 비교로는 이미 유실된 선언을 찾을 수 없다.
- top-level `targets` 및 개별 target의 `output/template/sections` 중복도 검사한다.
- parser/config validation을 sync/lint/status/preview 공통 경로에 둔다. sync의 어떤 output도 쓰기 전에 전체 설정을 검증한다.

## Acceptance / 회귀 검증

- retained duplicate target fixture가 lint/sync 모두 nonzero이며 sync는 출력 파일을 생성/수정하지 않는다.
- 동일한 값을 가진 duplicate key도 명시적 오류다.
- 유일한 이름의 정상 target은 기존 render/lint 결과를 보존한다. output missing/drift negative control도 계속 실패한다.
