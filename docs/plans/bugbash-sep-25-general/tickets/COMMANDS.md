# Ticket verification commands

Commands are run from the repository root. Every listed target exists and is selected by the registered gates. These are owner-local proofs; the clean CI-profile receipt remains the qualification authority.

| Ticket | Pre-change/control | Post-change owner-local |
|---|---|---|
| BG25-001 | `python3 docs/plans/bugbash-sep-25-general/tickets/validate_plan.py` | After an accepted public-contract change: `just doctest`<br>`just rustdoc`<br>`just consumer-msrv` |
| BG25-002 | `cargo test --locked -p taskmesh-contract --test task_plan_validation --test contract_roundtrip` | `cargo test --locked -p taskmesh --test strict_ingress --test config_inventory`<br>`cargo test --locked -p taskmesh --features rayon --test strict_ingress` |
| BG25-003 | `cargo test --locked -p taskmesh-contract --test contract_roundtrip --test snapshot_oracle --test error_surface` | `just doctest`<br>`just rustdoc` |
| BG25-004 | `cargo test --locked -p taskmesh-engine --test hardening_child_scope --test hardening_lease_token --test cross_governor_ids` | `cargo test --locked -p taskmesh-engine --test identity_authority --test cross_governor_ids`<br>`just consumer-msrv` |
| BG25-005 | `cargo test --locked -p taskmesh --test hardening_executor_protocol --test hardening_executor_authority --test runtime_cpu_executor` | Re-run the same targets plus `cargo test --locked -p taskmesh --features rayon --test hardening_executor_authority` |
| BG25-006 | `cargo test --locked -p taskmesh --test hardening_deadline_custody --test deadline_cancel --test hardening_drain --test hardening_executor_protocol --test runtime_cpu_executor` | `cargo test --locked -p taskmesh --test host_open_loop terminal_caller_keeps_worker_charged_through_pre_drain_queue -- --exact`<br>`cargo test --locked -p taskmesh --lib runtime::claim_acquisition_tests::external_preemption_blocks_host_dispatch_and_drain_until_token_release_v1 -- --exact` |
| BG25-007 | `cargo test --locked -p taskmesh --test runtime_local --test hardening_dispatch_resolution --test e2e_scenarios` | `cargo test --locked -p taskmesh --test hardening_root_child_scope` |
| BG25-008 | `cargo test --locked -p taskmesh-engine --test hardening_exact_accounting --test pending_resolver --test memory_overcommit` | `cargo test --locked -p taskmesh-engine --test hardening_admission_matrix --test hardening_admission_ledger` |
| BG25-009 | `cargo test --locked -p taskmesh-engine --test hardening_fairness_reference --test hardening_effect_retirement --test pending_resolver --test differential_model` | `cargo test --locked -p taskmesh-engine --test hardening_queue_history`<br>`cargo test --locked -p taskmesh --test hardening_drain` |
| BG25-010 | `cargo test --locked -p taskmesh-engine --test hardening_memory_epochs --test hardening_snapshot_projection`<br>`cargo test --locked -p taskmesh-contract --test snapshot_oracle` | `cargo test --locked -p taskmesh-engine --test hardening_memory_ledger` |
| BG25-011 | `cargo test --locked -p taskmesh-bench --test hellgate --test inferno --test fairness_property` | `cargo test --locked -p taskmesh-bench --test host_simulator_comparison`<br>`cargo test --locked -p taskmesh --test host_open_loop`; H28 quiet-host timing/population qualification remains separate |
| BG25-012 | `python3 docs/plans/bugbash-sep-25-general/tickets/validate_plan.py` | `python3 docs/plans/bugbash-sep-25-general/tickets/validate_scenario_evidence.py`<br>`just gates-inventory`<br>`just dev`<br>clean isolated `just verify-macos-ci` |

## Final source binding

Before BG25-012 qualification, record `git rev-parse HEAD`, `git rev-parse HEAD^{tree}`, and `git status --porcelain`. After the run, require unchanged HEAD/tree/path digest and validate `target/verification/macos-gates.json`. Do not aggregate owner-local results from different commits.
