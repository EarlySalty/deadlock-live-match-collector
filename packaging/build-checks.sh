#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p dist
python3 -B -m unittest discover -s packaging -p 'test_*.py'
python3 -B packaging/dependency_evidence.py dist
python3 -B packaging/dependency_evidence.py dist --verify
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
bash packaging/repro-win64.sh
archive=$(realpath -e dist/live-match-collector-*-win64.zip)
bash packaging/scan-win64.sh "$archive"
bash packaging/verify-package.sh "$archive"
