# H16-014 — Production regression·mutation·concurrency proof

- 상태: IMPLEMENTED — 구현·local regression 완료 (2026-09-16); qualification은 H16-022
- 실행 우선순위: P1 (원본 bug severity 변경 아님)
- 책임 역할: V — 독립 검증 owner (실제 assignee 미지정)
- 선행 완료: [H16-013](H16-013-observability-and-invariants.md)
- 원본 finding: [TM16-040](../../../bugbash/sep-16-general/tickets/TM16-040-mixed-soak-discards-all-run-errors.md)
- 배타적 write lease: `engine-tests`, `host-tests`; [적용 순서](EXECUTION.md) 준수

## 목적

원본 production transition과 테스트가 실제 결함·전건 거부를 검출하는 proof를 만든다.

## 변경 범위

- 기존: [crates/taskmesh/tests/e2e_chaos.rs](../../../../crates/taskmesh/tests/e2e_chaos.rs)
- 기존: [crates/taskmesh/tests/runtime_cancel_leak.rs](../../../../crates/taskmesh/tests/runtime_cancel_leak.rs)
- 기존: [crates/taskmesh-engine/tests/loom_governance.rs](../../../../crates/taskmesh-engine/tests/loom_governance.rs)
- 기존: [crates/taskmesh-engine/tests/shuttle_governance.rs](../../../../crates/taskmesh-engine/tests/shuttle_governance.rs)
- 기존: [crates/taskmesh-engine/tests/prop_invariants.rs](../../../../crates/taskmesh-engine/tests/prop_invariants.rs)
- 제안 경로: `crates/taskmesh-engine/tests/hardening_model.rs` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음
- 제안 경로: `tools/verification/run_mutations.py` — 향후 구현 시 생성/이름 확정; 현재 존재한다고 가정하지 않음

## 구현 액션

- [ ] runtime/engine TM16 observation을 corrected-behavior production regression으로 옮긴다. tools/bench/PM finding은 해당 lane이 검증하고 전체 40건 취합은 H16-022에서 수행한다. audit harness의 bug-preserving assertions는 역사 자료로 남기고 green을 closure로 쓰지 않는다.
- [ ] mixed soak에 substrate별 attempted/started/completed/typed outcome count와 live sweeper overlap barrier를 추가한다. zero ledger만으로 success를 판정하지 않는다.
- [ ] repaired original test에 동일 all-disabled mutation만 적용하고 expected assertion failure를 확인한다. source extractor guard·compile failure·0 tests·timeout은 mutation kill로 인정하지 않는다.
- [ ] production synchronization seam을 cfg로 교체하여 작은 lifecycle interleaving을 Loom에 연결한다. 별도 기존 toy model과 coverage를 구분하고 remaining uninstrumented APIs를 기록한다.
- [ ] 독립 wide ledger/reference scheduler와 event sequence property test를 연결한다. cancel/drop/reap/start/complete/shutdown, callback reentry·stale effect·nested wait를 포함한다.
- [ ] 행동 동기화에는 start/claim/drop barriers를 사용한다. scheduler fairness와 wall-clock SLA를 arbitrary sleeps로 단언하지 않는다.
- [ ] hang 재현은 process isolation+deadline+kill+wait를 사용한다. negative control이 의도한 assertion까지 도달했다는 marker를 보존한다.

## 구현 결과 — 2026-09-16

**상태: 구현 완료 (local).**

- `tools/verification/run_mutations.py` + `mutations.json`: 단일 편집 mutation 43건(control 1건, Python 도구 5건 포함; 최초 20건 → 37 → 43).
  각 mutation은 **지정된 test가 지정된 사유로** FAIL해야 KILLED다. compile error·다른 test 실패·
  0 collected·timeout은 kill로 인정하지 않는다(`INVALID_*`/`WRONG_TEST`/`WRONG_REASON`). control은
  behaviour-preserving 편집으로 suite가 GREEN이어야 하며 runner가 항상 실패만 찾는 것이 아님을 증명한다.
  `just mutants-critical`로 wired, CI `mutation-gate` job.
- mixed soak(TM16-040): substrate별 attempted/completed/started 계수, 모든 run 결과 검증(`Err`는 panic),
  sweeper가 실제 live inflight를 관측했음을 단언(`max_live_seen > 0`), 누적 counter와 대조.
  all-disabled mutation(`mixed-soak-all-classes-disabled`)은 KILLED.
- ~~loom 4 / shuttle 3 test가 새 engine seam에서 PASS~~ — **감사(A2-P0-1)에서 거짓으로 판정.**
  최초 구현의 두 파일은 locking 설계를 손으로 베낀 toy model이었다. 지금은 engine `src/sync.rs`
  seam으로 **production `Governor`**를 checker의 mutex/atomics 위에 컴파일하며, loom 5개
  (9–810 interleavings 전수: admit/release conservation, promote-claim-abandon 3-way,
  lost-wakeup freedom, claim-vs-reap 양방향, 무순서 reconcile)와 shuttle 4개(각 10,000 schedule)가
  실제 `admit`/`claim`/`abandon`/`release`/`reap_leaks`/`reconcile_memory`를 호출한다.
  checker는 optional feature(`loom`/`shuttle`) 뒤에 있어 consumer lockfile을 바꾸지 않는다.
- mutation inventory는 감사 후 43건(control 1, `runner: pytest` 5)이며 모든 non-control entry에
  `expect_message`가 강제된다. TM16-002/024/012의 hang-only test는 5초 상한과 release-on-drop으로
  bounded되어 각각 mutation으로 KILLED된다. TM16-010은 `profile: release`로 test의 assertion이
  detector임을 확인했다.
- barrier/handshake: 신규 host regression은 oneshot·mpsc barrier + bounded drain 대기를 사용한다.
- hang 재현: deadlock probe는 worker thread + `recv_timeout`으로 bounded 관측
  (`hardening_effect_retirement.rs::with_deadline`).

Receipt: `docs/plans/sep-16-hardening/receipts/local-2026-09-16.mutations.json`.

## 검증 / 완료 조건

- [x] `H16-014-A01` runtime/engine 40건 각각 corrected regression 연결 (COVERAGE 표)
- [x] `H16-014-A02` 정상 soak PASS, all-disabled mutant FAIL, 미선택은 runner가 `INVALID_NO_TESTS`
- [x] `H16-014-A03` release 누락/dead claim/wrong capability/late success/invalid memory report
      mutation 각각 KILLED (`dead-ticket-survives-unwind`, `capability-not-checked-at-admission`,
      `run-deadline-judged-from-the-timer-only`, `saturating-capacity-check` 등)
- [x] `H16-014-A04` loom/shuttle가 production `Governor`를 실행 (`src/sync.rs` seam; toy model 폐기 —
      [AUDIT-2026-09-16](../AUDIT-2026-09-16.md) T2-P0-1)
- [x] `H16-014-A05` debug/release(workspace test는 debug, loom/shuttle는 release), default/Rayon
      (`just test-rayon`), local non-Send path 확인

## 실행 명령

아래는 현재 존재하는 진입점이며 구현 후 실행할 후보다. 이 계획 작성에서 실행한 결과가 아니다. 새 테스트/feature와 플랫폼별 정확한 command는 구현 receipt에 고정한다.

```sh
cargo test --workspace
cargo test -p taskmesh --features rayon
just loom
just shuttle
```

## 호환성 / 실패 모드

- 전체 상태공간 formal proof나 unknown external executor의 올바름으로 확대하지 않는다.
- mutation runner는 신규 구현 대상이다. 현재 존재하는 command처럼 안내하지 않는다.

## 인계 / 완료 증거

- [ ] acceptance ID별 exact-source receipt와 정상/negative 결과를 [검증 계약](VERIFICATION.md)에 맞춰 첨부한다.
- [ ] 공용 파일 변경은 lease owner에게 인계하고, production 통합·외부 소비자·activation 상태를 독립 표시한다.
- [ ] 남은 예외는 owner·사유·만료/재검토 조건을 기록한다. 티켓 구현 완료가 전체 qualification 완료는 아니다.
