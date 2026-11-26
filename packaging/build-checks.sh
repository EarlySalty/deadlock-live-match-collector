#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 -B -m unittest discover -s packaging -p 'test_release_checks.py'
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
mkdir -p dist
{
  printf 'source=%s\n' "$(git rev-parse HEAD)"
  printf 'runner_image=%s\nrunner_image_version=%s\n' "${ImageOS:-local}" "${ImageVersion:-local}"
  rustc -Vv
  cargo -V
  x86_64-w64-mingw32-gcc --version
  x86_64-w64-mingw32-ld --version
  dpkg-query -W 'gcc-mingw-w64*' 'binutils-mingw-w64*' 'mingw-w64*'
  zip -v
} > dist/build-environment.txt
archive=$(bash packaging/package-win64.sh)
bash packaging/scan-win64.sh "$archive"
bash packaging/verify-package.sh "$archive"
printf 'archive=%s\n' "$archive" >> "${GITHUB_OUTPUT:?GITHUB_OUTPUT must be set}"
