#!/usr/bin/env bash
# CircleCI checks out a PR head. Qualify GitHub's current merge ref instead.
set -euo pipefail

[[ "${CIRCLE_SHA1:-}" =~ ^[0-9a-f]{40}$ ]] || { echo "missing CircleCI head SHA" >&2; exit 2; }
test "$(git rev-parse HEAD)" = "$CIRCLE_SHA1"

case "${TASKMESH_CI_EVENT:-}" in
  push)
    test "${CIRCLE_BRANCH:-}" = main || { echo "only main pushes are admitted" >&2; exit 2; }
    expected_sha="$CIRCLE_SHA1"
    ;;
  pull_request)
    [[ "${CIRCLE_PULL_REQUEST:-}" =~ ^https://github\.com/josongmin/quanta-taskmesh/pull/([1-9][0-9]*)/?$ ]] || {
      echo "missing or foreign pull request identity" >&2; exit 2;
    }
    pr_number="${BASH_REMATCH[1]}"
    git fetch --no-tags origin "+refs/pull/${pr_number}/merge:refs/remotes/origin/taskmesh-pr-${pr_number}-merge"
    expected_sha="$(git rev-parse "refs/remotes/origin/taskmesh-pr-${pr_number}-merge^{commit}")"
    test "$(git rev-parse "${expected_sha}^2")" = "$CIRCLE_SHA1" || {
      echo "PR merge ref does not contain the triggered head" >&2; exit 2;
    }
    git checkout --detach "$expected_sha"
    ;;
  *) echo "unsupported CircleCI event: ${TASKMESH_CI_EVENT:-unset}" >&2; exit 2 ;;
esac

test -z "$(git status --porcelain --untracked-files=normal)" || {
  echo "verification source is dirty" >&2; exit 2;
}
mkdir -p target/ci
printf 'export TASKMESH_EXPECTED_SHA=%s\n' "$expected_sha" > target/ci/source.env
printf 'verification source: %s (event=%s, trigger=%s)\n' \
  "$expected_sha" "$TASKMESH_CI_EVENT" "$CIRCLE_SHA1"
