# Structural target

## AS-IS risks

1. Raw Serde DTOs can be used as execution/configuration authority even though they intentionally accept defaults and unknown fields.
2. Executor capabilities are validated at build but not frozen as runtime authority.
3. Raw permit/ticket identifiers contain no Governor ownership proof.
4. Requested-stack response ownership is entangled with owned-runtime teardown.
5. Parent-plan membership and detached-child lifetime have no explicit external owner contract.
6. Strong isolated tests exist, but scenario-to-gate provenance is not machine-readable for the 53 incomplete scenarios.

## TO-BE boundaries

- **Raw compatibility DTO:** preserves existing Serde behavior and is never described as safe untrusted ingress.
- **Strict ingress adapter:** owns pre-parse byte limit, depth/stage limit, duplicate/unknown key rejection, explicit wait declaration, explicit blocking dispatch tag, and typed conversion to validated plan/config.
- **Frozen execution descriptor:** Builder reads and validates `ExecutorCapabilities` once; runtime plan, debug, snapshot-facing accessor, and dispatch use the same stored value.
- **Owner-bound identity:** authorization handles contain unforgeable Governor authority plus a local sequence; display/telemetry IDs remain separate values.
- **Response arbiter:** caller response, root completion, owned-runtime teardown, child completion, and lease release are separate events. `RunFor` and `CompleteBy` keep distinct contracts.
- **Planner boundary:** external planner owns cross-plan parent-stage membership; engine owns exact live-parent generation, recursion, and declared wait-cycle prevention.
- **Single capacity authority:** semantic and physical requirements remain one engine transition. No executor-specific pool proliferation or second unbounded scheduling queue.
- **Evidence inventory:** each scenario records the exact target, case, feature/platform conditions, oracle, selected gate, and source digest.

## Invariants preserved during implementation

- Unknown classes/pools and malformed authoritative inputs fail closed.
- Existing queued work may settle after close; no new valid admission succeeds.
- Worker custody outlives caller response when real work remains.
- Parallel declaration never implies Taskmesh executes branches or reducers.
- Fairness and policy governance remain separate from worker/executor governance.
- Snapshot values are observations, not the independent oracle used to prove their own correctness.
