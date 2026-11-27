#!/usr/bin/env bash
# Shared fail-closed exact-EXE gate for authorized release builds.
set -euo pipefail
cd "$(dirname "$0")/.."
test "$#" -le 1
directory="${1:-dist}"
result=0
python3 packaging/virustotal_report.py "$directory" || result=$?
if [[ -n "${GITHUB_STEP_SUMMARY:-}" && -f "$directory/virustotal-report.md" ]]; then
  cat "$directory/virustotal-report.md" >> "$GITHUB_STEP_SUMMARY"
fi
exit "$result"
