#!/bin/sh
# toolchain-channel.sh over every combination of the channel's value, its quoting, where the key sits and what follows it, and the
# repository's own rust-toolchain.toml. A file passes only with exactly one quoted MAJOR.MINOR.PATCH channel in [toolchain].
#
#   test-toolchain-channel.sh [path to a copy of toolchain-channel.sh]   (a path points it at a deliberately broken copy)
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
SCRIPT="${1:-$HERE/.github/scripts/toolchain-channel.sh}"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
pass=0; failn=0
ok() { pass=$((pass + 1)); }
bad() { failn=$((failn + 1)); echo "BAD  $*"; }

n=0
for value in 1.100.0 0.0.1 10.20.30 stable beta nightly nightly-2026-11-01 1.100 1.100.0-beta.1 1.100.0.1 v1.100.0 1.x.0 ''; do
  for quote in double single none; do
    for place in toolchain other comment twice; do
      for tail in none comment; do
        n=$((n + 1)); f="$T/$n.toml"
        case "$quote" in double) q="\"$value\"" ;; single) q="'$value'" ;; none) q="$value" ;; esac
        line="channel = $q"
        [ "$tail" = comment ] && line="$line # pinned"
        {
          echo "# a channel = \"9.9.9\" in a comment is not a key"
          case "$place" in
            toolchain) printf '[toolchain]\n%s\ncomponents = ["clippy"]\n' "$line" ;;
            other) printf '[toolchain]\ncomponents = ["clippy"]\n\n[other]\n%s\n' "$line" ;;
            comment) printf '[toolchain]\n# %s\n' "$line" ;;
            twice) printf '[toolchain]\n%s\n%s\n' "$line" "$line" ;;
          esac
        } > "$f"
        want=1
        if [ "$place" = toolchain ] && [ "$quote" != none ] && printf '%s\n' "$value" | grep -qxE '[0-9]+\.[0-9]+\.[0-9]+'; then want=0; fi
        out="$(sh "$SCRIPT" "$f" 2> "$T/err")"; got=$?
        label="value='$value' quote=$quote place=$place tail=$tail"
        if [ "$got" != "$want" ]; then bad "$label: exit $got, wanted $want ($(cat "$T/err"))"
        elif [ "$want" = 0 ] && [ "$out" != "$value" ]; then bad "$label: printed '$out', wanted '$value'"
        elif [ "$want" = 1 ] && ! grep -q 'toolchain-channel:' "$T/err"; then bad "$label: refused without saying why"
        elif [ "$want" = 1 ] && [ -n "$out" ]; then bad "$label: refused but printed '$out'"
        else ok; fi
      done
    done
  done
done

# no such file: exit 2, not 1 (a missing file is a call gone wrong, not a bad pin)
sh "$SCRIPT" "$T/missing.toml" > /dev/null 2>&1; st=$?
if [ "$st" = 2 ]; then ok; else bad "a missing file: exit $st, wanted 2"; fi
# a file with no [toolchain] table at all
printf 'channel = "1.100.0"\n' > "$T/notable.toml"
sh "$SCRIPT" "$T/notable.toml" > /dev/null 2>&1; st=$?
if [ "$st" = 1 ]; then ok; else bad "a channel outside any table: exit $st, wanted 1"; fi
# the repository's own file, read by default: one exact version
out="$(sh "$SCRIPT")"; st=$?
if [ "$st" = 0 ] && printf '%s\n' "$out" | grep -qxE '[0-9]+\.[0-9]+\.[0-9]+'; then ok; else bad "the repository's rust-toolchain.toml: exit $st, printed '$out'"; fi

echo "passed=$pass bad=$failn (of $((n + 3)))"
[ "$failn" -eq 0 ]
