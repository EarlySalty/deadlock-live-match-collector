#!/usr/bin/env bash
set -euo pipefail
archive=$(realpath -e -- "${1:?ZIP required}")
cd "$(dirname "$archive")"
name=$(basename "$archive")
sha256sum "$name" | cmp - SHA256SUMS
unzip -t "$name"
digest=$(unzip -p "$name" live-match-collector.exe | sha256sum | cut -d ' ' -f 1)
sha256sum -c SHA256SUMS.exe
[[ $(sha256sum live-match-collector.exe | cut -d ' ' -f 1) == "$digest" ]]
grep -Fx "exe_sha256=$digest" clamav-report.txt
grep -Fx 'Infected files: 0' clamav-report.txt
test -s build-environment.txt
