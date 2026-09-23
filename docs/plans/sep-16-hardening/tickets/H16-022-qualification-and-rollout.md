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
  따라서 이 bundle은 historical execution evidence일 뿐 현재 schema-v4 validator의
  qualification receipt가 아니다. 당시 계획은 수정된 collector를 commit한 뒤 Linux CI에서
  새 receipt of record를 수집하는 것이었다.
- rollout/rollback: 새 snapshot schema(v2)는 구버전 reader가 읽지 못하므로 downgrade는
  drain-and-restart 또는 forward fix가 필요하다(release-checklist 기록). 배포/activation은 수행하지 않았다.

## 현재 qualification 경로 — 2026-09-23

- 위 CI push/credential 설명은 2026-09-16 실행 기록이다. hardening 구현은 현재
  `origin/main@e55d3aa`에 포함됐고, GitHub `ci`·`bench-gates`·`release-qualification`
  workflows는 수동 비활성화 상태다. push나 hosted CI를 기다리는 것으로 A04가 닫히지 않는다.
- 현재 [release checklist](../../../release-checklist.md)와 `Justfile`은 clean Linux
  checkout에서 `just qualify-local` 및 `just validate-local-qualification`을 ordinary
  qualification의 권위로 둔다. 현재 receipt schema는 v4다. `bench-iai`는 같은
  fingerprint의 baseline 생성과 후속 `QUALIFIED` 실행을 구분한다. 현재 HEAD의 local
  qualification receipt는 없다.
- 이 local 경로는 clean 시작·종료 source identity를 확인하지만 checkout의 중간 수정 불가성을
  강제하지 않는다. 위 2026-09-16 "수정 불가한 별도 checkout" 액션의 hosted 구현 기록을 현재
  local run의 속성으로 옮겨 주장하지 않는다. 최종 실행은 배타적 clean checkout에서 수행하고
  실행 중 writer가 없음을 운영 경계로 확보해야 한다.
- merge-to-main은 확인됐다. `semantica-codegraph-v2@f2b5180b28e55d8fc268ec6fb299f1d16a81995b`의 추적된
  `quanta-runtime/Cargo.toml`은 Taskmesh 0.3.0 optional path dependency와
  `taskmesh-governance-test-hooks` 소비자 테스트를 선언한다. 이는 소비자 경로의 발견이지
  현재 dirty checkout과 Taskmesh 후보의 호환성 실행 증거가 아니다. PR review,
  consumer compile/test, deployment, activation 및 rollback owner 승인은 여전히
  별도 증거가 필요하다. [예외 대장](EXCEPTIONS.md)에 현재 경로를 기록한다.

## 검증 / 완료 조건

- [x] `H16-022-A01` 22/40/33 추적 가능 (README, COVERAGE, 각 ticket)
- [x] `H16-022-A02` dirty/untracked mismatch·missing/duplicate gate·stale gate summary·mutation
      count/status/evidence digest mismatch가 receipt 검증 실패 (`tools/qualification/tests`, 임시 git repo
      fixture로 hermetic). gate 결과 자체는 `collect`가 attest하며 그 경계를 `receipt.py` docstring과
      release-checklist에 명시했다.
- [x] `H16-022-A03` curated mutation inventory 103건 = 102 defect probes + 1 CONTROL_GREEN
      (cargo runner 90, pytest runner 13; generated cargo-mutants score와 별도). historical receipt는
      당시 inventory 100건을 담는다. 현재 inventory는 그 100개 ID 대비 6개가 추가되고
      3개가 제거되어 103건이다 (`tools/verification/mutations.json`).
- [ ] `H16-022-A04` MSRV consumer의 과거 local PASS는 현 HEAD의 qualification이 아니다.
      Linux IAI와 ordinary required gates는 clean Linux 후보에서 `just qualify-local`로 수집하고
      `just validate-local-qualification`이 동일 source에 `QUALIFIED`를 반환해야 한다.
      IAI의 첫 동일-fingerprint run은 baseline 생성이며 qualified 비교가 아니다.
      2026-09-23 owner-local IAI fixture에서는 `Ir` 누락·비수치·0이 비교를
      `QUALIFIED`로 오인하던 결함과 raw `.out` 없이 baseline을 생성하던 결함을 재현 후
      수정했다 (`tools/bench/tests/test_iai_gate.py`). 후속 감사에서는 runner가 세 case 중
      하나만 출력해도 `QUALIFIED`가 되던 경로를 재현하고, `perf-gate.json`의 명시적
      case inventory와 실제 summary identity의 완전 일치, case별 raw `.out`의 존재와
      case 간 raw 출력의 비공유를 강제했다.
      `target` 상위 경로 symlink가 외부 baseline을 지우는 경로도 재현해 차단했다. 이 fixture 결과는
      Linux Valgrind 실측, 정상 control 및 >5% negative, clean-source qualification을 대체하지 않는다.
      Release raw 파일의 digest가 맞아도 비교 manifest가 `BASELINE_CREATED`인 경우를
      release validator가 별도로 거절하지 않던 경로를 2026-09-23 재현해 보강했다.
      현재 validator는 source fingerprint, baseline/raw summary, 기대 case와 ordinary
      `bench-iai` 상태줄을 다시 대조한다. in-store 상위 symlink를 통한 raw `.out` 별칭도
      두 case가 동일 출력으로 인정되지 않도록 차단했다. Dirty owner-local release 55/55 및 IAI 36/36
      테스트 PASS는 이 검증 로직의 회귀 증거이며 Linux 실측이나 release qualification이 아니다.
  → 예외 대장: [EXCEPTIONS.md](EXCEPTIONS.md)
- [ ] `H16-022-A05` hardening 구현의 `origin/main` 포함과 Semantica 소비자
      `f2b5180b28e55d8fc268ec6fb299f1d16a81995b`의 추적된 optional path dependency는 확인했다. 현재 두 소스를 결합한
      consumer compile/test, PR review disposition, deployment 및 activation은
      **UNVERIFIED**다.
  → 예외 대장: [EXCEPTIONS.md](EXCEPTIONS.md)
- [ ] `H16-022-A06` rollback 계약 문서화; owner 승인은 별도
  → 예외 대장: [EXCEPTIONS.md](EXCEPTIONS.md)

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
git rev-parse HEAD
git status --porcelain=v1 --untracked-files=all
just gate
just release
```

## 호환성 / 실패 모드

- 이 계획의 문서 검증 PASS는 production qualification이 아니다.
- 이전 audit repro의 PASS는 결함 관찰 성공이며 수정 완료 증거가 아니다.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-19.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)
