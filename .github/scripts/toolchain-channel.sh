#!/bin/sh
# Print the Rust version rust-toolchain.toml pins, and refuse a file that does not pin exactly one MAJOR.MINOR.PATCH.
# Used by .github/actions/setup-rust (CI) and scripts/rs (locally), so both build with the same compiler.
#
#   .github/scripts/toolchain-channel.sh [path to rust-toolchain.toml]   (default: the repository's)
#
# Exit status: 0 and the version on stdout; 1 the file pins no exact version (why, on stderr); 2 no such file.
set -eu
HERE="$(cd "$(dirname "$0")/../.." && pwd)"
FILE="${1:-$HERE/rust-toolchain.toml}"
[ -f "$FILE" ] || { echo "toolchain-channel: $FILE is not a file" >&2; exit 2; }

# Only the [toolchain] table's own `channel` key counts; a `channel` elsewhere (a comment, another table) does not.
channels="$(awk '
  /^[[:space:]]*\[/ { table = $0; gsub(/[[:space:]]/, "", table); next }
  table == "[toolchain]" && /^[[:space:]]*channel[[:space:]]*=/ {
    v = $0; sub(/^[^=]*=[[:space:]]*/, "", v); sub(/[[:space:]]*(#.*)?$/, "", v); print v
  }' "$FILE")"
count="$(printf '%s' "$channels" | grep -c . || true)"
[ "$count" -eq 1 ] || { echo "toolchain-channel: $FILE has $count channel keys in [toolchain], not one" >&2; exit 1; }
case "$channels" in
  \"*\") version="${channels#\"}"; version="${version%\"}" ;;
  \'*\') version="${channels#\'}"; version="${version%\'}" ;;
  *) echo "toolchain-channel: the channel in $FILE is not a quoted string: $channels" >&2; exit 1 ;;
esac
if ! printf '%s\n' "$version" | grep -qxE '[0-9]+\.[0-9]+\.[0-9]+'; then
  echo "toolchain-channel: $FILE pins '$version'; it must be an exact MAJOR.MINOR.PATCH (not stable, beta, nightly or 1.100)" >&2
  exit 1
fi
echo "$version"
