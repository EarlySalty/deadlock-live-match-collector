#!/usr/bin/env bash
set -euo pipefail

archive=$(realpath -e -- "${1:?Usage: scan-win64.sh <path of the packaged ZIP>}")
report="$(dirname "$archive")/clamav-report.txt"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
exe="$work/live-match-collector.exe"
unzip -p "$archive" live-match-collector.exe > "$exe"
test -s "$exe"
chmod 644 "$exe"
docker pull -q clamav/clamav:stable
scanner=$(docker image inspect --format '{{index .RepoDigests 0}}' clamav/clamav:stable)
{
  echo "$scanner"
  docker run --rm --network none --entrypoint clamscan "$scanner" --version
  (cd "$work" && sha256sum live-match-collector.exe)
  docker run --rm --network none --entrypoint clamscan \
    --mount "type=bind,src=$exe,dst=/scan/live-match-collector.exe,readonly" \
    "$scanner" /scan/live-match-collector.exe
} | tee "$report"
