# Acquisition execution ownership

Base: `031d7b9f5bf070e884fbd6244f201caa5ae60be5` (result-trust PR #4).

## Reachable defects and structural fixes

- Standalone sampling, controls, non-witness builds, and typed validators could
  block indefinitely. All acquisition subprocesses now use a common finite,
  positive deadline and bounded capture implementation.
- A zero-exit collector could leave live descendants, including nested helpers
  in new sessions. Private cooperative owner registries propagate through build,
  study, admission, runner, and control execution. Live owned descendants cause
  failure and cleanup. Root cleanup covers its entire private registry even if
  a parent record is malformed; nested cleanup preserves unrelated siblings.
- Successful samples could mask abnormal command execution. Resource schema v3
  binds deadline and terminal flags; receipt validation independently rejects
  timeout, interruption, aborted capture, and unsuccessful exit.
- Interrupted control series could lose planned denominator entries. Bundles
  retain indexed execution records and capture hashes; later arms are explicitly
  `not_launched`. Validation binds command, mode, deadline, output, and exit.
- Failure output could disappear. Builds and validators retain partial stdout,
  stderr, and execution metadata in fresh paths. Combined capture is capped at
  8 MiB, with a 0.5-second termination grace and bounded pipe cleanup.

## Receipt migration

Execution provenance v6, resources v3, special receipts v2, and control bundles
A/A v2, snapshot v2, recorder v3, sampler v2 are required. Structural summary v3
and build witness v3 remain current. Old acquisition receipts must be reacquired;
copying terminal flags into an old receipt is not acquisition evidence.

## Verification boundary

Regression tests execute actual hung builds/probes/validators, interrupted and
overproducing children, zero-exit collectors with surviving descendants, nested
sessions, malformed owner records, and sibling isolation. Final qualification
requires a clean committed source and all 16 CI-profile gates locally, on the PR,
and after protected merge. Owner-local tests on changing source are not closure.

The ownership contract covers cooperative helpers inheriting the private owner
environment and ordinary process groups. Deliberate environment stripping or an
unregistered launcher escaping the process group is outside that contract.
Deadlines are execution bounds, not measurement SLOs. Quiet-host comparisons,
longitudinal RSS/recovery studies, consumer H7 coverage, and nightly/release
qualification are separate evidence requirements; this repair does not claim
those measurements or run mutation campaigns.
