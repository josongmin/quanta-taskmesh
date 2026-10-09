# 0006. Source-bound verification authority

- Status: Accepted for current local verification policy
- Date: 2026-09-25
- Source: [Sep-25 scenario inventory](../evidence/sep25/scenarios.md),
  [BG25 implementation map](0008-audit-implementation-record.md),
  and `Justfile` / `tools/gates/{inventory,required}.json`

## Context

A path-to-test mapping, a selected test invocation, and a complete gate receipt
answer different questions. A PASS attached to one source revision cannot
qualify a later document or code commit. An external consumer run does not prove
which mutable path dependency was built unless that dependency is bound in its
receipt.

## Decision

Result-producing evidence paths retain the original waitable process-group
leader until owned-group observation and cleanup finish; pipe EOF or leader
exit alone does not prove completion. Exceptional paths perform bounded group
kill before reap and remain incomplete. Termination authority covers the
original process group.
Descendants that leave it through a new session or PGID require separate
ownership; otherwise only inherited capture pipes have a bounded deadline.
Two complete group observations must agree on member PID, birth identity and
PGID, and both must show the held leader exited. A member's scheduling state
may change without changing its identity. The later observation determines
live membership; any identity or membership change remains incomplete.
On Linux, a zombie thread-group leader can still own live worker threads.
Its stat state is quiescent only with a positive observed thread count of one;
zero, missing or invalid counts leave custody unproved. Thread count is mutable
liveness evidence, not process identity, and is read with state in the same stat
observation before the original group leader is reaped.
Linux procfs directory enumeration is not a group-membership snapshot. A
non-leader stat read that fails with ENOENT or ESRCH may be omitted only when
the kernel's PGID lookup identifies another group or reports that the PID no
longer exists. An unreadable owned member, an unreadable held leader, or any
other inspection error leaves custody unproved. This classification does not
replace the two complete observations or the thread-count liveness check.

1. The 104-scenario inventory is the oracle index. Its K 51 / P 34 / G 19
   baseline and the 104-row manifest describe static target, case, oracle,
   feature, platform, and gate selection. `MAPPED` means that relationship was
   checked; it does not mean the case executed or the scenario passed.
2. Owner-local and `dev` results are diagnostic. The CI profile is qualified
   only by a complete clean-source receipt with all applicable required gates
   PASS, no required NOT_RUN or FAIL, and stable HEAD, tree, and counted path
   digest. The receipt is valid for its exact source only.
3. Nightly producers, release adjudication, performance comparisons, and
   external deployment adoption have their own denominators and authorities.
   A focused run, partial campaign, simulator metric, or static ticket status
   cannot substitute for them.
4. Gate membership and commands remain in `tools/gates/required.json`,
   `tools/gates/inventory.json`, and `Justfile`; the scenario manifest is a
   selected-case provenance map, not a second gate inventory.
5. Retired receipts, measurements and superseded text remain in Git history,
   bound by original source and byte digest in the document catalog. Active
   static maps/validators remain under `docs/evidence`. A new source or tool
   revision needs a new receipt, and a
   consumer path dependency needs an exact dependency identity before it can
   support source attribution.
6. `local` and `dev` are diagnostic feedback scopes; `ci` is the 16-gate
   clean-source profile; `nightly` is the seven-gate costly proof profile; and
   `release` additionally needs release-only artifacts and human adjudication.
   Profile, trigger and proof strength are independent. `nightly` is not a cron.
7. The bounded `.circleci/config.yml` is the automatic PR/main CI authority.
   Its `required` job checks the GitHub PR merge ref (or the main push SHA),
   runs the same 16-gate `ci` profile, validates the clean-source receipt, and
   stores it as an artifact. CircleCI's project trigger and GitHub branch rule
   are external state and must be inspected during cutover; a config file or
   green branch head alone does not prove that protection is active. The former
   `.github/workflows/pr-ci.yml` is a manual fallback. Full qualification and
   mutation-bearing workflows remain manual and separate from this check.
8. Pin the automatic CI workflow's Rust toolchain to a versioned compiler with Clippy and
   rustfmt. A rolling `stable` changed its warning set between the local audit
   and hosted execution, so a source-identical check was not reproducible.
   Toolchain upgrades require their own source change and exact-merge-SHA run;
   the consumer MSRV remains a separate Rust 1.81 check.
9. The generated mutation runner binds cargo-mutants scenario jobs separately
   from Cargo compiler jobs and nextest test processes. Its approved child
   environment uses `TASKMESH_BUILD_JOBS` and `TASKMESH_TEST_JOBS` with the
   existing four-job development default and a positive 32-job maximum, and
   records the effective build/test job counts, default nextest
   profile and zero retries; ambient nextest overrides reject. The repository
   nextest configuration is a required source-bound producer config. A passing
   baseline alone does not establish the child worker count; retain the actual
   cargo-nextest argv and complete source-bound campaign receipt.

## Completed verification changes

Mutation execution requires a fresh explicit selection of its scope. Generic audit,
finish, merge or release requests do not select a campaign. The independent
`required.json` marks both mutation gates as requiring explicit selection.
Named gate IDs are explicit; bulk selectors need `--include-mutation` and reject
before gate launch without it. The collector applies the same policy before clearing
old producer evidence. Required proof membership and partial-run verdicts are unchanged.

- The Sep-22 test-optimization tickets added focused cancellation,
  backpressure, drain, Rayon and benchmark structural oracles plus Python-floor
  and Semgrep enrollment checks. Their owner-local results remain scoped to
  their recorded source; no broad release verdict is inferred.
- The Sep-24 W1–W3 work strengthened real-tree Semgrep enrollment, complete
  fairness drain assertions, discovered Rust/Python/fuzz target selection, and
  source-bound gate receipts. W4 added the bounded hosted PR/main workflow and
  the strict required branch rule described above.
- A gate's static target catalog, a focused test result, a 16-gate receipt,
  nightly producer artifacts and a release decision retain distinct
  denominators. The command authority remains the current inventory and
  `Justfile`; historical stage/ticket plans are available in Git history.

## Retired plan disposition

| Removed planning packet | Durable decision or evidence | Open authority |
|---|---|---|
| Jun-4 startup and Sep-16 hardening | ADR 0001–0003; ADR 0008; original receipts in Git/digest catalog | Remaining work and release checklist |
| Sep-22 test optimization and Sep-24 CI stages | This ADR; original measurements in Git/digest catalog | New exact-source receipt for later commits |
| S25/BG25 implementation | ADR 0004–0006 and 0008; 104-case static mapping under `docs/evidence/sep25` | D1–D9 adoption, performance and release in remaining work |
| B04/B07 benchmark packets | ADR 9000 for measurement and completed diagnostic boundaries | Measured admission and qualified series in remaining work |
| SEP-27 SDK proposals | No unimplemented proposal was promoted to an ADR | Adopted facade error exports and Rayon constructor change in the current source; further DX work needs a concrete consumer contract |

## Consequences

The [release checklist](../release-checklist.md) remains the operational
procedure. Release qualification and external deployment adoption remain
open in [remaining work](../remaining-work.md).
The [scenario manifest](../evidence/sep25/scenario-evidence.json) and validators
remain executable static evidence, even after old plans are removed.

## Document authority and audit scope

Completed ticket/audit narratives are compressed into ADR 0008–0009. One
remaining-work list owns unfinished implementation, proof, compatibility and
consumer work. Current API/wire and release/measurement procedures retain their
own contracts; navigation does not copy source/CI snapshots. Deleted inputs are
bound to their original Git commit and byte digests in
[document history](../evidence/document-history.json).

Normal reviews freeze source/dirty ownership, inspect reachable owner and caller
paths, separate a reproduced defect from a coverage gap, and run the smallest
meaningful owner check. Unknown classes/handles/ingress reject; capacity and
worker custody require exact transition evidence. Static maps, filtered tests,
historical receipts and partial campaigns retain their denominators. Required
checks not executed are NOT_RUN. Human compatibility and consumer activation
are separately owned. Mutation/final qualification runs require explicit current
request authorization under AGENTS.md; an old packet command is not authorization.
