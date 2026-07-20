#!/usr/bin/env bash
# Rewrite packaging/homebrew/Formula/smelt.rb for a released version: fetches
# the standalone tarballs from the matching GitHub release and replaces the
# version + sha256 fields. Never hand-edit the formula's sha256 values.
#
# Usage: scripts/update-homebrew-formula.sh X.Y.Z
set -euo pipefail

if [ $# -ne 1 ]; then
  echo "usage: $0 X.Y.Z" >&2
  exit 1
fi

version="$1"
repo="adbrowne/smelt-sql"
formula="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/packaging/homebrew/Formula/smelt.rb"

declare -A targets=(
  [macos_aarch64]="smelt-macos-aarch64.tar.gz"
  [linux_x86_64]="smelt-linux-x86_64.tar.gz"
  [linux_aarch64]="smelt-linux-aarch64.tar.gz"
)

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

declare -A sha256s
for key in "${!targets[@]}"; do
  asset="${targets[$key]}"
  url="https://github.com/${repo}/releases/download/v${version}/${asset}"
  echo "Fetching ${url}"
  curl -sL --fail -o "${tmpdir}/${asset}" "$url"
  sha256s[$key]="$(sha256sum "${tmpdir}/${asset}" | cut -d' ' -f1)"
done

sed -i.bak \
  -e "s/version \"[0-9.]*\"/version \"${version}\"/" \
  "$formula"

# Replace each target's sha256 in file order: macos-aarch64, linux-x86_64,
# linux-aarch64 (must match the on_macos/on_linux block order in the formula).
python3 - "$formula" "${sha256s[macos_aarch64]}" "${sha256s[linux_x86_64]}" "${sha256s[linux_aarch64]}" <<'PYEOF'
import re
import sys

path, macos_sha, linux_x86_64_sha, linux_aarch64_sha = sys.argv[1:5]
with open(path) as f:
    text = f.read()

shas = iter([macos_sha, linux_x86_64_sha, linux_aarch64_sha])
text = re.sub(r'sha256 "[0-9a-f]+"', lambda _: f'sha256 "{next(shas)}"', text)

with open(path, "w") as f:
    f.write(text)
PYEOF

rm -f "${formula}.bak"
echo "Updated ${formula} to version ${version}"
