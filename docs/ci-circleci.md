# CircleCI operations

The automatic check is the complete 16-gate `ci` profile from
`tools/gates/required.json`. `.circleci/config.yml` invokes the same
`tools/gates/run.py` and validates its clean-source receipt. A PR run first
fetches `refs/pull/<number>/merge`, requires its second parent to be the
triggered head SHA, then verifies that merge commit. Main runs verify the push
SHA. A base update needs a fresh PR result.

## External settings

- Personal CircleCI account: `944899@gmail.com`; GitHub source owner:
  `josongmin`. The project link in `.circleci/info.yml` is generated from
  CircleCI CLI lookup, not copied from another repository.
- One GitHub App trigger: `only-build-prs`, which covers PR open/update/reopen
  and default-branch pushes. Tag jobs are filtered out in the config. No
  schedule or broad all-branch push trigger.
- Project setting `auto-cancel-builds=true` cancels superseded non-main branch
  workflows. CircleCI does not auto-cancel default-branch workflows.
- On successful exact-merge-source execution, GitHub `main` branch protection
  must require the observed `ci/circleci: required` status strictly. Disable the former
  GitHub Actions PR workflow in GitHub settings after the new check is required.
  Do not remove the old required context before observing the CircleCI result.

Check live settings with `circleci project get`, `circleci pipeline list`,
`circleci project trigger list`, `circleci project setting get auto-cancel-builds`,
`circleci run list`, and GitHub branch-protection API. These settings are not
proved by the repository YAML alone.

## Execution and cache policy

The regular workflow runs on a pinned Ubuntu 24.04 machine `medium` executor
(two vCPUs) with two Cargo build/test workers. Rust 1.99.0 and the separate
1.81.0 consumer MSRV, nextest 0.9.104, cargo-deny 0.19.7, just 1.58.0,
uv 0.9.11, Python 3.12.12, and Semgrep's tracked version are checked or
installed exactly. `UV_LOCKED=1`, `uv sync --locked`, Cargo `--locked`, and
unchanged lockfiles are mandatory. Every shell step starts with
`set -euo pipefail`.

Only dependency downloads are cached: Cargo registry/git and uv downloads.
Keys identify Taskmesh, Linux/amd64, tool versions, dependency kind, and the
relevant lockfile. Restore fallback is followed by a real locked fetch/sync,
then a nonempty cache check before saving a new exact key. Cargo `target/`
is never cached, so Clippy, tests, and benchmarks cannot overwrite each
other's immutable build cache.

`run_fuzz`, `run_load`, and `run_benchmark` are separate pipeline booleans,
all default `false`. Their workflows produce diagnostics when explicitly
selected; none is a release, performance-admission, or nightly qualification
receipt. Mutation campaigns remain under the separately authorized release
qualification procedure.
