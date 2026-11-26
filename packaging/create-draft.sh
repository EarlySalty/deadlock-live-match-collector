#!/usr/bin/env bash
set -euo pipefail
archive=${1:?ZIP required}
: "${RELEASE_TAG:?Release tag required}"
: "${GITHUB_SHA:?Release SHA required}"
: "${GITHUB_REPOSITORY:?Repository required}"
bash packaging/verify-package.sh "$archive"
gh attestation verify "$archive" --repo "$GITHUB_REPOSITORY" \
  --signer-workflow "$GITHUB_REPOSITORY/.github/workflows/release.yml" \
  --source-ref refs/heads/main --source-digest "$GITHUB_SHA" --signer-digest "$GITHUB_SHA" --deny-self-hosted-runners
gh attestation verify dist/live-match-collector.exe --repo "$GITHUB_REPOSITORY" \
  --signer-workflow "$GITHUB_REPOSITORY/.github/workflows/release.yml" \
  --source-ref refs/heads/main --source-digest "$GITHUB_SHA" --signer-digest "$GITHUB_SHA" --deny-self-hosted-runners
python3 packaging/release_checks.py tag
python3 packaging/release_checks.py ci
python3 packaging/release_checks.py preflight
gh release create "$RELEASE_TAG" --draft --verify-tag --title "$RELEASE_TAG" \
  --notes-file packaging/release-notes.md "$archive" dist/SHA256SUMS \
  dist/live-match-collector.exe dist/SHA256SUMS.exe \
  dist/clamav-report.txt dist/build-environment.txt
