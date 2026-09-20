# SEP-21 parallel execution prompt pack

이 디렉터리는 `tickets/README.md`의 exclusive write ownership을 실제 병렬 작업 단위로 변환한다.
12개 티켓을 12명에게 동시에 배포하지 않는다. 같은 파일과 authority를 공유하는 티켓은 한
owner가 끝까지 직렬 통합한다.

## 실행 packet

| packet | 포함 티켓 | 시작 gate | 내부 순서 |
| --- | --- | --- | --- |
| [`10-contract-c01.md`](10-contract-c01.md) | C01 | campaign baseline frozen | C01 |
| [`20-engine-core.md`](20-engine-core.md) | E01,E02,E03,E04 | 즉시 시작; E04만 C01 contract-ready 대기 | E01→E02→E03→E04 |
| [`30-proof-envelope-v01.md`](30-proof-envelope-v01.md) | V01 | 즉시 시작 | schema-ready→producer 대기→shared integration |
| [`40-host-runtime.md`](40-host-runtime.md) | H01,H03,H02 | H01은 C01 뒤; H03/H02는 E04 뒤 | H01→H03→H02 |
| [`50-mutation-v02.md`](50-mutation-v02.md) | V02 | V01 schema-ready | V02 |
| [`60-concurrency-v03.md`](60-concurrency-v03.md) | V03 | V01 schema-ready | V03 |
| [`70-release-r01.md`](70-release-r01.md) | R01 | 모든 product/proof closure 뒤 | R01 only |

전체 조정은 [`00-orchestrator.md`](00-orchestrator.md)를 별도 coordinator에게 준다.

## 최대 안전 병렬성

```text
W0: contract + engine-core + proof-envelope
W1: engine-core + host(H01) + mutation + concurrency
W2: host(H03→H02) + proof-envelope(shared integration)
W3: release only
```

동일 lane을 둘로 나누거나 `runtime.rs`, `governor.rs`, shared receipt/inventory/workflow,
README/CHANGELOG/ADR을 여러 agent가 동시에 수정하면 안 된다.

## 모든 worker의 공통 규칙

1. 저장소 `AGENTS.md`, 담당 ticket 원문, `tickets/README.md`, `plan.json`을 먼저 읽는다.
2. 첫 production edit 전에만 strict baseline validator를 실행한다. 다른 worker가 이미 source를
   바꿨다면 coordinator가 보존한 baseline receipt를 사용하고 strict PASS를 재현했다고 주장하지
   않는다.
3. 구현 중에는 `validate_plan.py --structure-only`만 사용한다. 이는 qualification이 아니다.
4. 시작 시 HEAD/tree, `git status --short`, 담당 path의 diff를 기록한다. 기존 dirty hunk를
   되돌리거나 포맷팅으로 덮지 않는다.
5. assigned write scope 밖 수정이 필요하면 직접 고치지 말고 정확한 symbol/type/error contract를
   coordinator와 owner에게 interface request로 보낸다.
6. 같은 lane은 한 worker가 최종 API까지 소유한다. compatibility shim, dual representation,
   after-the-fact check를 중간 merge하지 않는다.
7. Cargo 작업은 lane별 `CARGO_TARGET_DIR=target/sep21/<packet>`을 사용한다. mutation packet은
   V02가 정의한 isolated target/worktree만 사용한다.
8. shared README, CHANGELOG, external interface, library spec, ADR은 R01만 수정한다. product
   worker는 ticket closure에 documentation delta만 남긴다.
9. focused test PASS를 release qualification으로 표현하지 않는다. 미실행/실패/플랫폼 제한을
   그대로 기록한다.
10. 완료 보고에는 base/final HEAD·tree, 변경 파일, acceptance별 증거, 명령·exit code·test count,
    negative fixture, 잔여 risk, 다음 dependency signal을 포함한다.

## dependency signal 이름

- `C01_CONTRACT_READY`
- `E04_RESOLVER_READY`
- `V01_SCHEMA_READY`
- `V02_PRODUCER_READY`
- `V03_PRODUCER_READY`
- `PRODUCT_LANES_CLOSED`
- `PROOF_LANES_CLOSED`

signal은 문장형 주장만 보내지 않는다. commit/diff identity, public symbols, changed paths, 실행한
검증과 미실행 항목을 같이 보낸다.
