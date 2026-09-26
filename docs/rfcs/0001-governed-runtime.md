# Governed Runtime RFC

Status: `Historical design summary`

This early RFC records the original principles and is not an active implementation
plan or a complete inventory of today's public API. Current architecture is owned by
[ADR 0001](../adr/0001-hexagonal-feature-sliced-architecture.md); implemented APIs are
specified in the [library spec](../taskmesh-library-spec.md) and
[external interface](../taskmesh-external-interface.md).
The active SDK improvement proposal is
[SEP-27 SDK DX and extensibility](../rfc/sep-27-sdk-dx-extensibility.md).

## Executive Decision

1. semantic policy와 worker governance는 분리한다.
2. public API는 product-neutral naming만 쓴다.
3. topology와 class policy는 분리한다.
4. capability pool만 연다.
5. unknown class는 fail-closed reject다.
6. parallel stage는 deterministic reduce contract가 필요하다.

## Built-in Capability Pools

1. `cpu`
2. `blocking`
3. `large_stack`
4. `maintenance`
5. `local_runtime`

## Public Surface

1. `TaskClass`
2. `TaskStage`
3. `TaskSpec`
4. `Runtime`
5. `ClassPolicy`
6. `ResourceBudget`
7. `Snapshot`

## Hard Rules

1. engine-specific pool proliferation 금지
2. unbounded competing channel 금지
3. telemetry-only를 memory governance라고 부르지 않음
4. child permit은 root operation에 귀속
5. `run_local`은 `local_runtime` 예외 전용
6. `RunError<E>`에서 governor rejection과 task failure를 분리
