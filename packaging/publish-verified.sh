#!/usr/bin/env bash
set -euo pipefail
archive=${1:?ZIP required}
: "${GITHUB_REPOSITORY:?}"
: "${RELEASE_TAG:?}"
# The gate prints the verified draft ID as its final line. No tag lookup is
# performed after validation, and failure anywhere prevents the PATCH.
verified=$(bash packaging/verify-before-publish.sh "$archive")
id=$(tail -n 1 <<< "$verified")
[[ "$id" =~ ^[1-9][0-9]*$ ]]
test "$RELEASE_MODE" = prerelease
# Explicit narrow policy applies even if this script is invoked separately.
python3 packaging/release_policy.py
result=$(gh api --method PATCH "repos/$GITHUB_REPOSITORY/releases/$id" -F draft=false -F prerelease=true -f make_latest=false)
jq -e --argjson id "$id" --arg tag "$RELEASE_TAG" \
  '.id == $id and .tag_name == $tag and .draft == false and .prerelease == true' <<< "$result" >/dev/null
# Verify all public remote asset digests against the same local bytes.
python3 packaging/verify_draft.py "$archive" --published "$id"
