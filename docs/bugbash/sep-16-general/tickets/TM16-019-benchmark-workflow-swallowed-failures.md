# TM16-019 — Benchmark trend workflow가 pipeline/branch lookup 실패를 삼킨다

- Severity: P2
- Status: OPEN
- Lane: verification-benchmark
- Baseline: 9ae9547216c70458f54f37368a67661321060886 (2026-09-16)

## 근거

.github/workflows/bench.yml:94-112,117-135

## Trigger / 관찰

cargo bench가 실패하지만 tee가 성공하거나, ls-remote가 network/auth failure로 nonzero를 반환한다.

shell을 명시하지 않은 bash -e pipeline은 마지막 tee 성공으로 exit 0 가능하다. main의 bash -e 'false | true' diagnostic exit 0. ls-remote exit code 2(missing)와 128(transport/auth)을 모두 exists=false로 변환한다. main에서도 absent branch 시 store/bootstrap을 skip한다.

## 원인 / 영향 / 범위

partial output가 다음 store action에 전달되면 해당 action이 일부 실패를 찾을 수는 있지만 모든 bench 실패를 보장하지 않는다. raw benchmark command failure가 step에서 직접 전파되지 않는 문제다. history bootstrap 결여는 신규 repo의 trend activation gap이며 runtime defect가 아니다.

## 보완 계획

명시적 shell:bash/pipefail로 benchmark failure를 전파하고 output validity도 확인한다. ls-remote missing vs infra failure를 분리한다. main의 absent history는 bootstrap publication path를 설계한다.

## Acceptance / 회귀 검증

partial benchmark output 후 nonzero, tee failure, ls-remote missing/128, 신규 main branch bootstrap, PR no-write 조건을 shell fixture로 검증한다. [GitHub shell reference](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#jobsjob_idstepsshell).

