#!/usr/bin/env bash
# Read-only final gate for the separately authorized protected publishing job.
# Does not create, edit or publish a release.
set -euo pipefail
archive=${1:?ZIP required}
: "${RELEASE_TAG:?Release tag required}"
: "${GITHUB_SHA:?Release SHA required}"
: "${GITHUB_REPOSITORY:?Repository required}"
: "${RELEASE_MODE:?Release mode required}"
bash packaging/verify-package.sh "$archive"
python3 packaging/virustotal_report.py dist --verify
python3 packaging/dependency_evidence.py dist --verify
for subject in "$archive" dist/live-match-collector.exe dist/cargo-sbom.cdx.json; do
  gh attestation verify "$subject" --repo "$GITHUB_REPOSITORY" \
    --signer-workflow "$GITHUB_REPOSITORY/.github/workflows/release.yml" \
    --source-ref refs/heads/main --source-digest "$GITHUB_SHA" --signer-digest "$GITHUB_SHA" --deny-self-hosted-runners
done
case "$RELEASE_MODE" in
  stable) python3 packaging/release_checks.py preflight ;;
  prerelease) ;;
  *) exit 2 ;;
esac
python3 packaging/release_checks.py tag
python3 packaging/release_checks.py ci
# The protected publisher must check this immediately before mutation, use the
# verified release ID (not a new tag lookup), and enforce exclusive asset/tag
# mutation rights. The API check is not an atomic compare-and-publish.
python3 packaging/verify_draft.py "$archive"
