# Ticket verification commands

Commands are run from the repository root. Proposed test targets marked `new` must exist and be collected before execution; BG25-012 verifies selection. These are owner-local proofs, not substitutes for the final clean CI-profile receipt.

| Ticket | Pre-change/control | Post-change owner-local |
|---|---|---|
| BG25-001 | `python3 docs/plans/bugbash-sep-25-general/tickets/validate_plan.py` | After an accepted public-contract change: `just doctest`<br>`just rustdoc`<br>`just consumer-msrv` |
| BG25-002 | `cargo test --locked -p taskmesh-contract --test task_plan_validation --test contract_roundtrip` | `cargo test --locked -p taskmesh --test strict_ingress --test strict_ingress_host` (`new`) |
| BG25-003 | `cargo test --locked -p taskmesh-contract --test contract_roundtrip --test snapshot_oracle --test error_surface` | Add the actual consumer-negative target if created, then run `just doctest`<br>`just rustdoc` |
| BG25-004 | `cargo test --locked -p taskmesh-engine --test hardening_child_scope --test hardening_lease_token` | `cargo test --locked -p taskmesh-engine --test cross_governor_ids` (`new`), public facade target, `just consumer-msrv` |
| BG25-005 | `cargo test --locked -p taskmesh --test hardening_executor_protocol --test hardening_executor_authority --test runtime_cpu_executor` | Re-run the same targets plus `cargo test --locked -p taskmesh --features rayon --test hardening_executor_authority` |
| BG25-006 | `cargo test --locked -p taskmesh --test hardening_deadline_custody --test deadline_cancel --test hardening_drain --test runtime_cpu_executor` | Re-run the same targets plus the actual new response-timeline target if split |
| BG25-007 | `cargo test --locked -p taskmesh --test runtime_local --test hardening_dispatch_resolution --test e2e_scenarios` | `cargo test --locked -p taskmesh --test hardening_root_child_scope` (`new`) |
| BG25-008 | `cargo test --locked -p taskmesh-engine --test hardening_exact_accounting --test pending_resolver --test memory_overcommit` | `cargo test --locked -p taskmesh-engine --test hardening_admission_matrix` and host capacity target (`new`) |
| BG25-009 | `cargo test --locked -p taskmesh-engine --test hardening_fairness_reference --test hardening_effect_retirement --test pending_resolver --test differential_model` | Run new queue-history and drain-multiwait targets; modelcheck only under explicit authorization |
| BG25-010 | `cargo test --locked -p taskmesh-engine --test hardening_memory_epochs --test hardening_snapshot_projection`<br>`cargo test --locked -p taskmesh-contract --test snapshot_oracle` | Add and run `hardening_memory_ledger` (`new`), then rerun the controls |
| BG25-011 | `cargo test --locked -p taskmesh-bench --test hellgate --test inferno --test fairness_property` | Run bounded `taskmesh` host-open-loop correctness target (`new`); timing qualification remains separate |
| BG25-012 | `python3 docs/plans/bugbash-sep-25-general/tickets/validate_plan.py` | 104-row scenario-evidence validator, target collection, `just gates-inventory`, `just dev`, then clean isolated `just verify-macos-ci` |

## Final source binding

Before BG25-012 qualification, record `git rev-parse HEAD`, `git rev-parse HEAD^{tree}`, and `git status --porcelain`. After the run, require unchanged HEAD/tree/path digest and validate `target/verification/macos-gates.json`. Do not aggregate owner-local results from different commits.
