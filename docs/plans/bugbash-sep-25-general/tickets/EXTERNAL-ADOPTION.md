# External consumer adoption ledger

This ledger separates a discovered Rust consumer from deployment acceptance. The source observation below is pinned to the consumer checkout and must be refreshed before a final adoption claim.

## Semantica CodeGraph V2 candidate

- Checkout observation pinned to `/Users/songmin/Documents/code-new/semantica-codegraph-v2` at `f60780bc849ad0ce2cd0f37947f8860b8a38a4c9` (dirty in unrelated paths when inspected on 2026-09-25). The local checkout later moved to `6a3afc319537b3ce6a6a8a452436bc2c3e3865bb`; the observations below were not refreshed at that newer source. The named dependency, governance sources, and focused contract test had no committed diff through `802a7d69e4f43ecf783439eb9600cf8ae1b2d1a9`; this does not qualify a deployed binary.
- Dependency: `packages/analysis/quanta-v2/crates/quanta-runtime/Cargo.toml` has an optional `taskmesh` path dependency on this repository and a `taskmesh-governance` feature.
- Runtime caller: `packages/analysis/quanta-v2/crates/quanta-runtime/src/governance/runtime_adapter.rs` builds one `TokioRuntime` through `Builder` and calls `run_io`, `run_blocking`, `run_cpu`, and requested-stack async paths. `plan_builder.rs` constructs `TaskSpec` in Rust with fixed product class policies.
- Scope search: the inspected `governance/` source and `taskmesh_query_async_support_contract.rs` contain no `parse_task_spec`, `parse_runtime_config`, `child_of`, `awaited_child_of`, or `parent_stage` call. This is a scoped disk-text search, not proof about every Semantica entrypoint or deployed binary.
- Snapshot surface: `governance::snapshot_v1()` returns Taskmesh `Snapshot` as a Rust value. The inspected path does not establish an external serialized wire consumer or its version negotiation.

| Decision | Observed consumer path | Remaining acceptance evidence |
|---|---|---|
| D1 strict bytes | Current inspected adapter constructs typed specs; no untrusted Taskmesh bytes ingress was found in that scope. | Identify any deployed JSON ingress and prove it calls `parse_task_spec`/`parse_runtime_config`, or record this consumer path as not applicable. |
| D2 awaited child | No child submission was found in the inspected governance scope. | Identify deployed child JSON ingress and test explicit `parent_awaits`, or record this consumer path as not applicable. |
| D3 blocking dispatch | Typed Rust builders select blocking and requested-stack paths in the adapter. | If deployed bytes declare dispatch, test explicit `shared_blocking`/`requested_stack` decode and the selected worker domain. |
| D4 parent membership | Inspected builders submit root specs; no child or parent-stage caller was found in that scope. | Identify any external planner that submits children and rejects undeclared stages before submission; otherwise record this consumer path as not applicable. |
| D5 opaque handles | This adapter uses `TokioRuntime`, not raw Governor permit/ticket construction. | Run its feature-specific consumer build/test at the final Taskmesh source; inspect other consumers before declaring migration complete. |
| D6 response and custody | The adapter forwards an absolute deadline through `run_blocking_with`; the consumer's response/custody assumptions are not proved by source inspection. | Run consumer deadline/cancellation tests against the final Taskmesh commit or add a focused consumer assertion if the path is deployed. |
| D7 Tokio context | The inspected adapter constructs one lazily initialized `TokioRuntime`. | Prove the feature-selected caller supplies its required Tokio context or observes the typed preflight refusal. |
| D8 shared executor | No shared custom executor was observed on the inspected path. | Confirm the deployment topology and any externally shared executor capacity authority; record not applicable if the built-in runtime is the only path. |
| D9 wire evolution | A Rust `Snapshot` return exists; serialized/versioned downstream handling was not located in the inspected scope. | Identify actual serialization boundary and test unknown variant/version failure, or record no external wire boundary for this consumer. |

Status: **consumer identified; adoption unverified**. The Taskmesh library's accepted D1–D9 choices do not supply deployment approval, a clean consumer receipt, or proof that this is the only consumer.
