# H16-020 — PM validated render plan·safe apply

- 상태: IMPLEMENTED 후 RETIRED — 실제 생성 target이 0임을 확인하고 2026-09-23 제거
- 실행 우선순위: P2 (원본 bug severity 변경 아님)
- 책임 역할: Q — PM/문서 owner (실제 assignee 미지정)
- 선행 완료: [H16-001](H16-001-contracts-and-compatibility.md)
- 원본 finding: [TM16-025](../../../bugbash/sep-16-general/tickets/TM16-025-prompt-manager-real-target-drift.md), [TM16-035](../../../bugbash/sep-16-general/tickets/TM16-035-pm-template-path-collapses-to-basename.md), [TM16-039](../../../bugbash/sep-16-general/tickets/TM16-039-pm-duplicate-target-key-hides-output.md)
- 배타적 write lease: `pm`; [적용 순서](EXECUTION.md) 준수

## 목적

PM parse/validate/render-plan/apply를 분리해 lint가 실제 쓰일 파일을 정확히 검증하도록 한다.

## 변경 범위

- 2026-09-23 현재 생성 target이 하나도 없고 `AGENTS.md`는 사용자 소유다.
- 미래 사용 가능성만 위해 renderer·fixture·Jinja2·필수 gate를 유지하지 않는다.

## 구현 액션

- [x] duplicate YAML mapping key를 loader 단계에서 reject하고 unknown target/config fields를 명시적 schema 정책으로 처리한다. dictionary collapse 이후 검사는 금지한다.
- [x] target identity는 basename이 아닌 검증된 relative path다. nested path/template mapping/중복 normalized output 충돌을 검증한다.
- [x] lint와 apply가 동일 ValidatedRenderPlan을 소비하도록 한다. render-plan에는 exact target/template/source digest/expected output digest를 포함한다.
- [x] lint는 읽기 전용으로 만들고 fixture 및 실제 target의 전후 digest로 무변경을 검사한다. normalized path containment와 symlink 정책을 D11에서 확정한다; 기존 상태를 보안 사고로 단정하지 않는다.
- [x] 현재 AGENTS.md 등 사용자 소유 설정과 생성 target drift를 비교해 migration diff를 제시한다. 실제 target overwrite는 명시적 승인·복구본·원자적 replace 절차를 요구한다.
- [x] apply의 다중 파일 부분 실패와 concurrent edit를 감지하고 변경 전 digest 불일치 시 중단한다. 롤백 가능한 per-file 결과를 남긴다.
- [x] 이전 제품 branding/없는 source 경로/잘못된 template 선택을 expected content assertion으로 잡는다.

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- `_StrictLoader`: mapping 구성 중 duplicate key 거절(line 번호 포함) — TM16-039 fixture가 lint/sync
  모두 exit 2, sync는 아무 파일도 쓰지 않음.
- template identity = loader root 기준 relative path(TM16-035); resolved path의 containment 검사
  (symlink escape 거절). 감사 fixture에서 `RIGHT TEMPLATE` 렌더, 기존 `WRONG.md`는 drift.
- `RenderPlan`(template path·section digest·rendered digest·output path)을 lint/sync가 공유.
- lint/status/preview는 read-only(전후 tree digest 동일 test). sync는 전체 plan 후 적용, hand-edit
  overwrite는 `--force` 없이는 refused, atomic replace, per-file 결과 보고, 한 target 실패 시 아무것도
  쓰지 않음.
- **실제 target 결정(D11)**: PM 소스는 다른 프로젝트(logq)의 것이었다. 렌더하면 잘못된 agent 지시가
  생성되므로 소스/템플릿을 제거하고 `targets.yaml`을 빈 집합으로 **명시**했다. `AGENTS.md`는 사용자
  소유이며 PM target이 아니다. 실제 target overwrite는 수행하지 않았다.

Historical regression at implementation time: `tools/pm/tests/test_render_plan.py` (21) + 기존 8.

2026-09-23 정리: 위 regression은 구현 당시 증거다. 실제 owner가 0인 상태가 지속되어 PM 코드와
vacuous `pm-lint` gate를 제거했다. 생성 대상이 생기면 그 대상과 함께 다시 설계한다.

## 검증 / 완료 조건

- [x] `H16-020-A01` duplicate key·nested path collision·unknown key negative가 실패
- [x] `H16-020-A02` nested template expected content; basename 혼동 재현이 drift로 실패
- [x] `H16-020-A03` lint 전후 bytes/mode/symlink 변경 0
- [x] `H16-020-A04` 기존 target이 다르면 sync refused; AGENTS 유지
- [x] `H16-020-A05` fixture apply 성공·중간 실패 시 무기록; 실제 target 변경 미실행

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
just py-test
just py-lint
```

## 호환성 / 실패 모드

- PM 정합성 수정이 repo instruction 파일을 자동 재생성할 권한은 아니다.
- 경로 containment의 허용 범위를 정하지 않고 무조건 repo 밖 출력 금지로 바꾸면 의도된 사용을 깨뜨릴 수 있다.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-19.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)
