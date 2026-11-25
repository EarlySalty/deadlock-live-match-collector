#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
target=x86_64-pc-windows-gnu
cargo build --locked --release --target "$target"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cargo metadata --format-version 1 --locked --filter-platform "$target" > "$stage/metadata.json"
version=$(jq -r '.packages[] | select(.name == "live-match-collector").version' "$stage/metadata.json")
out="$(pwd)/dist"
archive="live-match-collector-${version}-win64.zip"
tree="$stage/package"
mkdir -p "$tree/Lizenzen" "$out"
target_dir=$(jq -r '.target_directory' "$stage/metadata.json")
exe="$target_dir/$target/release/live-match-collector.exe"
imports=$(x86_64-w64-mingw32-objdump -p "$exe" | awk '/DLL Name:/ { print $3 }' | sort -u)
echo "$imports" >&2
if grep -Eiq '^lib(gcc|winpthread|stdc)' <<< "$imports"; then
  echo "The executable depends on a MinGW runtime DLL that Windows does not ship" >&2
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
    echo "Missing third-party notices for $package" >&2
    exit 1
  fi
done < <(jq -r '.packages[] | select(.source != null) | [(.name + "-" + .version), .manifest_path] | @tsv' "$stage/metadata.json")
cp "$(rustc --print sysroot)/share/doc/rust/COPYRIGHT-library.html" "$tree/Lizenzen/Rust.html"
cp /usr/share/doc/mingw-w64-common/copyright "$tree/Lizenzen/MinGW-w64.txt"
(cd "$tree" && zip -q -X -r "$stage/$archive" .)
mv "$stage/$archive" "$out/$archive"
sha256sum "$out/$archive"
