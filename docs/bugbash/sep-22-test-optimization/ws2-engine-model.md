# WS2 — engine model tests: no actionable optimization

Baseline: `7023e945e1c9b3e7e7cca1b26f7af8467df6ab60`.

No merge, trim, or gate move remains justified in `crates/taskmesh-engine/tests/`.

## Keep the current proof topology

- `hardening_fairness_reference.rs`: deterministic independent fairness oracle; the smaller fairness
  scenario tests do not subsume it.
- `differential_model.rs`: independent executable-spec agreement after every operation. The file
  documents why 1,024 cases are retained.
- `loom_governance.rs`: bounded exhaustive interleavings, already isolated behind `just loom`.
- `shuttle_governance.rs`: larger-state fixed-seed schedule sampling, already isolated behind
  `just shuttle`.
- `concurrency_stress.rs` and `concurrency_fuzz.rs`: real OS-thread invariants and memory-mode paths
  not covered by Loom/Shuttle's modeled scheduling.
- `harness/mod.rs`: shared fixture infrastructure only; merging test targets would reduce failure
  attribution without removing executed work.

## Reopen condition

Reopen WS2 only with per-binary timing or flake evidence from the current HEAD. Any proposed seed or
schedule reduction must retain an inventory-backed full rail and show that the default rail is the
measured bottleneck. Inspection-only operation counts are insufficient.
