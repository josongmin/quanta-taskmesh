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

- the host declares one capability-pool **limit** per registered substrate
  whose pool is topology-sized `> 0` (`PolicySet::with_capability_limits`, set
  by `Builder::build`); the engine refuses a limit for a pool no substrate
  provides, and a pool absent from the inventory is ungated;
- occupancy is accounted **inside the engine**, in the same admission
  transition as class inflight and the resource budget. There is no host-side
  semaphore: a request that cannot have a worker is queued under its own
  class's overflow policy or shed, never parked in a queue that policy cannot
  see (this replaced the former `SubstrateGates`);
- the hint→pool mapping is the contract's `SubstrateHint::capability_pool()`,
  resolved once per submission into a `ResolvedExecutionPlan` (a stack request
  resolves to `large_stack` whichever blocking-family hint carried it), and
  frozen onto the pending record at intake;
- topology (`blocking_threads`, `large_stack_slots`, `local_runtime_slots`,
  `maintenance_workers`, resolved CPU workers) supplies the **slot count**,
  validated by `TopologyConfig::validate` before anything is sized and with
  detected parallelism read once per build; the inventory supplies
  **existence + kind + capability pool**. The two compose:
  inventory ∩ topology = the live limit set, visible as `Snapshot::capabilities`.

So `SubstrateRecord.kind` / `capability_pool` are governance authority (snapshot,
validation, *and* gate derivation), and a `0` slot count means "unlimited" for an
existing pool — distinct from a pool that does not exist at all.

## Structural no-go ratchet

CI runs the same `just` recipes a developer runs (`tools/gates/inventory.json`
lists them; `just gates-inventory` proves Justfile/CI/required-set parity), and
`tools/arch/check_crate_boundaries.py` reads the real `cargo metadata` graph —
every workspace member, every external edge, and feature optionality — rather
than intersecting it with a hardcoded crate list first. These invariants hold:

1. **No raw spawn bypass.** All execution goes through the `Runtime` facade.
   `tokio::task::spawn_blocking` appears in exactly two places, both inside the
   host: behind the `CpuExecutor` port (`BlockingPoolCpuExecutor`, the default
   for `run_cpu`) and in `run_blocking_on_pool` (the `run_blocking` path, which
   is the blocking pool by definition and is governed by the `blocking` /
   `large_stack` / `maintenance` capability gates). Neither is reachable
   without an execution lease. Rayon plugs into the same `CpuExecutor` port.
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

Measurement contract (H16-015/016/017): every measured op asserts its expected
verdict and the harness checks `completed == attempted` and a drained ledger;
the IAI benchmark returns its input so fixture teardown is outside the measured
region; the contention bench times exactly the iteration count Criterion
divides by; the open-loop simulator records one raw latency sample per started
request (no omission back-fill on an open-loop population); workload schedules
are validated (finite, non-negative, non-decreasing, representable) and the
MMPP generator processes phase boundaries before drawing at the new rate.
Thresholds and baseline-compatibility inputs live in `tools/bench/perf-gate.json`
and are read by both local recipes and CI; a run without a compatible IAI
baseline reports `BASELINE_CREATED`, which is not a regression-qualified pass.
The allocation gate parses its producer's structured line strictly (a
malformed number is a failure, never `0`).
