# Accepted decisions

BG25-001 accepts these library contracts and compatibility boundaries. External adoption remains owned by each deployment or consumer.

| Decision | Proposed contract | Compatibility boundary |
|---|---|---|
| D1 strict bytes | Add one official bounded strict ingress; keep raw DTO Serde compatibility separate. | Do not silently add `deny_unknown_fields` to public raw DTOs without versioning. |
| D2 awaited child | Strict child input must explicitly contain `parent_awaits`, including explicit `false`. | Internal Rust builders remain `child_of` and `awaited_child_of`. |
| D3 blocking dispatch | Strict input requires `shared_blocking` or `requested_stack { stack_size_bytes }`. | Raw `TaskSpec::stack_size_bytes: Option<_>` remains compatibility surface. |
| D4 parent membership | External planner owns membership in the parent's declared plan. | Moving it into engine requires retained parent-plan generations and a new design. |
| D5 raw IDs | Target owner-bound opaque handles; until migration, document raw APIs as same-Governor scoped. | Public aliases/outcomes may require semver and deprecation. |
| D6 deadlines | `RunFor` remains worker execution budget. `CompleteBy` includes caller response; cleanup may retain the lease after terminal response. | If consumers rely on late success, version or add a distinct response-deadline option. |
| D7 Tokio context | Default blocking/CPU paths reject missing Tokio context before admission with a typed error. | Rayon/custom adapters retain their separately declared context requirements. |
| D8 shared executor | Each runtime governs only its own submissions unless given one explicit shared capacity authority. | Do not create per-engine pools that pretend to enforce an external global bound. |
| D9 wire evolution | Snapshot version and enum-version handling are separate; consumers must handle decode/version failure. | A tolerant envelope requires a separately versioned wire contract. |
