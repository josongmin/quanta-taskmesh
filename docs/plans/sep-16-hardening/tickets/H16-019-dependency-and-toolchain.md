# H16-019 — Dependency advisory·consumer/dev toolchain 분리

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P1 (원본 bug severity 변경 아님)
- 책임 역할: D — Dependency/toolchain owner (실제 assignee 미지정)
- 선행 완료: 없음
- 원본 finding: [TM16-007](../../../bugbash/sep-16-general/tickets/TM16-007-dependency-advisories.md), [TM16-021](../../../bugbash/sep-16-general/tickets/TM16-021-developer-toolchain-floor.md)
- 배타적 write lease: `manifests`; [적용 순서](EXECUTION.md) 준수

## 목적

dependency advisory와 consumer MSRV/developer toolchain 요구를 실제 graph별로 분리해 검증한다.

## 변경 범위

- 기존: [Cargo.toml](../../../../Cargo.toml)
- 기존: [Cargo.lock](../../../../Cargo.lock)
- 기존: [crates/taskmesh/Cargo.toml](../../../../crates/taskmesh/Cargo.toml)
- 기존: [crates/taskmesh-engine/Cargo.toml](../../../../crates/taskmesh-engine/Cargo.toml)
- 기존: [config/deny.toml](../../../../config/deny.toml)
- 제안 경로: `tools/consumer-msrv/Cargo.toml` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음
- 제안 경로: `tools/consumer-msrv/src/main.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음
- 제안 경로: `rust-toolchain.toml` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [x] cargo metadata와 reverse dependency graph로 crossbeam-epoch/proc-macro-error2 도입 경로를 기록한다. optional iai feature graph도 명시적으로 활성화하고 출력 부재를 dependency 제거 증거로 쓰지 않는다. advisory id·DB revision·조회 시각·대상 graph를 고정하며 exploit 가능성을 단정하지 않는다.
- [x] 호환 수정 버전·직접/간접 의존 변경을 최소 범위로 적용하고 Cargo.lock 단일 소유자 D가 갱신한다. 해결 불가 예외는 owner/근거/expiry/대체 통제를 갖춰 승인받는다.
- [x] declared consumer MSRV와 workspace 개발·test·bench toolchain floor를 별도 문서/CI matrix로 정의한다. dev-only dependency의 floor를 library consumer MSRV 위반으로 혼동하지 않는다.
- [x] workspace 밖 minimal consumer fixture를 별도 workspace로 구성하고 실제 선언된 MSRV toolchain에서 default 및 Rayon public surface를 compile한다. root workspace resolver/dev-deps가 fixture를 오염시키지 못한다.
- [x] 개발 toolchain 및 lockfile 버전과 fresh checkout/bootstrap 절차를 검증한다. 정확한 CI toolchain은 설치 가능성과 지원 정책에 맞춰 승인하고 floating latest에 의존하지 않는다.
  → fresh clone bootstrap은 확인(validator·workspace build·doc-examples); CI dev toolchain은 `stable` channel, 소비자 MSRV는 1.81 고정·별도 job
- [x] 각 lane의 manifest/export/feature 변경 요청을 D가 직렬 적용하고 feature additive/optional dependency graph를 재확인한다.

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- crossbeam-epoch 0.9.18 → 0.9.21 (`cargo update -p`, lockfile diff 1 package). RUSTSEC-2026-0204 제거.
- proc-macro-error2 (RUSTSEC-2026-0173): 최신 iai-callgrind 0.16.1도 여전히 의존함을 확인
  (`/tmp/iaiprobe` graph). safe upgrade 없음 → `config/deny.toml`에 **reason·범위·retire 조건**이 있는
  exception. cargo-deny는 unused ignore를 경고하므로 advisory 소멸 시 자동으로 드러난다.
- `just deny` green.
- `tools/consumer-msrv/`: 자체 `[workspace]`인 외부 consumer fixture. default + rayon surface를
  **실제 1.81.0 toolchain**에서 compile → PASS. toolchain 부재 시 `NOT_RUN` exit 2 (PASS로 기록 안 함).
  CI `consumer-msrv` job이 1.81.0을 설치해 실행.
- workspace `rust-version = 1.81`은 consumer MSRV; dev/bench graph(getrandom/proptest/clap 1.85+)는
  stable에서만 resolve된다는 사실을 `check.py` doc과 Justfile에 명시.
- clippy `incompatible_msrv`로 `Option::is_none_or`(1.82) 사용을 잡아 제거했다.

## 검증 / 완료 조건

- [x] `H16-019-A01` crossbeam 제거; proc-macro-error2는 만료형 exception + receipt(이 절)
- [x] `H16-019-A02` 실제 1.81 default/rayon compile PASS (로컬 receipt: 위); NOT_RUN은 exit 2
- [x] `H16-019-A03` dev/bench graph는 stable(1.95)에서 전체 gate green
- [x] `H16-019-A04` lockfile diff = crossbeam-epoch 1건, unrelated update 없음

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
just deny
cargo metadata --format-version 1 --locked
cargo tree -i crossbeam-epoch
cargo tree --locked -p taskmesh-bench --features iai -i proc-macro-error2
```

## 호환성 / 실패 모드

- 의존 업데이트만으로 특정 CVE exploitability가 입증되거나 제거되었다고 과장하지 않는다.
- MSRV를 올리는 선택은 소비자 호환성 결정이며 자동 수용하지 않는다.

## 인계 / 완료 증거

- [x] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다. → `../receipts/local-2026-09-19.json` (gate·mutation receipt; [EXCEPTIONS.md](EXCEPTIONS.md) §인계 항목 1)
- [x] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다. → 단일 작업자(인계 없음); production 통합·외부 소비자·activation은 UNVERIFIED로 [EXCEPTIONS.md](EXCEPTIONS.md)에 표시
- [x] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다. → [EXCEPTIONS.md](EXCEPTIONS.md)
