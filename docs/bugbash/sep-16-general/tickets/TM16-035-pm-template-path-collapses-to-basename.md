# TM16-035 — PM이 지정된 template 경로를 basename으로 바꿔 다른 template을 렌더링한다

- Severity: P3
- Status: OPEN / hermetic actual CLI reproduction
- Lane: Q — PM rendering correctness
- Baseline: `9ae9547216c70458f54f37368a67661321060886`, 2026-09-16

## 근거

- `tools/pm/pm.py:96-100`: `pm_dir / target.template`가 존재하는지 검사한다.
- `tools/pm/pm.py:103,108`: loader root는 `pm_dir/templates`지만 조회할 때는 `Path(target.template).name`만 사용한다.
- [Retained fixture](evidence/pm-nested-template/targets.yaml): `templates/nested/demo.j2`를 지정하며 root/nested template에 서로 다른 본문을 둔다.

## Trigger / 관찰

실제 CLI preview는 지정된 `RIGHT TEMPLATE` 대신 `templates/demo.j2`의 `WRONG TEMPLATE`을 출력한다. 이 잘못 렌더링된 fixture output에 대해 실제 CLI lint는 `OK: 1 target(s) up to date`, exit 0을 반환한다.

```sh
uv run python tools/pm/pm.py \
  --targets docs/bugbash/sep-16-general/tickets/evidence/pm-nested-template/targets.yaml \
  --pm-dir docs/bugbash/sep-16-general/tickets/evidence/pm-nested-template \
  --repo-root docs/bugbash/sep-16-general/tickets/evidence/pm-nested-template \
  preview --target nested
```

같은 인자에서 `preview --target nested`를 `lint`로 바꾸면 false-green을 재현한다. 실제 저장소 target에 sync를 실행하지 않았다.

## 원인 / 영향 / 범위

검증한 file identity와 렌더링한 file identity가 다르다. 같은 basename의 template이 있으면 잘못된 내용을 쓰고 lint도 같은 오류로 green이 된다. 현재 기본 5개 target은 flat template 경로이므로 이 버그가 기존 generated AGENTS를 손상시켰다는 증거는 없다. nested/custom target 설정 범위이며 TM16-025의 실제 target drift와 독립이다.

## 보완 계획

- validated template 경로를 loader root 기준의 relative path로 사용하여 nested identity를 보존한다.
- 지원하는 경로 root/absolute/parent traversal 정책을 명시하고 loader가 같은 파일을 읽는지 검증한다.
- TemplateNotFound/render errors도 target과 full path를 포함한 PM error로 반환한다.

## Acceptance / 회귀 검증

- 동일 basename의 root/nested template에서 지정된 nested content를 렌더링한다.
- 잘못된 root-template output에 대해 lint가 nonzero를 반환한다.
- nested-only template도 동작하고 flat default targets와 template include 동작을 보존한다.
