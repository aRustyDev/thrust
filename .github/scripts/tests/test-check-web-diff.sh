#!/bin/sh
# Exercise scripts/check-web-diff.sh (the Q-R2 gate for web/), including the cases that must FAIL.
# Every case goes through `expect`, which checks the exit status AND a message fragment, so a case cannot pass for the
# wrong reason. Each case builds a tiny stand-in for the real layout: a "pin" repository, and a repository whose web/ is a
# copy of it with PIN-MANIFEST, SOURCE and ALLOWED-CHANGES as the real import has them.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
G="$HERE/scripts/check-web-diff.sh"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
pass=0; failn=0
expect() { # expect <label> <want-exit> <want-fragment> <cmd...>
  label="$1"; want="$2"; frag="$3"; shift 3
  "$@" > "$T/last.out" 2>&1
  got=$?
  if [ "$got" = "$want" ] && grep -qF -- "$frag" "$T/last.out"; then
    pass=$((pass + 1)); echo "ok   $label (exit $got; saw '$frag')"
  else
    failn=$((failn + 1)); echo "BAD  $label: wanted exit $want + '$frag', got $got"; sed 's/^/     | /' "$T/last.out"
  fi
}

g() { git -c user.name=t -c user.email=t@t -c commit.gpgsign=false -c core.autocrlf=false "$@"; }

# The pin: four files under apps/web and packages.
P="$T/pin"
mkdir -p "$P/apps/web/app" "$P/packages/utils/src"
printf 'one\n' > "$P/apps/web/app/a.tsx"
printf 'two\n' > "$P/apps/web/package.json"
printf 'three\n' > "$P/packages/utils/src/b.ts"
printf 'four\n' > "$P/packages/utils/package.json"
( cd "$P" && git init -q && g add -A && g commit -q -m pin )
PIN_APPS="$(cd "$P" && git rev-parse HEAD:apps/web)"
PIN_PKGS="$(cd "$P" && git rev-parse HEAD:packages)"

mk() { # mk <name>: an imported repository, committed, with an empty allowlist
  d="$T/$1"; mkdir -p "$d/web"
  cp -R "$P/apps" "$P/packages" "$d/web/"
  ( cd "$P" && git ls-tree -r HEAD apps/web packages ) > "$d/web/PIN-MANIFEST"
  printf 'source_tree_apps_web=%s\nsource_tree_packages=%s\n' "$PIN_APPS" "$PIN_PKGS" > "$d/web/SOURCE"
  printf '# nothing is allowed to differ\n' > "$d/web/ALLOWED-CHANGES"
  ( cd "$d" && git init -q && g add -A && g commit -q -m import )
}
commit() { ( cd "$T/$1" && g add -A && g commit -q -m "$2" ); } # commit <name> <message>
run() { ( cd "$T/$1" && "$G" "${2:-HEAD}" ); }                 # run <name> [<ref>]
A=web/apps/web/app/a.tsx

mk ok
expect "the import as it is passes" 0 "4 files in the pin's apps/web and packages, 0 differ, all of them allowed" run ok

mk m1; printf 'changed\n' > "$T/m1/$A"; commit m1 edit
expect "a changed file that is not listed fails" 1 "not in web/ALLOWED-CHANGES (Q-R2): apps/web/app/a.tsx" run m1
# the same history: the commit before the edit still passes, so the ref argument is read and not the working tree
expect "an earlier ref passes though HEAD fails" 0 "0 differ" run m1 HEAD~1

mk m2; printf 'changed\n' > "$T/m2/$A"; printf '  apps/web/app/a.tsx   # the auth client\n\n' >> "$T/m2/web/ALLOWED-CHANGES"; commit m2 edit
expect "a changed file that is listed passes (indent and a trailing comment parse)" 0 "1 differ, all of them allowed" run m2

mk m3; printf 'apps/web/app/a.tsx\n' >> "$T/m3/web/ALLOWED-CHANGES"; commit m3 list
expect "a listed file that does not differ is a stale line and fails" 1 "do not differ from the pin; remove the line: apps/web/app/a.tsx" run m3

mk m4; ( cd "$T/m4" && g rm -q web/packages/utils/src/b.ts ) && commit m4 delete
expect "a deleted file that is not listed fails" 1 "not in web/ALLOWED-CHANGES (Q-R2): packages/utils/src/b.ts" run m4

mk m5; printf 'new\n' > "$T/m5/web/packages/utils/src/c.ts"; commit m5 add
expect "an added file that is not listed fails" 1 "not in web/ALLOWED-CHANGES (Q-R2): packages/utils/src/c.ts" run m5

mk m6; printf 'new\n' > "$T/m6/web/packages/utils/src/c.ts"; printf 'packages/utils/src/c.ts\n' >> "$T/m6/web/ALLOWED-CHANGES"; commit m6 add
expect "an added file that is listed passes" 0 "1 differ, all of them allowed" run m6

mk m7; chmod +x "$T/m7/web/packages/utils/src/b.ts"; commit m7 mode
expect "a mode change that is not listed fails" 1 "not in web/ALLOWED-CHANGES (Q-R2): packages/utils/src/b.ts" run m7

# A manifest edited to agree with a changed tree: the tree no longer rebuilds the hashes that web/SOURCE records for the pin.
mk m8; ( cd "$T/m8" && g rm -q $A ) && grep -v 'apps/web/app/a.tsx' "$T/m8/web/PIN-MANIFEST" > "$T/m8.manifest" && cp "$T/m8.manifest" "$T/m8/web/PIN-MANIFEST"; commit m8 hide
expect "a doctored manifest fails the tree-hash check" 1 "does not rebuild the pin's tree apps/web" run m8

mk m9; ( cd "$T/m9" && g rm -q web/ALLOWED-CHANGES ) && commit m9 rm
expect "a missing allowlist fails (the check cannot be switched off by deleting its list)" 1 "no web/ALLOWED-CHANGES" run m9

mk m10; printf 'package.json\n' >> "$T/m10/web/ALLOWED-CHANGES"; commit m10 outside
expect "an entry outside the compared trees fails" 1 "outside apps/web and packages" run m10

mk m11; ( cd "$T/m11" && g rm -q -r web/apps web/packages ) && commit m11 empty
expect "empty trees fail rather than agreeing with nothing" 1 "have no files" run m11

mk m12; : > "$T/m12/web/PIN-MANIFEST"; commit m12 empty
expect "an empty manifest fails" 1 "PIN-MANIFEST is empty" run m12

mk m13; printf 'nothing\n' > "$T/m13/web/SOURCE"; commit m13 source
expect "a SOURCE that records no tree hash fails" 1 "web/SOURCE has no source_tree_apps_web" run m13

echo "passed=$pass bad=$failn"
[ "$failn" -eq 0 ]
