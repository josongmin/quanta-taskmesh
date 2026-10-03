# 0.3 technical compatibility review

This is a non-approving inventory for the release owner. It does not set
`version_decision`, create `target/release/input/adjudication.json`, attest
`all_tool_findings_reviewed`, or qualify a release.

- Reference clean report: `7e0e5de4c8647e2d85ef12baa905e1be223a42ce`,
  tree `a645248a724d45cc86a9925f1209812273d0d139`. Its report was
  generated before the `AdmissionVerdict` order repair below.
- Immutable baseline: 0.2.0 commit
  `39bee682d7daa1efaf1c10993ba6221fd0a90871`.
- Producer: pinned `cargo-semver-checks 0.50.0`, `--default-features`,
  `--release-type minor`; `target/release/semver/semver-manifest.json`
  reported all four crates with exit 100. The manifest binds the raw logs by
  SHA-256. The `taskmesh` facade report alone cannot cover its reexports.

| Crate | Raw report groups | Technical disposition |
|---|---|---|
| `taskmesh-contract` | `TopologyConfig.physical_domains`; removed derives and enum-to-struct `PlanSource`; `TerminalReason` derive/discriminant changes; shifted `AdmissionVerdict` discriminants in the reference report; `TaskScope::Child` fields; `TaskSpec::child_of` arity | Published Rust source breaks. The new child identity and provenance rules also need wire/behavior review. |
| `taskmesh-engine` | `PendingView` and `PermitLedgerView` fields; `Provenance`/`ClaimOutcome` derives; `ReleaseOutcome::HeldByLease`; removed `PolicySet::capability_limit(s)` | Published direct-embedder source breaks. Lease custody changes behavior and must be reviewed independently. |
| `taskmesh` | `BlockingPoolCpuExecutor` changed from unit struct to a struct with a worker count | Published `taskmesh::ext` source break. Builder users do not directly construct this adapter. The facade's reexported contract/engine/Rayon surface still needs manual review. |
| `taskmesh-rayon` | Removed `RayonCpuExecutor::new` and `from_topology` | Published source breaks. The typed `try_*` constructors replace these panic wrappers; their return type also changed and is outside this tool's type comparison. |

The corresponding migration text is in `CHANGELOG.md` under 0.3.0. These
groups are not a claim that every raw finding has been individually mapped to
an approved break item. In particular, the release reviewer must bind the
exact log digests and each raw locator in a reviewer-owned adjudication copy.

## Avoidable discriminant change removed

The reference report listed nine `enum_no_repr_variant_discriminant_changed`
findings because the new `AdmissionVerdict::IdentityExhausted` preceded
`ClassificationFailed`. The current source places it after the 0.2 variants.
A focused dirty-source `cargo semver-checks check-release -p taskmesh-contract`
comparison against the immutable baseline no longer reported that lint;
`error_surface` passed 12/12. This diagnostic does not replace the final
clean four-crate report or CI receipt.

## Tool blind spots to adjudicate

- `RayonCpuExecutor::try_new` and `try_from_topology` now return
  `RayonBuildError` instead of `rayon::ThreadPoolBuildError`; zero workers and
  invalid topology are typed failures. Existing `?` propagation and explicit
  error matches can break even though the methods still exist.
- `PlanSource` remains a JSON string for valid legacy values, but invalid
  identifiers now reject. `TaskScope::Child` requires an exact parent operation
  at strict ingress; `parent_awaits` is defaulted at raw serde. The wire
  migration cannot be inferred from the Rust API report.
- Synchronous `SubmitOptions::CompleteBy` still rejects blocking/CPU work.
  The additive `run_blocking_response_by`/`run_cpu_response_by` APIs impose an
  absolute caller-response boundary while a started worker retains custody.
  The Semantica caller remains on the old API and is deferred.
- `AdmissionVerdict` and `TerminalReason` shape changes, physical capability
  accounting, preflight and acquisition precedence change direct consumers'
  exhaustive matches and observable behavior. Check every applicable public
  facade and direct-embedder use against the [external contract](../taskmesh-external-interface.md)
  and [release checklist](../release-checklist.md).

The 16-gate CI result tests the implementation at this source; it is not the
human version/API/wire/behavior decision or the 23-gate release receipt.
