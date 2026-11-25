#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
exe="$work/live-match-collector.exe"
unzip -p dist/*.zip live-match-collector.exe > "$exe"
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
} | tee dist/clamav-report.txt
