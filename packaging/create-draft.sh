#!/usr/bin/env bash
set -euo pipefail
archive=${1:?ZIP required}
: "${RELEASE_TAG:?Release tag required}"
: "${GITHUB_SHA:?Release SHA required}"
: "${GITHUB_REPOSITORY:?Repository required}"
: "${RELEASE_MODE:?Release mode required}"
bash packaging/verify-package.sh "$archive"
python3 packaging/virustotal_report.py dist --verify
python3 packaging/dependency_evidence.py dist --verify
gh attestation verify "$archive" --repo "$GITHUB_REPOSITORY" \
  --signer-workflow "$GITHUB_REPOSITORY/.github/workflows/release.yml" \
  --source-ref refs/heads/main --source-digest "$GITHUB_SHA" --signer-digest "$GITHUB_SHA" --deny-self-hosted-runners
gh attestation verify dist/live-match-collector.exe --repo "$GITHUB_REPOSITORY" \
  --signer-workflow "$GITHUB_REPOSITORY/.github/workflows/release.yml" \
  --source-ref refs/heads/main --source-digest "$GITHUB_SHA" --signer-digest "$GITHUB_SHA" --deny-self-hosted-runners
gh attestation verify dist/cargo-sbom.cdx.json --repo "$GITHUB_REPOSITORY" \
  --signer-workflow "$GITHUB_REPOSITORY/.github/workflows/release.yml" \
  --source-ref refs/heads/main --source-digest "$GITHUB_SHA" --signer-digest "$GITHUB_SHA" --deny-self-hosted-runners
python3 packaging/release_checks.py tag
python3 packaging/release_checks.py ci
release_flags=()
notes=$(mktemp)
trap 'rm -f "$notes"' EXIT
cp packaging/release-notes.md "$notes"
printf '\n' >> "$notes"
cat dist/virustotal-report.md >> "$notes"
report_assets=(dist/virustotal-report.md dist/virustotal-report.json dist/cargo-sbom.cdx.json dist/dependency-gate.json dist/Cargo.lock.sha256)
case "${RELEASE_MODE:-stable}" in
  stable) python3 packaging/release_checks.py preflight ;;
  prerelease)
    release_flags=(--prerelease --latest=false)
    printf '\n%s\n' '**Pre-release: the production API may not be deployed yet. Do not use for real matches until the backend and permitted game access are confirmed.**' >> "$notes"
    ;;
  *) echo 'Invalid release mode' >&2; exit 2 ;;
esac
gh release create "$RELEASE_TAG" --draft --verify-tag "${release_flags[@]}" --title "$RELEASE_TAG" \
  --notes-file "$notes" "$archive" dist/SHA256SUMS \
  dist/live-match-collector.exe dist/SHA256SUMS.exe \
  dist/clamav-report.txt dist/build-environment.txt "${report_assets[@]}"
# Verify the uploaded draft, including all remote asset digests. The protected
# publisher repeats this check immediately before publishing by this release ID.
python3 packaging/verify_draft.py "$archive"
