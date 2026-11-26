#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p dist
export TZ=UTC LC_ALL=C
{
  printf 'source=%s\nrunner_image=%s\nrunner_image_version=%s\n' "$(git rev-parse HEAD)" "${ImageOS:-local}" "${ImageVersion:-local}"
  rustc -Vv
  cargo -V
  x86_64-w64-mingw32-gcc --version
  x86_64-w64-mingw32-ld --version
  dpkg-query -W 'gcc-mingw-w64*' 'binutils-mingw-w64*' 'mingw-w64*'
  zip -v
} > dist/build-environment.txt
sha256sum Cargo.lock > dist/Cargo.lock.sha256
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
export CARGO_INCREMENTAL=0 RUSTC_WRAPPER="" RUSTC_WORKSPACE_WRAPPER=""
target=x86_64-pc-windows-gnu
# Both target directories must be absent before their respective build, and
# each packaged EXE must match its independently generated target output.
test ! -e "$work/first"
test ! -e "$work/first-build"
archive=$(CARGO_TARGET_DIR="$work/first" CARGO_BUILD_BUILD_DIR="$work/first-build" bash packaging/package-win64.sh)
test -d "$work/first-build"
test -s "$work/first/$target/release/live-match-collector.exe"
cmp "$work/first/$target/release/live-match-collector.exe" dist/live-match-collector.exe
cp "$archive" "$work/first.zip"
cp dist/live-match-collector.exe "$work/first.exe"
test ! -e "$work/second"
test ! -e "$work/second-build"
archive=$(CARGO_TARGET_DIR="$work/second" CARGO_BUILD_BUILD_DIR="$work/second-build" bash packaging/package-win64.sh)
test -d "$work/second-build"
test -s "$work/second/$target/release/live-match-collector.exe"
cmp "$work/second/$target/release/live-match-collector.exe" dist/live-match-collector.exe
# Expose the second package even if comparison fails so the independent local scan
# can still run; the build step itself remains failed.
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  printf 'archive=%s\n' "$archive" >> "$GITHUB_OUTPUT"
fi
status=0
{
  printf 'source=%s\n' "$(git rev-parse HEAD)"
  printf 'first_exe=%s\n' "$(sha256sum "$work/first.exe" | cut -d ' ' -f1)"
  printf 'second_exe=%s\n' "$(sha256sum dist/live-match-collector.exe | cut -d ' ' -f1)"
  printf 'first_zip=%s\n' "$(sha256sum "$work/first.zip" | cut -d ' ' -f1)"
  printf 'second_zip=%s\n' "$(sha256sum "$archive" | cut -d ' ' -f1)"
  cmp -s "$work/first.exe" dist/live-match-collector.exe || status=1
  cmp -s "$work/first.zip" "$archive" || status=1
  printf 'target_dirs=two-fresh-distinct\nbuild_dirs=two-fresh-distinct\ncompiler_wrappers=disabled\ncomparison_exit=%s\n' "$status"
} > dist/reproducibility.txt
python3 -B packaging/check_pe.py dist/live-match-collector.exe > dist/pe-evidence.txt || status=1
exit "$status"
