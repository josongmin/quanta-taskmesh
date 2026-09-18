#!/usr/bin/env bash
#
# Does the benchmark history branch exist on the remote? (bench.yml `trend`)
#
# Prints exactly one `exists=true|false` line for $GITHUB_OUTPUT. Only "the ref
# does not exist" (`git ls-remote --exit-code` → 2) is a reason to skip the
# trend compare. Any other failure — network, authentication, a bad remote —
# is an infrastructure error and fails the job (TM16-019): "skip the compare"
# on an auth failure is how a regression sails through with a green check.
#
#   bash tools/bench/history_branch.sh [remote] [ref]
set -uo pipefail

remote="${1:-origin}"
ref="${2:-refs/heads/gh-pages}"
err="$(mktemp)"
trap 'rm -f "${err}"' EXIT

git ls-remote --exit-code "${remote}" "${ref}" >/dev/null 2>"${err}"
code=$?
case "${code}" in
  0) echo "exists=true" ;;
  2) echo "exists=false" ;;
  *)
    echo "::error::git ls-remote ${remote} ${ref} failed with exit ${code} (not 'missing ref'):" >&2
    cat "${err}" >&2
    exit "${code}"
    ;;
esac
