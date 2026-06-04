# T09 Rayon Executor Adapter

## Summary

phase-2 shared CPU convergence의 실제 code owner를 이 레포 안에 만든다.

## Decisions Frozen

1. `taskmesh-rayon`은 optional crate로 추가한다.
2. `run_cpu`는 executor abstraction을 타야 한다.
3. large-stack requirement는 별도 capability 예외로 남는다.
4. `spawn_blocking` direct-only shape는 interim이 아니라 residue다.

## Files To Touch

1. `Cargo.toml`
2. `crates/taskmesh-rayon/Cargo.toml` 신규
3. `crates/taskmesh-rayon/src/lib.rs` 신규
4. `crates/taskmesh-rayon/tests/rayon_smoke.rs` 신규
5. `crates/taskmesh-tokio/src/lib.rs`
6. `crates/taskmesh-tokio/tests/runtime_cpu_executor.rs` 신규

## Implementation

1. workspace에 `taskmesh-rayon` crate를 추가한다.
2. CPU executor abstraction trait를 정의한다.
3. default `taskmesh` runtime은 CPU path에서 executor abstraction을 사용한다.
4. `taskmesh-rayon`는 shared pool adapter를 제공한다.
5. `run_cpu`의 current tokio-only fallback을 abstraction 뒤로 숨긴다.
6. large-stack path는 여전히 `SubstrateHint::LargeStackCapability`/separate handling으로 둔다.

구체 선택:

1. 라이브러리
   - `rayon = "1"`
2. crate shape
   - `taskmesh-rayon` exposes `RayonCpuExecutor`
3. abstraction
   - `trait CpuExecutor { fn execute<T, E, F>(&self, job: F) -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>; }`
4. tokio bridge
   - `tokio::sync::oneshot`로 Rayon closure result를 async로 전달
5. pool shape
   - single shared `rayon::ThreadPool`
   - worker count는 `TopologyConfig.cpu`에서 계산
6. auto worker calculation
   - `available_parallelism() - reserve_cores`
   - `[min_workers, max_workers]` clamp

스니펫:

```rust
let (tx, rx) = tokio::sync::oneshot::channel();
pool.spawn(move || {
    let _ = tx.send(job());
});
```

## Tests

1. rayon adapter compile/smoke
2. CPU work executes through abstraction
3. runtime still works without rayon crate feature if designed optional
4. large-stack path does not get incorrectly routed to shared CPU pool
5. shared pool worker count respects topology clamp

## Not Done If

1. `run_cpu`가 forever `spawn_blocking` wrapper로만 남는다.
2. executor abstraction 없이 phase-2 ticket을 닫으려 한다.
3. CPU path와 large-stack 예외가 다시 섞인다.
