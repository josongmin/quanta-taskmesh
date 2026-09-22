# WS4 — contract, bench, Rayon, and fuzz: actionable findings

Baseline: `7023e945e1c9b3e7e7cca1b26f7af8467df6ab60`.

## TO-07 — Rayon smoke has an unbounded receive (P1)

`crates/taskmesh-rayon/tests/rayon_smoke.rs:13-19` waits with `rx.recv()`. If the adapter accepts but
silently drops or never schedules the closure, the test has no local verdict and waits for the
external job timeout.

Required fix:

- Use `recv_timeout` with a named duration and diagnostic.
- Keep the worker-count and returned-value assertions unchanged.
- Run `cargo test -p taskmesh-rayon --test rayon_smoke`.

## TO-08 — structural USL smoke pays benchmark-scale work (P1)

Measured warm binary times on the current HEAD:

| Binary | Time |
| --- | ---: |
| `fairness_property` | 4.74 s |
| `hellgate` | 15.94 s |
| `inferno` | 0.22 s |

The earlier report ranked `fairness_property` and `inferno` above `hellgate`; measurement rejects
that ordering. `hellgate.rs:140-154` runs `contention_throughput(t, 50_000)` for 1, 2, and 4 threads,
or 350,000 total admit/release cycles. Its oracle requires positive samples, a recoverable
three-point fit, and a real empirical peak; it enforces no throughput threshold.

Required fix:

- Measure this test in isolation across repeated runs without concurrent Cargo work.
- Reduce or cap `ops_per_thread` only if the same three structural assertions remain stable across
  the supported CI hosts.
- Record before/after median and worst-case time. Do not turn timing values into product thresholds.
- Keep full performance measurement in the Criterion/IAI authorities; this test remains a validity
  smoke.

## Already resolved

The previous `wire_formats` finding is closed on this HEAD:

- `fuzz/seed-corpus/wire_formats/` contains tracked positive seeds for `Snapshot`,
  `TopologyConfig`, and `ClassPolicy`.
- `fuzz/corpus-manifest.json` binds their digests.
- `tools/fuzz/producer-manifest.json` requires `snapshot_valid`, `topology_valid`, and
  `class_policy_valid`.
- `tools/fuzz/evidence.py` rejects zero valid inputs or a missing checkpoint.
- `tools/qualification/producer_evidence.py` and receipt tests reject zero/partial witnesses.

TM21-003 is also closed by the current physical shared-blocking domain implementation and its
release proof. `topology_validation.rs` is not waiting on that fix.

## Explicit non-findings

- Keep `contract_roundtrip.rs::conservation_identities_reject_an_inconsistent_projection`. It is a
  cheap integration check that the wire-round-tripped type still exposes conservation validation;
  `snapshot_oracle.rs` owns detailed diagnostic direction. Removing it has negligible cost.
- Keep all per-step snapshots in `inferno`. The whole binary is 0.22 s; sampling every 16th step
  would weaken the hard-cap oracle for no material gate saving.
- Keep the 3,000 fairness seeds until a measured gate budget and an inventory-backed full rail exist.
- Keep all Hellgate load points unless exact duplicate scenario computation is shared. Dropping a
  lambda point reduces regime coverage and is not the primary measured owner established here.
- Contract, doctest, and Rayon adapter tests are negligible on the green path; do not gate-move them.
