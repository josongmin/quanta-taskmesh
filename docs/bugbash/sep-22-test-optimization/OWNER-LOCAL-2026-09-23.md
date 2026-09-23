# Owner-local verification — 2026-09-23

This is focused execution evidence on `main@1d2bb2fedcbe45ed5809d5f87e4f66560f5c7d9b`
plus a dirty shared worktree. At the end of the runs, the T01–T07 owner source files
matched HEAD. `hellgate.rs` (T08) had an uncommitted 10,000-to-1,000 cycle change.
Other dirty paths included the focused-mutation tool and planning documents. These runs
are not an exact-source qualification receipt or before/after performance measurement.

| Owner | Command | Result |
| --- | --- | --- |
| T01/T02 | `cargo test --locked -p taskmesh --test runtime_cancel_timeout --test cancellation_policy --test deadline_cancel` | exit 0; 10/10, 3/3, 15/15; no ignored or filtered tests |
| T03/T04 | `cargo test --locked -p taskmesh --test e2e_scenarios --test e2e_chaos` | exit 0; 9/9 and 3/3; no ignored or filtered tests |
| T05 | `uv run --isolated --python 3.9 pytest tools/verification/tests/test_run_generated_mutants.py -q` | exit 0; 28/28 |
| T06 | `uv run pytest tools/semgrep/tests/test_rules_fire.py -q`; `just semgrep` | exit 0; 47/47; pinned Semgrep 1.157.0 scanned 133 files with zero findings |
| T07 | `cargo test --locked -p taskmesh-rayon --test rayon_smoke` | exit 0; 4/4; no ignored or filtered tests |
| T08 | `cargo test --locked -p taskmesh-bench --test hellgate` | exit 0; 6/6 on the dirty 1,000-cycle source; no ignored or filtered tests |

The focused mutation diagnostic's producer-exit rule was also checked with
`uv run pytest tools/verification/tests/test_run_focused_mutants.py -q` (6/6),
the same focused test under isolated Python 3.9 (6/6),
`uv run ruff check` (exit 0), and `uv run ruff format --check` (exit 0). A complete
raw mutant denominator now cannot yield `DIAGNOSTIC_PASS` when the producer exits
nonzero; cached reports recheck process completion.
The cache input includes the execution supervisor and source-digest owner along with
the selected package's transitive local dependencies and tests.
The per-mutant `--timeout` and whole-run `--campaign-timeout` are separate cache inputs
and execution bounds.
The shared campaign owner rejects inherited `CARGO_MUTANTS_*` settings that would
change execution outside the recorded command identity. The generated and focused
runner owner tests passed together (35/35 on the default interpreter and Python 3.9).
The curated runner tests that share this environment owner passed 50/50.

The gate and final receipt owners were checked for interrupted execution. A child
that handles termination and exits zero is now recorded as `FAIL`; the local
receipt accepts that row as a valid failed execution, while both local and final
qualification validators reject a `PASS` row carrying timeout, signal, or
interruption evidence. Three focused regressions failed before the validator
repair and passed afterward. The gate inventory and qualification receipt suites
passed together (247/247); Ruff and `git diff --check` passed.

A second gate-runner fault appeared at the parallel batch boundary: with five
approved static gates and a four-process batch limit, a failed first batch
omitted the fifth gate from `results`; an interruption or spent global deadline
could also launch it under `--keep-going`. Three parameterized cases were red
before repair. The runner now records the unstarted wave tail as `NOT_RUN` with
`blocked_by`, and stops the wave for fail-fast, signal, or deadline. A real
`just` batch fixture confirms the receipt retains all five ids. Gate inventory,
batch supervisor, and qualification receipt tests passed together (251/251);
the isolated Python 3.9 run passed 252/252. Ruff passed.

T07's `executor_runs_cpu_work` also has isolated one-edit negatives: dispatch
removal with a live sender fails at the one-second local timeout, sender drop
fails as `Disconnected`, and a wrong result fails the 42-value assertion. Each
executed 1/1 test and exited 101. The restored positive passed 1/1; the exact
binary then passed 20/20 direct runs (worst process wall time 0.0057 s on this
macOS arm64 host). The owner file stayed at SHA-256
`4a6c3fa70d554c76e01824551a51ed804a70bdb36a126c1aa41ab5a13d9a8c20`.

T06's synthetic rule suite passed 47/47 and the one canonical real-source gate
passed on Semgrep 1.157.0 (133 files, zero findings). With `CI=1` and Semgrep
absent from `PATH`, rule collection failed with exit 2 and the canonical gate
failed with exit 1. This verifies the missing-binary policy; the observed 4.71 s
and 9.78 s process durations are not before/after savings evidence.

T03 and T04 were compared against single-function or single-sleep ablations on
the same production source and `test` profile. Each variant ran its exact binary
10/10 times in alternating order. T03 current versus prior retry-loop median/worst
was 0.0076/0.2398 s versus 0.0070/0.2361 s; no successful-path cost saving
was observed. T04 overload current versus restored 60 ms sleep was
0.0522/0.2867 s versus 0.1221/0.3509 s. T04 chaos current versus restored
50 ms sleep was 0.0317/0.2546 s versus 0.0920/0.3107 s. The two T04 median
differences match the removed waits; contended-host worst outliers do not
qualify a general wall-time claim. Both owner files were restored to their
original SHA-256 values after compilation.

T08's earlier 1,000-cycle and same-source 50,000-cycle exact binaries ran in
alternating order 10/10 each on macOS arm64. Median/worst process wall times
were 0.0891/0.7376 s versus 3.9653/4.4518 s, a 97.75% local median reduction.
The candidate passed ten more direct runs for 20/20 total. This is focused
Mac cost evidence; Linux and full-gate performance remain unqualified.

At that checkpoint, the [macOS candidate ladder](T08-MACOS-LADDER-2026-09-23.json)
had six variants passing 10/10 each, and 2,000 cycles was the one-step local
margin above the smallest passing 1,000-cycle variant. Linux comparison and
clean-source qualification were still open. The later cross-OS measurements
below close the owner-local Linux comparison; the selected source remains
`IMPLEMENTED_UNQUALIFIED` until the complete gate set and receipt run on a
clean frozen candidate.

A later Rust 1.92.0 owner-local ladder used separate detached checkouts and
the same six `hellgate.rs` source variants on macOS arm64 and Docker Linux
arm64. The final 2,000-cycle source SHA-256 is
`aeaaa0abea0f555355d7d21fec5e0d4f236864421fc6c49df1d31c63f0055ce8`.
Each variant passed 10/10 exact runs, and the one-step 2,000-cycle margin
passed 20/20 in each environment. Raw process times and the three measured
throughputs from every run are in [macOS](T08-MACOS-RUST192-LADDER-2026-09-23.json)
and [Docker Linux](T08-LINUX-CONTAINER-LADDER-2026-09-23.json). The 50k to 2k
focused median reduction was 96.15% on macOS and 95.82% in Docker Linux.
`cargo test --offline --locked -p taskmesh-bench` passed 72/72 on each selected
source. Linux ran in a VM on the Mac hardware, and host work was not monitored
continuously; this is OS-class owner-local evidence, not quiet-host performance
qualification. The selected 2k source is still dirty and the full clean-source
gate/matrix/qualification remains open.

The current focused mutation and generated mutation owner suites passed 87/87 together;
`tools/pm/tests` passed 4/4 and `just prompt-check` passed. The installed
`cargo-mutants` CLI accepts the runner's `--in-diff` and `--cargo-arg` options.
The H16 document validator passed. The SEP22 validator passed with
`--structure-only` (8 tickets, 8 findings, 32 acceptance IDs, 28 citations,
19 links). Its default source-freeze mode fails because `plan.json` deliberately
records the old `7023e945` baseline while the current HEAD is `1d2bb2f`;
structure validation is not a current-source qualification.

A saved local receipt could previously treat a conditional tool prerequisite
such as `consumer-msrv`'s installed toolchain as a platform exclusion. The
platform-scope evaluator now accepts `SKIPPED_PLATFORM` only when the gate
inventory actually excludes the host and the declared conditional matches that
platform. The direct negative was red before the repair; the gate inventory,
batch supervisor, and final receipt owner suites then passed 253/253. This
hardens saved-receipt validation; it is not a substitute for running the gate.
The same 253 tests passed under isolated Python 3.9; Ruff and both plan document
validators passed in their applicable modes.
`just gates-inventory` also passed: 24 inventoried gates, 23 required, and parity
with 24 workflow invocations.

The local gate receipt validator also now re-derives `qualified`,
`required_not_run`, and `required_not_passed` from the saved result rows and
source-stability fields. Previously, altering any of those three summaries in
an otherwise valid macOS receipt still produced `VALID`. The owner-local
negative was red before repair and green afterward. This checks internal
consistency of a saved receipt; it does not authenticate that a hand-written
PASS row came from an executed gate.

The focused generated-mutation cache previously accepted a report with no
`source` object if its raw mutant outputs and process fields matched. A
preflight reuse could therefore return a diagnostic with no source identity;
the snapshot fallback also indexed the missing field. A red owner-local test
covered missing and inconsistent snapshot identities. Reuse now requires the
report schema/kind, parse success, a Git HEAD, and matching before/snapshot
SHA-256 digests. The generated, focused, and curated mutation runner tests
passed together 87/87 on the default interpreter and isolated Python 3.9.

The workspace `taskmesh-doc-examples` build script reads root `README.md`,
`docs/taskmesh-external-interface.md`, and `CHANGELOG.md` outside its package
directory. The focused cache dependency closure had omitted them, so a document
change could reuse a diagnostic produced against different generated Rust
examples. The missing-input test was red before repair. The runner now derives
those paths from the build script's `SOURCES` declaration and refuses an
unrecognized declaration; a README edit changes the cache key in the owner
test. During the combined run, a separate process-group test exposed a marker
creation race: the observer could read the PID file after creation but before
its contents were written. The child now publishes the completed PID file by
same-directory atomic rename, as does the parallel gate supervisor test.
The timeout escalation test also now waits for an explicit child readiness
marker before starting its 0.1-second supervisor deadline, so a slow process
start cannot masquerade as a cleanup failure. The mutation runner plus batch
supervisor owner suites passed 90/90 on isolated Python 3.9 and 3.11; Ruff,
format, and `git diff --check` passed. The system Python 3.11 pytest could not
collect because its installed `anyio` plugin imports missing `_pytest.scope`;
the isolated Python 3.11 run is the valid result.

The shared process supervisor had another reachable false-green path: a child
could close its captured stdout/stderr, stay alive in the same process group,
and outlive a leader that exited zero. A direct reproduction returned zero
while that child was still running. New single-process and parallel-batch
negatives failed 2/2 before repair. Both supervisors now kill a remaining
owned group after the leader is reaped, append a diagnostic, and report no
successful group exit status. A PASS status line from the leader cannot make
the gate pass; curated/generated/focused mutation producers also reject the
missing exit status. The owner gate, qualification, and verification suites
passed 394/394 on isolated Python 3.9 and 3.11; Ruff and format checks passed.
This is process-custody proof, not a full clean-source gate receipt.

The timeout boundary had a separate escape case: a grandchild could create a
new session while retaining the leader's stdout/stderr pipes. The original
group no longer owned it. On macOS, the single supervisor raised
`PermissionError` when signaling the vanished/inaccessible group; the batch
supervisor remained blocked past an outer four-second watchdog. Both red
cases used a bounded wrapper and explicitly killed the escaped test child.
The pipe readers now stop after the command timeout and termination grace,
close their own capture streams, and return a timed-out failure with a
`capture pipes remained open` diagnostic even when the escaped child keeps its session alive.
The supervisor does not claim to kill processes outside its group. A normal
leader that writes output before and after multiple poll intervals still
returns both complete streams under single and batch supervision. The gate,
qualification, and mutation owner suites then passed 398/398 under isolated
Python 3.9 and 3.11. Full clean-source qualification remains pending.

The supervisor's exceptional cleanup had a further unbounded wait: if a
capture read raised, it retried `communicate()` after signaling the original
group. A descendant in a new session could keep the pipes open forever.
A real subprocess reproduction injected a capture read error only after the
escaped child had published its PID; the original supervisor exceeded the
outer three-second limit. The exception path now closes its capture streams,
waits only through the termination grace, and performs bounded direct leader
cleanup before re-raising the read error. The same reproduction passes under
Python 3.9, and the gate/qualification/verification owner suites pass 399/399
under isolated Python 3.9 and 3.11.
This closes the supervisor hang; it does not claim custody of a process that
left the owned process group. The final clean-source qualification remains open.

The parallel supervisor's exception path sent termination signals but did not
reap leaders whose capture reader failed before calling `communicate()`. A real
SIGTERM-ignoring child with an injected reader error left its retained `Popen`
with `returncode=None` after `run_process_batch` raised; the owner test was red
before repair. After all capture readers stop, the batch now closes their
streams, performs bounded leader cleanup, and kills any remaining owned group.
The same child is reaped with `-SIGKILL` before the original read error escapes.
The gate/qualification/verification owner suite passes 400/400 under isolated
Python 3.9; the changed supervisor file passes 8/8 under isolated Python 3.11.
Ruff, both plan document validators in their applicable modes, and
`git diff --check` pass. These are owner-local checks, not final qualification.

The final receipt collector had no outer bound around `tools/gates/run.py`:
the inventory's 18,000-second deadline was passed to the runner, but
`subprocess.run()` would wait forever if the runner itself stalled. It now
supervises the runner process group for the inventory budget plus 120 seconds
of startup/finalization allowance. A real child that wrote a plausible JSON
sidecar and then handled SIGTERM with exit 0 is reported as timed out, not
qualified. The saved receipt validator also requires explicit `timed_out=false`
and `interrupted_by_signal=null` gate-runner evidence; removing either field
or setting timeout true was red before the repair. A normal completed runner
still returns its sidecar and clean process status. The complete required gate
set and clean-source receipt have not been run in this dirty checkout.

The final gate/qualification/verification owner suite passed 405/405 under
isolated Python 3.9; the final receipt owner suite passed 182/182 under
isolated Python 3.11. Ruff, format, both plan validators in their applicable
modes, and `git diff --check` passed after these changes.

The shared source digest also collapsed every regular file's mode into one
executable yes/no bit. A tracked file changed from `0644` to `0600` without
changing its HEAD or digest; the owner-local regression failed before repair.
`source_tree_digest` now hashes the full permission mode with each file's path,
kind, and bytes. The source-identity regression and current mutation/receipt
users were checked together: gate/qualification/verification owner suites
passed 406/406 under isolated Python 3.9, and the receipt owner suite passed
183/183 under isolated Python 3.11. Ruff, format, both document validators
in their applicable modes, and `git diff --check` passed. No final clean-source
qualification was run.

The focused mutation preflight cache still hashed selected dependency bytes
only. Changing a selected file from `0644` to `0600` left its cache key
unchanged even after the canonical source digest gained full-mode identity;
retargeting a selected symlink to equal bytes would do the same. The owner
regression was red on the mode change. The cache now uses the shared
`source_tree_digest` over its selected dependency closure, so mode and symlink
target changes invalidate reuse while unrelated files do not.

After that repair, gate/qualification/verification/prompt-policy owner suites
passed 411/411 under isolated Python 3.9. The focused mutation and receipt
owners passed 194/194 under isolated Python 3.11. The reformatted focused
owner passed 11/11 again under Python 3.9; Ruff, format, prompt-check, plan
validators, and `git diff --check` passed. The shared checkout remains dirty,
so none of these runs is the final qualification receipt.

The shared source digest had a path-boundary collision. With files `a = X` and
`b = Y`, deleting `b` and putting its serialized path/mode/content record after
`X` in `a` produced the same SHA-256 input. This was reproduced directly and
the semantic owner test failed before the fix. `source_tree_digest` now length-
frames each path and content record and uses a new domain prefix; source
qualification and focused cache keys consequently change. The focused evidence
and mutation owner tests passed 55/55, and the gate/qualification/verification/
prompt-policy owner suite passed 412/412 under isolated Python 3.9. Ruff and
format checks passed for the changed Python owner. The receipt owner suite
passed 183/183 under isolated Python 3.11; the focused evidence/mutation
owners passed 55/55 under Python 3.11. These are owner-local checks;
the dirty checkout still has no final qualification receipt.

The gate runner previously accepted any occurrence of the required status
token on the final self-report line. A zero-exit `bench-iai` line containing
both `status=BASELINE_CREATED` and `status=QUALIFIED`, or two `status=QUALIFIED`
tokens, was recorded as `PASS`. Two gate-process negatives failed before the
fix. The runner now requires exactly one `status=` token, equal to the
inventory requirement. Both negatives and the normal baseline/final-line
cases pass; gate and qualification owner suites passed 315/315 under isolated
Python 3.9. This is receipt-status integrity, not an IAI benchmark run.

Saved receipt validation initially still trusted a `PASS` row whose retained
self-report line claimed both `BASELINE_CREATED` and `QUALIFIED`; the macOS
receipt is also the pre-push admission input. The local and final receipt
negative cases were red before repair. Their validators now share the runner's
single-status-token rule. Directly executed self-reporting gates require a
matching retained line; hosted imported producer rows are checked against their
registered envelope and raw artifacts instead. Missing, conflicting, and
multiline local status lines reject; a valid direct line and a verified
imported producer are positive controls. This extends status integrity without
claiming that validation reruns any gate. Gate and qualification owner suites
passed 320/320 under isolated Python 3.9; the inventory and receipt owners
passed 266/266 under Python 3.11. Ruff, format, both applicable plan validators,
and `git diff --check` passed. No complete clean-source gate was run.

The shared final-line helper moved from `tools/gates/run.py` to
`tools/gates/status_line.py`. The curated `an-earlier-status-line-stands-in-for-
the-verdict` mutation was re-anchored to that owner file. Its isolated baseline
passed and the one-edit mutation was `KILLED` by the exact named oracle with the
expected failure reason (`target/sep21/v02/status-line-owner-mutation/receipt.mutations.json`).
The verification owner suite passed 95/95 under isolated Python 3.9. This is
one selected curated mutation, not the complete 103-entry campaign.

The current `main@1d2bb2f` dirty checkout was rechecked for the T01/T02
runtime cancellation boundary with
`cargo test --locked -p taskmesh --test deadline_cancel --test runtime_cancel_timeout`.
The two exact binaries passed 15/15 and 10/10 (exit 0, no ignored or filtered
tests). A source inspection of the cancellation arbiter, detached worker
custody, and bounded drain tests found no additional confirmed P0–P2 defect
in this owner slice. This focused result does not close the full campaign.

The IAI receipt audit also reproduced a zero-exit three-case comparison in
which all summaries pointed to one raw `.out`. It incorrectly reported
`QUALIFIED` before repair. The finalizer now rejects duplicate raw output
paths across cases, as well as the previously repaired missing paths. The
IAI owner suite passed 35/35. A historical runner artifact's three summary
identities and three distinct raw paths were checked against the parser after
rebasing archive paths in memory; this is schema compatibility evidence only,
not a current Linux run.

The gate supervisor audit reproduced a zero-exit child emitting invalid UTF-8:
the serial and parallel gate paths raised `UnicodeDecodeError` before writing
their result rows. Both real-process negative cases failed before repair. The
gate runner now records `FAIL` with a decode diagnostic; missing executables
retain their existing diagnostic. Gate supervisor, inventory, and local receipt
owner suites passed 276/276; Ruff, format, and `git diff --check` passed. This
is owner-local receipt-failure coverage, not a full clean-source gate run.

The qualification collector's gate-runner sidecar reader also raised
`UnicodeDecodeError` for a zero-exit child that wrote invalid UTF-8. The
real-process negative was red before repair; the collector now records the
sidecar as invalid evidence. The complete receipt owner suite passed 186/186;
Ruff, format, and `git diff --check` passed. This is fail-closed parsing
coverage, not a source-qualified gate execution.

The modelcheck process owner was tested with a real blocked child in its own
session. Before repair, SIGTERM to the modelcheck parent left that child alive.
`tools/modelcheck/run.py` now uses the shared bounded process supervisor and
exits nonzero on interruption. The real-process negative passed after repair;
modelcheck, gate-supervisor, and curated mutation owner suites passed 68/68.
This is cancellation custody evidence, not a Loom/Shuttle full proof run.

Generated mutation saved-evidence validation accepted a `PASS` summary with
`interrupted_by_signal=15` on either the campaign or discovery process, and
accepted an excluded-count/list mismatch. Three negative cases were red before
repair. The validator now requires both processes to finish cleanly, and
re-derives exclusions from the hashed unfiltered listing, planned identities,
and source `exclude_re` policy. A raw listing altered together with its summary
and envelope digests is still rejected when it omits an unapproved mutant.
Qualification owner suites passed 233/233 and generated producer tests 30/30;
Ruff, format, and `git diff --check` passed. No full generated mutation sweep
or clean-source qualification was run.

The structured raw validator also raised `UnicodeDecodeError` when a hashed
`mutants.json` contained invalid UTF-8. A negative case reproduced the crash;
malformed raw JSON now becomes a validation problem instead of an exception.
The qualification, generated producer, and release owner suites passed
317/317 together after the change; Ruff, format, and `git diff --check` passed.

A subsequent combined Python owner run across gates, qualification, release,
verification, modelcheck, and bench passed 552/552. `ruff check tools` passed;
`ruff format --check` passed for all 19 changed tracked Python files. A
repository-wide Ruff format check still reports an unchanged formatting-only
line in `tools/gates/validate_inventory.py`; `py-lint` uses `ruff check`, not
`ruff format`, so this did not affect the owner run or claim a full gate pass.
The remaining architecture, consumer-MSRV, fuzz, and prompt-policy owner tests
passed 41/41; Semgrep rule fixtures passed 47/47 with installed Semgrep 1.157.0.
These three runs cover all 20 Python test modules under `tools/` (640 passing
cases total), but were split by owner and are not a clean-source gate receipt.
The five Taskmesh cancellation/e2e owner test binaries passed 40/40; Rayon
smoke passed 4/4 and Hellgate passed 6/6. `cargo fmt --check` passed for those
three packages. A compile warning about `AtomicBool` appeared during the first
Taskmesh test command despite its current use in `e2e_scenarios.rs`; a scoped
`cargo clippy --locked -p taskmesh --test e2e_scenarios -- -D warnings` then
passed. These are focused Rust owner checks, not `just gate` or matrix proof.

The prompt-policy checker previously followed an inventory path or import
through a symlink. Two direct startup-path negatives were red before repair;
an import-symlink negative was added with the same owner fix.
`tools/pm/check.py` now requires the inventory and every prompt file component
to be a non-symlink repository file. A separate inventory-symlink negative was
red before repair. An import spelling with `symlink/..` also bypassed lexical
normalization before repair; traversal is now checked before collapsing `..`.
Its owner suite passed 9/9 and the live `prompt-check`
returned PASS; Ruff and formatting checks passed for `tools/pm`. This postdates
the 640-case Python run above.

The IAI artifact resolver accepted a raw `.out` path through an in-store
symlinked directory. The direct negative was red before repair; two distinct
case paths could have named the same physical output. `confined_artifact` now
rejects symlinks in every path component, and a release-level alias case is
rejected. Bench IAI and release owner suites passed 91/91 after repair; the
Linux Valgrind comparison and clean-source qualification remain open.

The focused mutation cache previously raised `UnicodeDecodeError` on a
corrupted UTF-8 report instead of treating it as a cache miss. It also entered
preflight with a symlinked repository `target` parent, which could redirect
cache reads or writes outside the checkout. Both negatives were red before
repair. Cache root, entry, and report traversal now reject symlinks; malformed
UTF-8 is a miss. Focused, generated, and curated verification owner suites
passed 95/95, with Ruff, format, and `git diff --check` passing. A cached
focused diagnostic is still not a full workspace mutation qualification.

Final-run preflight (2026-09-23): `main@1d2bb2fedcbe45ed5809d5f87e4f66560f5c7d9b`
has 50 modified or untracked status entries, so the required clean-source gate
preflight rejects this checkout before executing a gate. The local
`rust:1.92-bookworm` Docker image provides Rust 1.92.0 and Python 3 on Linux
arm64, but lacks `valgrind`, `iai-callgrind-runner`, `uv`, `just`, and
`cargo-nextest`; it cannot
run `just qualify-local` as provisioned. macOS has the applicable toolchains,
but the separate `just verify-macos-nightly` profile includes the bounded full
generated mutation sweep and requires explicit authorization on a clean frozen
candidate. A macOS nightly receipt records the Linux-only `bench-iai` platform
skip; a CI receipt does not include that gate.

The first isolated macOS full-gate attempt at candidate `8bc5535` stopped at
`semgrep`: one debug `println!` left in `hellgate.rs:156`. Required later gates
were `NOT_RUN`, so that receipt is not a partial qualification. The measurement
samples already exist in the T08 ladder JSON; the print was removed. The
canonical Semgrep gate then passed (133 files, zero findings), and the exact
`hellgate` test binary passed 6/6. A refreshed clean candidate and full receipt
are still required.

The second clean-candidate attempt at `53b4063` passed the static gates,
Clippy, and Rust workspace tests, then stopped at `py-test` (649 passed,
1 failed). The candidate had excluded `.claude/CLAUDE.md` and both `.cursor`
rules as presumed personal files, but `tools/pm/inventory.json` declares
them required prompt-policy source. This was a candidate assembly error, not
a production test regression. The complete frozen candidate must include all
three files before rerunning the owner suite and full gate.

The third clean candidate, `86d8f4d`, passed every macOS-applicable required
gate through `coverage-report`, including curated mutation, modelcheck, TSan,
fuzz, and consumer MSRV. The generated cargo-mutants gate was stopped after
4,092 seconds because two `GovernorError::retry_after_ms` variants had already
timed out under its package-wide `cargo test` runner; the saved interrupted
receipt is `NOT_QUALIFIED`. Its partial raw files record 593 caught, 86
compile-unviable, 2 timeout, and 0 missed identities, but are not a complete
denominator or qualification evidence. The existing `error_surface.rs` test
asserts the returned hint directly. A separate same-source four-variant run
with `--test-tool nextest` had a green baseline and caught all four in about
two minutes, including both timed-out variants. The generated producer now
requires nextest, records its version, and the manual workflow installs it.
The generated-runner and gate-inventory owner suites passed 112/112; inventory
parity, Ruff, formatting, and `git diff --check` passed. A new clean-candidate
full sweep and receipt are still required for closure.

2026-09-24 source reconciliation: while the earlier candidate was running,
`main` advanced through `fa2492c`, `d9a359a`, and `95c1b6d` with test-oracle
pruning. Those commits change the generated mutation kill surface, so neither
earlier candidate receipt qualifies the new HEAD. At `95c1b6d`, the removed
cross-class storm has a stronger global-budget cross-class owner in
`e2e_chaos.rs`; substrate mismatch, local non-Send execution, blocking task
error/panic, and terminal retention have surviving named owners. The affected
Rust integration binaries passed 35/35, the architecture/allocation Python
owners passed 50/50, and the live crate-boundary checker passed. The H16 and
SEP22 plan validators passed in their document scopes. The nextest repair
remains a dirty overlay on `main`; full exact-source qualification
must wait for a stable final candidate.

The generated runner now watches cargo-mutants' incremental `missed.txt` and
`timeout.txt` outputs and stops its owned process group when either becomes
nonempty. The resulting partial campaign is `FAIL`, never qualification proof.
This change has syntax, lint, and diff checks only; the early-stop path and a
full current-source campaign have not been executed.
