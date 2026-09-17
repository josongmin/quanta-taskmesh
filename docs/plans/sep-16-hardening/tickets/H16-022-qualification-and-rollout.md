# H16-022 — Exact-head qualification·rollout·rollback

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P1 (원본 bug severity 변경 아님)
- 책임 역할: I — 통합/계약 owner (실제 assignee 미지정)
- 선행 완료: [H16-021](H16-021-quality-and-doc-migration.md)
- 원본 finding: 통합·계약·품질 작업; 독립 신규 결함 수에 가산하지 않음
- 배타적 write lease: `qualification`; [적용 순서](EXECUTION.md) 준수

## 목적

코드 수정·검증·review/merge·외부 소비자·runtime activation을 구별하고 exact-source qualification으로 종료한다.

## 변경 범위

- 기존: [docs/release-checklist.md](../../../../docs/release-checklist.md)
- 기존: [.github/workflows/ci.yml](../../../../.github/workflows/ci.yml)
- 기존: [.github/workflows/bench.yml](../../../../.github/workflows/bench.yml)
- 제안 경로: `tools/qualification/validate_receipt.py` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음
- 제안 경로: `docs/plans/sep-16-hardening/receipts/` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [ ] 모든 선행 티켓 acceptance와 결정 승인 상태, 미해결 exception을 matrix로 모은다. TM16-005는 계약 결정과 일치하는 regression을 요구하며 독립 runtime defect로 재분류하지 않는다.
- [ ] 개발 검증에는 HEAD/tree·dirty/untracked path/content/mode/symlink digest·명령·cwd·toolchain/features/env/OS·start/end·exit/test count/metrics/artifacts를 기록한다. 비밀 env 값은 기록하지 않는다.
- [ ] 최종 검증은 수정 불가한 별도 checkout/worktree 및 고정 의존·artifact에서 수행한다. 전후 digest만으로 중간 edit-and-restore를 검출할 수 없으므로 mutable shared tree를 final receipt로 인정하지 않는다.
- [ ] H16-014 repaired-original negative proofs, strict gate matrix, 실제 consumer MSRV, Linux performance baseline/candidate, PM nonmutating lint를 동일 source identity에 연결한다. 필요한 gate 미실행은 NOT_QUALIFIED다.
- [ ] hosting check run·required branch protection·review·merge SHA를 조회하여 exact SHA와 연결한다. 접근권한이 없거나 consumer repo/SHA가 미제공이면 해당 항목 UNVERIFIED/EXTERNAL_BLOCKED로 남긴다.
- [ ] runtime inventory diff, capability limits, queue/reject/fairness/cleanup 지표로 단계적 rollout 기준을 정한다. metadata-only shadow는 job을 재실행하지 않는다. deploy/activation은 실제 소비자 owner의 명시적 승인 후 별도 수행한다.
- [ ] rollback은 이전 배포 artifact/config/inventory와 API/schema 호환성을 먼저 검증한다. 새 상태/메모리 snapshot을 구버전이 읽지 못하면 drain-and-restart 또는 forward fix를 선택한다.
- [ ] 최종 disposition을 implemented/local-qualified/hosted-qualified/reviewed/merged/consumer-qualified/activated로 나눠 기록한다. 라이브러리 repo만으로 서비스 배포 완료를 선언하지 않는다.

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local qualification tooling); 최종 qualification은 NOT_QUALIFIED (아래 사유).**

- `tools/qualification/receipt.py`: `collect`(source identity = HEAD + tracked/untracked 전체
  path·content·mode·symlink digest, 전후 재계산, toolchain·lock digest·OS, gate/mutation/msrv 결과)와
  `validate`(schema, 현재 tree와 digest 대조, required gate 전부 PASS, mutation 0 problem, msrv PASS).
  dirty tree·SKIPPED_PLATFORM·NOT_RUN·mismatch는 모두 NOT_QUALIFIED 사유로 나열된다.
- 22 ticket acceptance, 40 finding(COVERAGE 표), Q01–Q33 처분이 추적된다.
- 실제 receipt: `docs/plans/sep-16-hardening/receipts/`. 이 tree에서의 verdict는 **NOT_QUALIFIED**이며
  사유는 (1) dirty working tree(immutable checkout 아님), (2) `bench-iai` SKIPPED_PLATFORM(macOS).
  두 사유 모두 이 저장소의 상태를 정확히 말하는 것이며 숨기지 않는다.
- rollout/rollback: 새 snapshot schema(v2)는 구버전 reader가 읽지 못하므로 downgrade는
  drain-and-restart 또는 forward fix가 필요하다(release-checklist 기록). 배포/activation은 수행하지 않았다.

## 검증 / 완료 조건

- [x] `H16-022-A01` 22/40/33 추적 가능 (README, COVERAGE, 각 ticket)
- [x] `H16-022-A02` dirty/untracked mismatch·missing gate가 receipt 검증 실패 (`tools/qualification/tests`,
      임시 git repo fixture로 hermetic). 감사(A3-P1-3): validator는 이제 HEAD·dirtiness를 tree에서
      재도출하고 sub-receipt 일관성을 검사한다; gate 결과 자체는 `collect`가 attest하며 그 경계를
      `receipt.py` docstring과 release-checklist에 명시했다 (mutation `receipt-validator-trusts-the-receipts-dirty-flag`)
- [x] `H16-022-A03` mutation 43건 KILLED/CONTROL_GREEN (receipt; 최초 20건 → 1차 감사 후 37건 → 2차 감사 후 43건)
- [ ] `H16-022-A04` MSRV consumer receipt 존재(PASS); Linux IAI는 **명시적 blocker** (macOS). CI
      `qualification` job(ci.yml)이 clean ubuntu checkout에서 `receipt.py collect`를 실행하고 artifact로
      올리도록 wired — 첫 push에서 BASELINE_CREATED, 두 번째부터 bench-iai QUALIFIED. 이 저장소에서
      push 권한이 없어(`songminjo` → `josongmin/quanta-taskmesh` 403) 실제 실행은 owner의 push 후다.
- [ ] `H16-022-A05` review/merge/consumer/activation은 **UNVERIFIED** (commit/push 미수행, 외부 접근 없음)
- [ ] `H16-022-A06` rollback 계약 문서화; owner 승인은 별도

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
git rev-parse HEAD
git status --porcelain=v1 --untracked-files=all
just gate
just proof
uv run python tools/pm/pm.py lint
```

## 호환성 / 실패 모드

- 이 계획의 문서 검증 PASS는 production qualification이 아니다.
- 이전 audit repro의 PASS는 결함 관찰 성공이며 수정 완료 증거가 아니다.

## 인계 / 완료 증거

- [ ] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다.
- [ ] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다.
- [ ] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다.

