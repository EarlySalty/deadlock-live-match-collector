#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
export TZ=UTC LC_ALL=C
umask 022
SOURCE_DATE_EPOCH=$(git show -s --format=%ct HEAD)
export SOURCE_DATE_EPOCH
exec 3>&1 1>&2
target=x86_64-pc-windows-gnu
out="$(pwd)/dist"
profile=release
suffix=""
case "${1:-}" in
  "")
    test "$#" -eq 0
    ;;
  --internal-test)
    test "$#" -eq 1
    out="$out/internal-test"
    source_sha=$(git rev-parse HEAD)
    suffix="-internal-${source_sha:0:12}"
    if [[ -n "${CARGO_SLOT:-}" ]] || command -v cargo-slot >/dev/null; then
      profile=verify
    else
      profile=debug
    fi
    ;;
  *)
    echo "Usage: package-win64.sh [--internal-test]" >&2
    exit 2
    ;;
esac
mkdir -p "$out"
rm -f "$out"/live-match-collector-*-win64.zip "$out/SHA256SUMS" "$out/clamav-report.txt"
if [[ "$profile" != release ]]; then
  rm -f "$out/live-match-collector.exe" "$out/SHA256SUMS.exe" "$out/build-environment.txt"
fi
if [[ "$profile" == release ]]; then
  cargo build --locked --release --target "$target"
elif [[ "$profile" == verify ]]; then
  CARGO_SLOT_DEPLOY=0 "${CARGO_SLOT:-cargo-slot}" build --locked --target "$target" --features internal-test
else
  cargo build --locked --target "$target" --features internal-test
fi
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cargo metadata --format-version 1 --locked --filter-platform "$target" > "$stage/metadata.json"
version=$(jq -r '.packages[] | select(.name == "live-match-collector").version' "$stage/metadata.json")
archive="live-match-collector-${version}${suffix}-win64.zip"
tree="$stage/package"
mkdir -p "$tree/Lizenzen"
target_dir=$(jq -r '.target_directory' "$stage/metadata.json")
exe="$target_dir/$target/$profile/live-match-collector.exe"
test -s "$exe"
imports=$(x86_64-w64-mingw32-objdump -p "$exe" | awk '/DLL Name:/ { print $3 }' | sort -u)
echo "$imports"
if grep -Eiq '^lib(gcc|winpthread|stdc)' <<< "$imports"; then
  echo "The executable depends on a MinGW runtime DLL that Windows does not ship"
  exit 1
fi
cp "$exe" "$tree/"
cp packaging/Anleitung.txt "$tree/"
while IFS=$'\t' read -r package manifest; do
  dest="$tree/Lizenzen/$package"
  mkdir -p "$dest"
  find -L "$(dirname "$manifest")" -maxdepth 1 \
    \( -iname 'license*' -o -iname 'licence*' -o -iname 'notice*' -o -iname 'copying*' -o -iname 'copyright*' \) \
    -exec cp -rL -- {} "$dest/" \;
  if [[ -z "$(ls -A "$dest")" ]]; then
    echo "Missing third-party notices for $package"
    exit 1
  fi
done < <(jq -r '.packages[] | select(.source != null) | [(.name + "-" + .version), .manifest_path] | @tsv' "$stage/metadata.json")
cp "$(rustc --print sysroot)/share/doc/rust/COPYRIGHT-library.html" "$tree/Lizenzen/Rust.html"
cp /usr/share/doc/mingw-w64-common/copyright "$tree/Lizenzen/MinGW-w64.txt"
find "$tree" -type d -exec chmod 755 {} +
find "$tree" -type f -exec chmod 644 {} +
chmod 755 "$tree/live-match-collector.exe"
find "$tree" -exec touch -d "@$SOURCE_DATE_EPOCH" {} +
(cd "$tree" && find . -mindepth 1 -type f -print | sort | zip -q -X "$stage/$archive" -@)
mv "$stage/$archive" "$out/$archive"
(cd "$out" && sha256sum "$archive" | tee SHA256SUMS)
if [[ "$profile" != release ]]; then
  unzip -t "$out/$archive"
  unzip -p "$out/$archive" live-match-collector.exe > "$out/live-match-collector.exe"
  (cd "$out" && sha256sum live-match-collector.exe | tee SHA256SUMS.exe && sha256sum -c SHA256SUMS && sha256sum -c SHA256SUMS.exe)
  {
    printf 'purpose=internal-function-test\nsource=%s\nprofile=%s\n' "$source_sha" "$profile"
    printf 'tracked_worktree_changes:\n'
    git status --short --untracked-files=no
    rustc -Vv
    cargo -V
    x86_64-w64-mingw32-gcc --version
    x86_64-w64-mingw32-ld --version
  } > "$out/build-environment.txt"
fi
echo "$out/$archive" >&3
