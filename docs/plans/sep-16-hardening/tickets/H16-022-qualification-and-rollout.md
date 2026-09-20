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

- [x] 모든 선행 티켓 acceptance와 결정 승인 상태, 미해결 exception을 matrix로 모은다. TM16-005는 계약 결정과 일치하는 regression을 요구하며 독립 runtime defect로 재분류하지 않는다.
- [x] 개발 검증에는 HEAD/tree·dirty/untracked path/content/mode/symlink digest·명령·cwd·toolchain/features/env/OS·start/end·exit/test count/metrics/artifacts를 기록한다. 비밀 env 값은 기록하지 않는다.
- [x] 최종 검증은 수정 불가한 별도 checkout/worktree 및 고정 의존·artifact에서 수행한다. 전후 digest만으로 중간 edit-and-restore를 검출할 수 없으므로 mutable shared tree를 final receipt로 인정하지 않는다.
  → isolated hosted checkout = CI `qualification` job(Linux clean checkout, `--hosted-ci`가
  GitHub Actions/workspace/GITHUB_SHA를 검증); local receipt는 exact-source 증거로만
- [x] H16-014 repaired-original negative proofs, strict gate matrix, 실제 consumer MSRV, Linux performance baseline/candidate, PM nonmutating lint를 동일 source identity에 연결한다. 필요한 gate 미실행은 NOT_QUALIFIED다.
  → Linux IAI만 BLOCKED(EXCEPTIONS H16-022-A04)
- [x] hosting check run·required branch protection·review·merge SHA를 조회하여 exact SHA와 연결한다. 접근권한이 없거나 consumer repo/SHA가 미제공이면 해당 항목 UNVERIFIED/EXTERNAL_BLOCKED로 남긴다.
  → EXTERNAL/UNVERIFIED(EXCEPTIONS H16-022-A05)
- [x] runtime inventory diff, capability limits, queue/reject/fairness/cleanup 지표로 단계적 rollout 기준을 정한다. metadata-only shadow는 job을 재실행하지 않는다. deploy/activation은 실제 소비자 owner의 명시적 승인 후 별도 수행한다.
  → rollout 기준은 `docs/release-checklist.md` §Rollout / rollback; activation은 consumer owner 승인
- [x] rollback은 이전 배포 artifact/config/inventory와 API/schema 호환성을 먼저 검증한다. 새 상태/메모리 snapshot을 구버전이 읽지 못하면 drain-and-restart 또는 forward fix를 선택한다.
  → `docs/release-checklist.md` §Rollout / rollback
- [x] 최종 disposition을 implemented/local-qualified/hosted-qualified/reviewed/merged/consumer-qualified/activated로 나눠 기록한다. 라이브러리 repo만으로 서비스 배포 완료를 선언하지 않는다.
  → implemented + local evidence only(schema-v1 summary inconsistency, bench-iai SKIPPED_PLATFORM);
  hosted/reviewed/merged/consumer/activated = UNVERIFIED(EXCEPTIONS)

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local qualification tooling); 최종 qualification은 NOT_QUALIFIED (아래 사유).**

- `tools/qualification/receipt.py`: `collect`(source identity = HEAD + tracked/untracked 전체
  path·content·mode·symlink digest, 전후 재계산, toolchain·lock digest·OS, gate/mutation 결과)와
  `validate`(schema, 현재 tree와 digest 대조, required gate 전부 PASS, gate summary·mutation
  count/status/digest 일관성). MSRV는 required gate의 실행 결과가 단일 권위이며 중복 실행하지 않는다.
  dirty tree·SKIPPED_PLATFORM·NOT_RUN·mismatch는 모두 NOT_QUALIFIED 사유로 나열된다.
- 22 ticket acceptance, 40 finding(COVERAGE 표), Q01–Q33 처분이 추적된다.
- historical schema-v1 receipt: `docs/plans/sep-16-hardening/receipts/local-2026-09-19.json`
  (clean committed tree `cc5b256`). raw `.gates.json`은 22 PASS + 1 SKIPPED + mutation NOT_RUN이고,
  combined 파일은 별도 curated mutation receipt에서 derived PASS를 append했지만 summary를 재계산하지
  않아 자기모순이었다. coverage도 instantiations 56.63%와 branch 미수집을 status line에서 누락했다.
  따라서 이 bundle은 historical execution evidence일 뿐 현재 schema-v2 validator의 qualification
  receipt가 아니다. 수정된 collector를 commit한 뒤 Linux CI에서 새 receipt of record를 수집해야 한다.
- rollout/rollback: 새 snapshot schema(v2)는 구버전 reader가 읽지 못하므로 downgrade는
  drain-and-restart 또는 forward fix가 필요하다(release-checklist 기록). 배포/activation은 수행하지 않았다.

## 검증 / 완료 조건

- [x] `H16-022-A01` 22/40/33 추적 가능 (README, COVERAGE, 각 ticket)
- [x] `H16-022-A02` dirty/untracked mismatch·missing/duplicate gate·stale gate summary·mutation
      count/status/evidence digest mismatch가 receipt 검증 실패 (`tools/qualification/tests`, 임시 git repo
      fixture로 hermetic). gate 결과 자체는 `collect`가 attest하며 그 경계를 `receipt.py` docstring과
      release-checklist에 명시했다.
- [x] `H16-022-A03` curated mutation inventory 105건 = 104 defect probes + 1 CONTROL_GREEN
      (cargo runner 90, pytest runner 15; generated cargo-mutants score와 별도). historical receipt는
      첫 100건이고 receipt/coverage/runner 무결성 probe 4건은 schema-v2 보완에서 추가했다.
- [ ] `H16-022-A04` MSRV consumer receipt 존재(PASS); Linux IAI는 **명시적 blocker** (macOS). CI
      `qualification` job(ci.yml)이 clean ubuntu checkout에서 `receipt.py collect`를 실행하고 artifact로
      올리도록 wired — 첫 push에서 BASELINE_CREATED, 두 번째부터 bench-iai QUALIFIED. 이 저장소에서
      push 권한이 없어(`songminjo` → `josongmin/quanta-taskmesh` 403) 실제 실행은 owner의 push 후다.
  → 예외 대장: [EXCEPTIONS.md](EXCEPTIONS.md)
- [ ] `H16-022-A05` review/merge/consumer/activation은 **UNVERIFIED** (commit/push 미수행, 외부 접근 없음)
  → 예외 대장: [EXCEPTIONS.md](EXCEPTIONS.md)
- [ ] `H16-022-A06` rollback 계약 문서화; owner 승인은 별도
  → 예외 대장: [EXCEPTIONS.md](EXCEPTIONS.md)

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

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-19.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)
