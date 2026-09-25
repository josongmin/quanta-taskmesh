# Taskmesh benchmark qualification: source audit and work plan

Status: **plan; host performance is not qualified**. Audited clean `main@7253fab2002f15a65a31a2070641f0c18498912f` on 2026-09-26. This was a static source audit; no benchmark, test, or gate was run for this plan. Existing historical measurements and receipts do not establish performance for this HEAD.

## Measurement boundary

Taskmesh is a governed execution library. The measured product paths are `Governor::{admit,claim,release,snapshot}` and `TokioRuntime::{run_io,run_blocking,run_cpu,run_local,run_async_with_requested_stack}` with their class policies, capability topology, cancellation/deadline custody, drain, and composite stage behavior. HTTP, database, retrieval quality, and generic server RPS are outside this benchmark. A workload body may model CPU, blocking, or async work, but its own speed must be reported separately from Taskmesh overhead.

There are three distinct claims:

1. **Engine micro-cost:** governor transition cost, allocations, instruction count, and contention. This does not establish host response latency.
2. **Deterministic policy behavior:** admission, queue bounds, fairness order, conservation, and host/simulator count agreement on a finite fixture. This does not establish sustained wall-clock capacity.
3. **Host performance:** actual public facade calls under scheduled arrival load, with terminal accounting, per-class/path latency, goodput, and worker custody. This claim is currently open.

Criterion and IAI remain Taskmesh's existing engine micro-cost tools. The host-load design below comes from execution-engine benchmark cases and runs against Taskmesh's **in-process** API.

## External execution-engine benchmark precedents

These are source-inspected **benchmark cases as of 2026-09-26**, not performance numbers or thresholds to import. Tokio and Asupersync are in-process runtime analogues; Temporal is a larger worker/task-queue service whose load-shape and accounting method transfers but whose network/persistence latency does not.

| External case and inspected source | Actual benchmark design | Taskmesh mapping and current gap |
|---|---|---|
| [Tokio `spawn.rs`](https://github.com/tokio-rs/tokio/blob/master/benches/spawn.rs) | One vs ten spawned tasks, current-thread vs threaded runtime, enqueue plus await completion. | Retain one-task no-op cost, add small-batch `run_*` completion per operation with identical runtime/work body/spec-creation boundaries. Current `governance_tax` is sequential and its raw comparison includes different setup. |
| [Asupersync scheduler](https://github.com/Dicklesworthstone/asupersync/blob/main/benches/scheduler_benchmark.rs), [spawn/adversarial](https://github.com/Dicklesworthstone/asupersync/blob/main/benches/spawn_throughput.rs) | Queue/schedule/pop cases at varied population, mixed-lane order, spawn/join throughput, quota-denial and cancel-storm request tails, teardown with pending tasks. | Add Governor queue promotion cost over class count/depth, then real-host cancellation/rejection storms and drain-settlement latency. Copy the *measurement split*, not its lane or priority semantics; Taskmesh fairness policies and custody are different. |
| [Temporal Maru](https://github.com/temporalio/maru) | Rate-limited steady/spike steps with configured count and driver concurrency; reports interval `started`, `closed`, and backlog. **Archived in 2024.** | For Taskmesh's in-process host, use offered-rate steps and interval success/rejection/backlog/custody time series. Record producer saturation separately; no Temporal cluster, RPC, or persistence benchmark is implied. |
| [Temporal benchmark-workers](https://github.com/temporalio/benchmark-workers), [latency suite](https://github.com/temporalio/benchmark-latency) | A separate runner maintains fixed concurrency; the low-load suite distinguishes schedule-to-start from end-to-end latency. The latency suite is **archived in 2025**. | Add an explicitly **closed-loop fixed-concurrency** throughput mode for comparison with the open-loop overload mode; never use its completion-only latency to claim overload p99. Expose Taskmesh intended-submit-to-body-start and full caller-response distributions separately. |

Ray's distributed node/object-store scalability envelope and Temporal's persistence/RPC absolute latencies are excluded: Taskmesh is a single-process governed execution library. No external throughput number, queue size, or latency SLO transfers to this engine.

## As-is audit

| Existing asset | What it proves or measures | Limit |
|---|---|---|
| `admit_release`, `contention`, `iai_governance`, `alloc_probe` | Governor micro-cost and lock contention; CI allocation gate is `<= 8 alloc/op` for one admit/release cycle; Linux IAI config names three cases and `Ir=5.0`. | No host work, queue residence, or end-to-end latency. The allocation threshold is a current config, not a measurement in this audit. |
| `governance_tax`, `host_edge_paths` | Sequential public-host no-op and selected deadline/saturation paths; raw Tokio, Semaphore, and Tower comparison. | The governed case creates `TaskSpec` inside the timed loop while raw Tokio does not; this is full caller cost, not isolated governance tax. No sustained concurrent host load. |
| `retrieval_saturation`, `overload_stability`, `multiclass_fairness`, `composite_reduce` | Simulator computation, deterministic promotion behavior, and child-admission/reduce-validation micro-cost. | Criterion time for the `*_sim` benches is time spent **running the simulator**, not simulated p99 or host latency. Fairness is an order property, not a host fairness-throughput result. Composite bench does not run a full parallel fan-out/reduce. |
| `workload`, `loadgen`, `host_simulator_comparison` | Seeded, validated arrival schedules and virtual admission-wait samples; finite nine-offer host/simulator admission accounting. | The simulator has no real Tokio scheduling or worker queue. The nine-offer host fixture has no paced sustained performance sweep. CSV records intended time and class only. |
| `bench-gate`, `bench-smoke`, `bench-iai` | Allocation regression and build/smoke in the CI profile; Linux IAI in nightly/manual deep workflow. | `bench-smoke` is not a latency gate. IAI first compatible run may only create a baseline. `.github/workflows/bench.yml` is manual and disabled for hosted automatic use. There is no qualified host wall-clock regression gate. |

`docs/adr/9000-benchmark-strategy.md` is a **Proposed** June north-star document, not a current implementation receipt. Its implementation-status text claims a five-allocation baseline, automatic dashboard/flamegraph jobs, and dropped-sample accounting that the current config, workflow, and load generator do not support. Its generic `< X%` governance-tax target is unset. Reconcile that text before citing it in a performance decision. Do not equate IAI instruction count with wall time or machine-independent application performance.

## Engine readiness

| Need | Current source | Decision |
|---|---|---|
| Drive all public paths | Builder constructs the host; `Runtime` exposes IO, blocking, CPU, local, and snapshot; the host also has a requested-stack async path and `_with` options. | **Ready** for black-box host workloads. No engine rewrite prerequisite. |
| Stable policy/topology | Builder validates and freezes resolved capability inventory. Without the `rayon` feature the default CPU executor shares Tokio's blocking physical domain; with it CPU uses a Rayon pool. External shared Rayon pools declare nonexclusive ownership. | **Ready**, but every receipt must record features, resolved topology, executor descriptor, and ambient-pool caveat. Compare these configurations separately. |
| Deterministic policy model | Governor and `ManualClock` support reproducible admission simulations. | **Ready** for policy oracles. `Clock::now_ms` is a wall/lease clock and explicitly is not an elapsed-time timer; host latency must use `Instant`. |
| Host observability | `Snapshot` has per-class queue/phase gauges, aggregate counters and capability occupancy; conservation check exists. | **Partly ready**. Aggregate snapshots do not timestamp an individual request. Use bench-owned intended-send, actual-submit, body-start, caller-terminal, and body-finish timestamps. Sample snapshots sparingly outside the timed critical path; quantify observation overhead. Add an internal optional phase event hook only if black-box measurements cannot answer a defined question. |
| Terminal and custody semantics | Blocking work can continue after caller deadline/cancel; drain and phase gauges retain worker ownership. | **Ready** to measure, but a benchmark must wait for post-response worker settlement and report it separately from caller latency. |

No new public Taskmesh API, semantic class, or engine-specific pool is needed to start. Unknown classes must remain rejected; a benchmark fixture must not silently register or admit them.

## To-be benchmark layout

| Lane | Entry point and code | Timed operation | Result and authority |
|---|---|---|---|
| A. Governor micro | Existing Criterion `admit_release`, `contention`, `multiclass_fairness`; add `queue_scaling`; existing allocation probe and Linux IAI | One explicit transition or admitted/released cycle; setup outside sample; class count, queue depth, and thread count declared | ns/op plus exact completed-op denominator, alloc/op, and named instruction cases. Regression signal for the engine only. |
| B. Policy model | Existing `workload.rs` + `loadgen.rs` and deterministic tests | Virtual admission/claim/release against validated fixed-rate fixtures or seeded Poisson/Zipf and MMPP burst arrivals | Conservation, promotion order, rejection, virtual *admission wait*. Never label simulator execution time or virtual wait as host latency. |
| C. Public host | New `host_load.rs` + `examples/host_load_probe.rs`, driving `TokioRuntime` public methods | Scheduled arrivals and completion of caller-owned work; `Instant` timestamps and finite outstanding cap | Raw per-request outcome, latency, and interval counters; sampled Snapshot; exact workload/topology receipt. This is the only lane for host p99/goodput. |
| D. Qualification | Existing CI allocation/smoke and Linux IAI; later `tools/bench/host_perf.py` on fixed quiet host | Comparable raw receipts for the same scenario | Missing/partial/incompatible runs reject. Host timing is diagnostic until repeated baselines establish variance and an explicit regression policy is approved. |

```mermaid
flowchart LR
    F[Versioned Taskmesh scenario<br/>class policy, topology, path, work body, seed] --> W[ValidatedArrivals]
    W --> S[Deterministic Governor simulator]
    W --> P[Bounded monotonic-time producer]
    P --> H[Public TokioRuntime run_*]
    H --> O[Caller terminal + worker-finish recorder]
    H --> G[Sampled Snapshot observer]
    S --> R[Policy-only result]
    O --> Q[Versioned raw host receipt]
    G --> Q
    Q --> C[Conservation and comparability checker]
```

### Host harness contract

- **Producer:** Precompute and validate a finite schedule before timing. A dedicated pacing thread uses `Instant` and submits Send paths to a fixed Tokio runtime; it never waits for the prior request's response. Cap pending submissions explicitly. If the cap is reached, count `not_submitted` and mark that rate point **generator-limited**, not a Taskmesh capacity result. `run_local` needs a separate caller-affine adapter because its future is `!Send`; requested-stack execution is a separate path fixture.
- **Scenario source:** Versioned, validated JSON fixtures under `tools/bench/scenarios/` specify path, class policies, resolved topology expectation, body kind/work amount, arrival mode and frozen rates, seed, warmup, sampling window, outstanding cap, and settlement timeout. The runner rejects unknown fields/classes/path-body mismatches before timing. The serialized fixture digest is part of every receipt.
- **Load modes:** A fixed-rate staircase establishes idle/steady/knee/overload; a seeded burst shows backlog and recovery. Run a separate closed-loop fixed-concurrency sweep for completion capacity. The two modes have different latency populations and cannot share one p99 series.
- **Body fixtures:** No-op for control cost; declared fixed CPU work for `run_cpu`; finite blocking work for `run_blocking`; awaited async work for `run_io`; local and requested-stack bodies only in their supported path fixtures. Record body-only duration. Never equate a sleeping body with CPU work or an IO future with a dedicated IO pool.
- **Timestamps:** For each request record intended send, actual submit, body start if any, caller terminal, and body finish if any. `actual_submit -> body_start` includes Tokio scheduling and Taskmesh admission, plus executor dispatch where applicable; `intended_send -> body_start` also includes producer lag. Exact governor-queue time requires the conditional B05 hook.
- **Populations:** Success end-to-end, typed rejection response, timeout/cancel response, and worker-after-response custody have separate distributions and counts per class/path. Compute goodput from successful terminal outcomes per measured wall time; report intended and actual offered rates separately. A quantile is published only with its stated sample count and predefined minimum population.
- **Conservation:** Check `intended = submitted + not_submitted`, `submitted = terminal + unanswered`, and final Snapshot conservation/zero owned resources after settlement. Warm up the same runtime, wait for its work to settle, then start fresh counters/histograms; take Snapshot deltas because engine totals include warmup. After scheduled injection ends, collect caller terminals and time `drain` separately to observe workers still holding custody; drain is one-way.
- **Observation:** Sample Snapshot at a fixed, documented cadence and label sampled peaks as lower bounds. Compare observer-on/off cost before using dense sampling. Never sample `snapshot()` on every request inside a hot-path timing span.
- **Artifacts:** Preallocate one record slot per intended request under a configured maximum; fill by request ID without a shared hot-path logger, then write the raw event artifact after timing. Quantify recorder-on/off overhead. Emit a schema-versioned summary with source/toolchain/host/features/topology/scenario digests, outcome counts, histogram populations, interval series, and raw-artifact digest. A summary without its raw artifact is not a qualifying measurement.
- **Comparators:** `run_blocking` vs raw `spawn_blocking` on the same Tokio runtime and body; `run_cpu` vs the same declared CPU executor/physical domain; `run_io` vs the same async body with a capacity-only control; `run_local` vs the same local caller path. Keep raw, capacity-only, and governed results separately labeled because their semantics differ. Match spec creation inside or outside every compared timed span.

**First release of the harness:** implement Send `run_io`, `run_blocking`, and default `run_cpu`, plus terminal/custody accounting. Add `run_local`, requested-stack, and Rayon feature fixtures after that denominator is stable. This keeps the first result reviewable without dropping those paths from the final matrix.

### Versioned fixture matrix

`R` below is a pilot rate recorded once for a fixed host, path, body, and topology before the candidate comparison. Freeze the resulting absolute rates in the scenario file; do not retune the baseline and candidate independently. The values are fixture design, not performance targets.

| ID | Fixture | Load and comparison | Question answered |
|---|---|---|---|
| M1 | One registered class, no-op body; class-count series 1/8/64 and queue-depth series 0/16/128, varying one dimension at a time | Governor admit/reject/queue/claim/release; 1/2/4/8 threads capped at available cores | Where do lock contention, inventory size, and promotion cost become material? |
| H0 | Each supported public path, no-op then one declared finite body | One request and batch of ten, raw/capacity-only/governed controls | Full facade tax and its setup boundary, not a policy-capacity claim |
| H1 | One class/path/body/topology per run | Open-loop fixed rates `0.25R/0.5R/0.75R/R/1.25R/2R`; separate closed-loop concurrency sweep | Success goodput, per-outcome latency, queue growth, rejection and capacity knee |
| H2 | Same H1 fixture | Base rate → short `2R` burst → base-rate recovery, with fixed time buckets | Backlog growth, bounded shedding, time to settle and post-burst latency |
| H3 | Two registered generic classes `interactive` and `batch`, both valid WFQ policies with declared 4:1 weights | Equal-cost and skewed-cost cases; overload `batch` while `interactive` continues | Protected-class tail, windowed service share, starvation, typed rejection |
| H4 | Matched CPU+blocking bodies | Default shared-blocking physical domain vs separate `rayon` feature build; requested-stack/local in separate fixtures | Physical-pool interference and whether capability occupancy explains the tail |
| H5 | Started blocking work plus queued followers | Deadline/cancel/caller drop burst, then graceful drain | Caller response vs worker-finish gap, retained lease, drain settlement and zero final custody |
| H6 | Parent plus separately submitted governed children | Skewed child times, one failure/cancel, caller-owned deterministic reduce | Per-child governance and parent end-to-end cost without claiming Taskmesh executes the reducer |

Pilot feedback can use shorter runs. The initial performance qualification contract is 30 seconds of warmup, at least 60 seconds of measurement, and five independent repetitions per frozen rate point; each repetition builds and warms a fresh runtime. Publish a p99 for one class/path/outcome only when that population has at least 10,000 samples **in each repetition**; extend duration instead of printing a thin-tail percentile. Revisit these durations after measuring dedicated-host variance; they are initial acquisition parameters, not an automatic PR latency threshold.

### Command and gate placement

| Scope | Command | Status and meaning |
|---|---|---|
| Owner edit loop | Selected `cargo bench --locked -p taskmesh-bench --bench <name>` | Existing; micro diagnostic only. Simulator benches must carry simulator labels. |
| CI profile | `just bench-gate`, `just bench-smoke` | Existing allocation gate and bench execution smoke; no host wall-clock pass. |
| Linux deep profile | `just bench-iai` | Existing named instruction cases with compatible-baseline requirement; first baseline creation is not a regression-qualified result. |
| Host exploration | Proposed `just bench-host <scenario-id>` | New bounded public-host runner; writes raw and summary artifacts, never updates CI required membership. |
| Performance qualification | Proposed `just bench-host-compare <baseline-receipt> <candidate-receipt>` | New fixed-host, exact-definition comparison after variance is established; a separate performance receipt, not automatic PR CI. |

## Required Taskmesh scenario set

| Priority | Scenario | Configuration and comparison | Required output |
|---|---|---|---|
| P0 | Governor cost | Admission success/reject, queue/claim/release, snapshot; 1/2/4/8/... physical-core contention; controlled class count and queue depth for promotion. | ns/op, alloc/op, IAI instructions where available, completed operations, contention and queue-cost curves. Keep simulator computation in its own category. |
| P0 | Real host capacity | `run_io`, `run_blocking`, `run_cpu`, `run_local` separately, under idle, rising load, knee, and overload; fixed work body and explicit class/capability limits. Open-loop rate steps for overload, separately labeled closed-loop fixed concurrency for completion capacity. | Intended/offered/submitted/started/terminal/unanswered counts, success goodput, reject by verdict, p50/p95/p99 and sample denominator per class/path, scheduler send lag, interval counts, sampled queue/phase peaks (lower bounds). |
| P0 | Backpressure and ownership | Bounded class/capability queues, deadline, cancellation, caller drop, started blocking worker, cancellation burst, drain. | Queue bound, terminal and unanswered conservation, caller-response latency, worker-finish latency, post-response custody, drain-settlement latency, final zero owned resources and recovery after burst. |
| P1 | Cross-class policy | FIFO vs weighted/deficit/deadline-aware configurations, mixed class costs, an overloaded noisy class beside a protected class. | Per-class success goodput and tail latency, rejection, share over fixed windows, starvation/priority inversion evidence. Do not use a final-total Jain score alone. |
| P1 | Physical topology | Default CPU-on-blocking and `rayon` feature CPU pool, blocking+CPU competition, requested-stack blocking/async and local slots; external shared Rayon only as a separately labeled ambient-interference case. `run_io` has no dedicated IO pool; any bound is class policy. | Resolved physical domains, configured and observed occupancy, each class/path result. Never interpret ambient external work as fully governed by Taskmesh. |
| P1 | Composite governance | Caller-owned parallel child submissions with declared deterministic reduce, skewed child durations and one failing/cancelled child. Taskmesh validates the declaration and governs separately submitted calls; it does not spawn or reduce the branches. | Parent/child admission and custody, caller-owned reduce result, end-to-end and child latency, outstanding custody at terminal/drain. Label orchestration/reducer time as caller cost. |
| P2 | Less frequent policy costs | Memory reconciliation/overcommit and maintenance saturation using real source-supported paths. | Event cost and rejection/settlement behavior. Promote to P1 only if a concrete consumer workload uses the path. |

Use a small declared matrix, not the Cartesian product of all policies and executors. Fix task body and compare the same path, concurrency, budget, features, and completion contract. Raw `spawn_blocking`/Semaphore/Tower are *mechanism baselines* with fewer semantics; label any difference as total API cost unless submission/spec construction and topology are matched.

## Tickets in execution order

### B01 — Repair and extend engine microbenchmarks

- **Purpose:** Make each existing result's population and unit explicit; measure Taskmesh queue scaling and small-batch facade cost before interpreting host load results.
- **Files:** `docs/adr/9000-benchmark-strategy.md`, `crates/taskmesh-bench/benches/{retrieval_saturation,overload_stability,governance_tax,host_edge_paths}.rs`, new `crates/taskmesh-bench/benches/queue_scaling.rs` and `crates/taskmesh-bench/Cargo.toml` if needed, this plan; `tools/bench/perf-gate.json` only if an approved measurement definition actually changes.
- **DoD:** ADR names current `8 alloc/op` config, manual/disabled bench workflow, raw started-request simulator samples, and no current host performance gate. `*_sim` output/documentation says simulator execution cost. Governance comparison states what timed setup differs; add a prebuilt-spec matched variant without deleting full API cost. Measure a fixed 1-vs-10 completed-call batch on the same path/runtime/body, and queue promotion over declared class-count/depth points with setup outside the timed region. Publish per-operation denominators; assert every operation reached the intended admitted/queued/terminal path. No old `< X%` becomes a gate by inference.

### B02 — Build a bounded, paced public-host load harness

- **Purpose:** Reuse `ValidatedArrivals` as intended arrival times but submit real `TokioRuntime` work through public methods. A synthetic simulator result remains a separate oracle.
- **Files:** new `crates/taskmesh-bench/src/{host_load,host_scenarios}.rs`, `crates/taskmesh-bench/examples/host_load_probe.rs`, `tools/bench/scenarios/*.json`, focused `crates/taskmesh-bench/tests/host_load_accounting.rs`, `crates/taskmesh-bench/Cargo.toml`, and crate exports as needed. Keep scenario-only body/path/options metadata in the bench crate; do not widen product `TaskSpec` for a benchmark.
- **DoD:** Implement the host harness contract above. A bounded finite schedule is paced from a monotonic run origin; submissions do not wait for prior responses. At a producer cap, record `not_submitted` and scheduled lag explicitly instead of building an unbounded competing queue or silently reducing offered load. A separate fixed-concurrency mode may submit after completion but is labeled closed-loop and never supplies overload-tail claims. Record one terminal classification per submitted request, or an explicit unanswered-at-deadline count; separate warmup from samples. Collect intended send, actual submit, body start, caller terminal, body finish, and final drain; `offered = submitted + not_submitted` and `submitted = terminal + unanswered` are checked. Report interval counts as well as run totals so a spike and its recovery remain visible. Per-class/path samples state whether rejects and timeouts belong to the population. Synthetic accounting fixtures cover a delayed producer, rejection, timeout/cancel, and caller response before worker finish.

### B03 — Run the Taskmesh scenario matrix and establish honest baselines

- **Purpose:** Characterize capacity and failure behavior on the real host; distinguish governance cost from CPU/body cost and simulator behavior.
- **Files:** bench-owned scenario fixtures/examples under `crates/taskmesh-bench/`, `tools/bench/host_perf.py` report parser/checker, source-backed scenario documentation in this plan.
- **DoD:** Run P0 before P1. A declared fixed workload is swept across arrival rate with repeat runs on the same quiet host, including a low-rate baseline, steady saturation, spike and recovery. Use fixed concurrency as a separate capacity comparison. Report per path/class success goodput, tail sample counts, reject types, unanswered, send lag, interval started/terminal counts, sampled queue/capability peaks (explicit lower bounds, never exact high-water claims), custody after response, and final conservation. Record clean HEAD, tree/lock digest, rustc, features, CPU/OS, resolved topology, seed/trace digest, work-body definition, warmup, measurement duration, and raw results. Compare only compatible runs. A knee or throughput limit is reported only from sustained measured points and an explicit application SLO, not a fitted USL extrapolation alone.

### B04 — Add performance qualification after a stable measurement contract

- **Purpose:** Prevent noisy hosted timing from becoming a misleading PR gate while retaining cheap correctness checks.
- **Files:** `tools/bench/` receipt/parser and configuration, `Justfile`, `tools/gates/{inventory,required}.json` only if a new required gate is intentionally adopted, workflow only after runner ownership is decided, release checklist/ADR.
- **DoD:** Keep existing CI allocation/smoke and Linux IAI scopes. Put host wall-clock measurement on a documented quiet, fixed host in a distinct manual or scheduled performance lane; first collect repeated baseline and variance. A regression rule states comparable fingerprint, minimum sample count, confidence/noise policy, and failure on missing or partial raw data. A new required gate needs explicit inventory/required membership and review; no dashboard or alert is counted as proof. A first IAI `BASELINE_CREATED` receipt is not `QUALIFIED`.

### Conditional B05 — Add low-overhead internal phase timestamps

- **Trigger:** B02/B03 show that an actionable regression cannot be separated between governor queue, executor acceptance, and work start using public closure timestamps and sparse snapshots.
- **Files:** narrowly scoped internal host/engine observation hook plus bench adapter and overhead comparison; update public specification only if a public contract change is independently justified.
- **DoD:** Disabled path has measured negligible overhead; events are bounded, never call an observer under the governor lock, preserve fail-closed behavior and phase accounting, and do not create an unbounded telemetry queue. Keep semantic policy separate from worker governance.

## Proof and stop rules

- `just bench-smoke` proves the bench targets execute, `just bench-gate` proves the configured allocation threshold, and a compatible `just bench-iai` result proves only its named instruction cases. None substitutes for B03 host measurements.
- Any performance claim must cite exact source, machine, features/topology, workload/seed, population, denominator, raw receipt, and comparator. Reject runs with missing terminals, generator saturation hidden as service latency, unstable source, or incompatible baseline.
- No mutation/nightly/release command is part of this plan's static audit. Benchmark execution and gate changes are separate follow-up implementation/qualification work.
