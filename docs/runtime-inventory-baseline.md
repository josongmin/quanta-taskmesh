# Runtime Inventory Baseline & Ratchet

This is the repo-native baseline for substrate inventory and the structural
invariants CI ratchets against. It travels with the code, not just the docs.

## Built-in substrate allowlist (SSOT)

The canonical built-in capability pools are fixed:

| substrate | kind | capability pool |
|---|---|---|
| `cpu` | CompetingExecution | `cpu` |
| `blocking` | CompetingExecution | `blocking` |
| `large_stack` | CompetingExecution | `large_stack` |
| `maintenance` | MaintenanceOnly | `maintenance` |
| `local_runtime` | CompetingExecution | `local_runtime` |

Sources kept in lockstep (drift fails CI):

- code: `taskmesh_engine::BUILTIN_SUBSTRATES`
- fixture: `crates/taskmesh-engine/tests/fixtures/substrate_allowlist.json`
- test: `substrate_inventory::builtin_set_matches_allowlist_fixture`

Adding a substrate is an explicit, reviewed change to all three. Registration
rejects duplicates and any non-`AuthorityOnly` record without a capability pool.

## Inventory backs the runtime (not just snapshot metadata)

The registered inventory is authoritative for *which* worker gates exist, not
merely descriptive:

- the host derives one capability-pool semaphore per **registered** substrate
  whose pool is topology-sized `> 0` (`SubstrateGates::from_inventory`); a pool
  absent from the inventory has no gate;
- the hint→pool mapping is the contract's `SubstrateHint::capability_pool()`, not
  a host hardcode — `run_*` looks a submission's hint up to its pool, then the
  pool up to its gate;
- topology (`blocking_threads`, `large_stack_slots`, `local_runtime_slots`,
  `maintenance_workers`, resolved CPU workers) supplies the **slot count**; the
  inventory supplies **existence + kind + capability pool**. The two compose:
  inventory ∩ topology = the live gate set.

So `SubstrateRecord.kind` / `capability_pool` are governance authority (snapshot,
validation, *and* gate derivation), and a `0` slot count means "unlimited" for an
existing pool — distinct from a pool that does not exist at all.

## Structural no-go ratchet

CI (`cargo clippy -D warnings` + the test matrix) holds these invariants:

1. **No raw spawn bypass.** All execution goes through the `Runtime` facade; the
   only `tokio::spawn_blocking` lives behind the `CpuExecutor` port
   (`BlockingPoolCpuExecutor`). Rayon plugs into the same port.
2. **No unbounded competing queue.** Every queueable class declares
   `max_queue_depth > 0` (enforced by `validate_policy`); overflow past it is
   `QueueFull`, never unbounded growth. Proven by `overload_stability` bench
   (`max_queue_observed <= max_queue_depth`).
3. **No engine-specific pool.** Substrates are the fixed allowlist above; the
   engine never grows per-engine pools.
4. **Fail-closed admission.** Unknown/disabled classes reject; no config-side
   default-admit escape hatch.
5. **Deterministic reduce.** No fan-out stage ships without a complete
   `DeterministicReducePolicy`.

## Performance baseline (ADR 9000)

Wall-clock numbers are runner-dependent (relative gates only). The
instruction-count / allocation track is the machine-independent regression gate;
the baseline is "regression 0 vs current", not "0-alloc" — see
`docs/adr/9000-benchmark-strategy.md`. Harness: `crates/taskmesh-bench`.
