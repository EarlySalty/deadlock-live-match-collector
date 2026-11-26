#!/usr/bin/env bash
set -euo pipefail

archive=$(realpath -e -- "${1:?Usage: scan-win64.sh <path of the packaged ZIP>}")
report="$(dirname "$archive")/clamav-report.txt"
rm -f "$report"
work=$(mktemp -d)
volume=""
cleanup() {
  if [[ -n "$volume" ]]; then
    docker volume rm "$volume" >/dev/null
  fi
  rm -rf "$work"
}
trap cleanup EXIT
exe="$work/live-match-collector.exe"
unzip -p "$archive" live-match-collector.exe > "$exe"
test -s "$exe"
chmod 644 "$exe"
chmod 755 "$work"
image=${CLAMAV_IMAGE:-clamav/clamav:stable}
if [[ "$image" != clamav/clamav:stable && ! "$image" =~ ^clamav/clamav@sha256:[0-9a-f]{64}$ ]]; then
  echo "Unexpected ClamAV image reference" >&2
  exit 1
fi
docker pull -q "$image"
scanner=$(docker image inspect --format '{{range .RepoDigests}}{{println .}}{{end}}' "$image" | grep -Ex 'clamav/clamav@sha256:[0-9a-f]{64}' | head -n 1)
[[ "$scanner" =~ ^clamav/clamav@sha256:[0-9a-f]{64}$ ]]
if [[ "$image" != clamav/clamav:stable ]]; then
  test "$scanner" = "$image"
fi
volume=$(docker volume create --label purpose=collector-clamav-scan --label "source=$(git rev-parse HEAD)")
db="type=volume,src=$volume,dst=/var/lib/clamav"
{
  printf 'scanner=%s\n' "$scanner"
  docker run --rm --mount "$db" --entrypoint freshclam "$scanner" --stdout
  docker run --rm --network none --mount "$db,readonly" --entrypoint clamscan "$scanner" --version
  docker run --rm --network none --mount "$db,readonly" --entrypoint sh "$scanner" -c 'cd /var/lib/clamav && sha256sum *.c[lv]d'
  printf 'exe_sha256=%s\n' "$(sha256sum "$exe" | cut -d ' ' -f 1)"
  docker run --rm --network none --entrypoint clamscan \
    --mount "$db,readonly" \
    --mount "type=bind,src=$exe,dst=/scan/live-match-collector.exe,readonly" \
    "$scanner" --fail-if-cvd-older-than=7 /scan/live-match-collector.exe
} | tee "$work/report"
mv "$work/report" "$report"
