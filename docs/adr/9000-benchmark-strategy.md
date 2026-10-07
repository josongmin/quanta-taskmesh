# 9000. Benchmark measurement and qualification strategy

- Status: Proposed for performance qualification; implemented measurement boundaries below
- Date: 2026-06-04; consolidated 2026-10-07
- Decision owner: Song Min
- Preregistered claim: [claim-contract.json](../../tools/bench/scenarios/claim-contract.json)
- Acquisition: [host series](../benchmarks/host-series.md)
- Open work: [T3](../remaining-work.md#t3--measured-host-performance)

## Context

Taskmesh is a single-process governed execution library. Work-body cost,
governance cost and public-host response/custody need separate populations.
Implemented tooling and structural diagnostics do not establish qualified host
p99/goodput or an industry performance ranking. Date-stamped audit narratives
and old measurements are retained by Git/digest in
[document history](../evidence/document-history.json).

## Decision

| Rail | Population / authority |
|---|---|
| Governor micro | Named admission/claim/release/snapshot operations with explicit fixture/setup denominator; ns/op, allocation or Linux IAI only. |
| Deterministic policy model | Seeded event order, virtual wait and conservation oracle; simulator computation time is not host latency. |
| Public host | Finite public `run_*` calls with intended/submitted/response/body/custody raw rows; only this rail measures host response/goodput. |
| Performance qualification | Preregistered repeated matched host measurements, controls, complete failed attempts and run-level uncertainty; single CI wall-clock samples cannot qualify it. |

### Population and accounting

- Open-loop records one raw row for every intended offer. Producer cap/lag leaves
  `not_submitted` rows. Preserve reject/cancel/deadline/drop/unanswered outcomes;
  missing responses do not receive zero latency. Closed-loop fixed concurrency
  has no exogenous intended schedule and remains a separate population.
- SLO-goodput is successful injection-cohort responses within each class/path SLO
  divided by injection-window seconds. Also report the same successes per total
  intended arrivals, completion fraction, and conditional successful p50/p95/p99
  with sample count and precision. Requests are not independent run replicates.
- Caller completion/drop and body finish/lease release are separate ledgers.
  Settled warmup precedes the measured baseline. Final per-class ledger deltas
  satisfy `started == body-start rows`, `started <= admitted <= submitted`, and
  `terminated == admitted` after settlement. Admission followed by cancellation
  before body start is valid. Classes with no offers have zero admission/termination.
  Closed-loop calls belong to `template.class`; successful rows match its started
  counter, and unused registered classes remain zero. Overall sums cannot replace
  per-class attribution. Rust and retained-artifact validators enforce these bounds.
- Sampled Snapshot/RSS/thread maxima are lower bounds, not exact high-water marks.
  Preserve sampled intervals and cumulative CPU deltas; one CPU sample supplies
  no delta. Finite drain does not prove longitudinal memory stability.
- Do not add histogram-corrected completions to an already complete intended
  population. Missing/failed raw rows remain visible instead of being synthesized
  into observed successes.

### Preregistered claim and controls

- Before measurement freeze SLO, completion floor, absolute rate grid, precision,
  minimum detectable effect, peers and exclusion rules. Missing B00 values remain
  `blocked`; synthetic load cannot substitute for representative consumer H7.
- Null-work generator headroom replays the finite schedule twice in the same
  window. Same-rate replay is not headroom. Compare lag/missing offers against
  preregistered measured budgets; a structural boolean is insufficient.
- Use same-binary A/A plus balanced Snapshot, full/minimal recorder and external
  sampler on/off arms. Bind cadence and all derived workload digests. Minimal
  recorder forbids Snapshot and does not create request latency. Preserve whole
  probe duration distortion, response counts and per-class/path minimum samples.
- Hold host, build feature, topology, class policy, work bodies and proportional
  offer mix fixed across rates; only arrival timestamps and population-dependent
  `max_records` vary. Baseline/candidate source revision is the declared comparison
  variable. Reject overlapping/reused runs, incomplete populations, changed
  workload/host and unbalanced chronological order.
- Control-budget PASS remains diagnostic `UNQUALIFIED`. Re-read artifact digest
  checks prevent bundle replacement from reusing a verdict. `host_compare.py`
  produces descriptive differences/bootstrap ranges and always `UNQUALIFIED`.
  Measured series admission is separately owned by `host_admission.py`.
- Tokio/Semaphore/Tower are narrower mechanism controls. An industry peer must
  match bounded admission/queue/deadline/cancel/custody/drain, or the claim must
  narrow to their common contract. Industry comparisons require independent rerun.

### Pacing and recorded submission

`max_producer_lag_ns` is pacing observation minus intended arrival. Measured
admission checks every reconstructed rate's lag against its hash-bound control
policy `max_host_lag_ns` before admission. The separate
`max_recorded_submit_lateness_ns` is max recorded submission minus intended time,
or `null` with no submitted row. Submission precedes spawning the public API call,
so it is a lower bound on invocation lateness. A submission/invocation-jitter
claim needs its own measurement boundary and preregistered budget; the pacing
policy does not supply it.

### Source and artifact custody

Bind source bytes/modes, effective compiler/wrapper/linker bytes, feature, binary,
scenario/raw/topology and host endpoints before and after build/oracle execution.
Revalidation runs private copies of digest-checked regular inputs with owned
process-group supervision. Apply each caller's confinement rules and reject
nonregular descriptors before reads; study-ledger symlink replacements reject.
Timeout, interruption, orphaned group members and partial output cannot pass.
Original-group cleanup cannot reclaim descendants that escaped to a new PGID;
regular-file kernel/network I/O stalls remain outside the bounded contract.
Create-only publication preserves failed/incomplete populations and refuses
replacement of existing artifacts; it does not promise crash durability.

## Implemented rails and their limits

- Criterion separates full-facade/prebuilt-spec governance tax, queue promotion,
  cold/warm host lifecycle and completed batch denominators. `*_sim` measures
  simulator computation. One explicit `bench_suite` target retains reviewed module
  membership. Allocation/IAI thresholds and baseline identity live in
  [perf-gate.json](../../tools/bench/perf-gate.json); first IAI baseline creation is
  not regression PASS. `bench-gate`, `bench-smoke` and `bench-iai` do not qualify
  public-host performance.
- Full-host raw v2 records Send IO/blocking/default CPU and requested-stack paths,
  caller/body timing, cut/settlement, topology, Snapshot and failed `invalid` raw.
  Current measured admission is restricted to IO/blocking/CPU; requested-stack
  diagnostics cannot expand its scope. Default/Rayon builds have distinct identities.
- Closed-loop has bounded caller concurrency; local raw v3 executes borrowed/`!Send`
  payloads on a current-thread LocalSet and records pacing/cancel/drop/deadline and
  settlement. Local v1/v2 cannot be promoted to v3. Composite H6 declares reduce
  metadata while the caller submits/awaits three public child calls and reduces
  successes; partial failure raw remains invalid evidence. These modes have typed
  structural diagnostics, not repeated performance admission.
- Special-mode booleans are not measured calibration; performance promotion fails
  closed. Frozen cold rebuilds and complete attempt accounting do not themselves
  supply B00, fixed-host repetitions, observer calibration, representative H7,
  matched peers or independent rerun.
### Recovery diagnostics

B07 recovery uses one runtime/warmup, bounded overload cycles, exact zero-owned
  checkpoints without intermediate drain, real-body canaries per used class/path,
  cumulative accounting and one final drain. Replay checks the whole indexed
  population; failed or interrupted attempts cannot publish success. The retained
  runner is digest-checked data, not external replay authority. Current-checkout
  validation reports structural PASS; current-source mode additionally requires
  stable endpoints and byte equality with the retained runner. Functional recovery
  does not supply RSS slope, long-term memory or recovery-SLO qualification.

## Consequences

Measurement costs increase, but each result keeps its denominator, source and
failure population. P1–P9 code references denote arrival, micro-cost, scale model,
workload, behavior, concurrency oracle, environment, regression infrastructure
and mechanism controls respectively; they are provenance labels, not completion
states. CI correctness, structural diagnostics and performance qualification keep
separate verdicts. [Remaining work](../remaining-work.md) owns unresolved inputs.
