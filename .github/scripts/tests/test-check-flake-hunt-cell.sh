#!/bin/sh
# check-flake-hunt-cell.sh: a target and pass are accepted exactly when they are a cell of scripts/contract-all.sh, and the iteration
# count exactly when it is a whole number from 1 to 50. Generated: every target (and one that is not) x every pass contract.sh knows
# (and one it does not), against the cells read again here a different way; then iteration counts, usage errors, a matrix that
# gains a pass, and the error text. Last, flake-hunt.yml's two choice lists must equal the matrix's targets and passes, so a pass
# added to the matrix cannot be left out of the workflow (the gap this script closed: csrfoff, mail, oracle and e2e were missing),
# and both flake-hunt.yml and contract-cell.yml must prepare a pass through the one shared action.
#
#   test-check-flake-hunt-cell.sh [path to a copy of check-flake-hunt-cell.sh]   (a path points it at a deliberately broken copy)
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
CHECK="${1:-$HERE/.github/scripts/check-flake-hunt-cell.sh}"
ALL="$HERE/scripts/contract-all.sh"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
pass=0; failn=0; n=0
ok() { pass=$((pass + 1)); }
bad() { failn=$((failn + 1)); echo "BAD  $*"; }

# expect <label> <want exit> <needle or ''> <args...>
expect() {
  label="$1"; want="$2"; needle="$3"; shift 3
  n=$((n + 1))
  sh "$CHECK" "$@" > "$T/out" 2>&1; got=$?
  if [ "$got" != "$want" ]; then bad "$label: exit $got, wanted $want"; sed 's/^/     | /' "$T/out"; return; fi
  if [ -n "$needle" ] && ! grep -qF -- "$needle" "$T/out"; then bad "$label: output lacks '$needle'"; sed 's/^/     | /' "$T/out"; return; fi
  ok
}

# is_cell <target> <pass> <file>: field by field, not the script's regular expression
is_cell() { awk -v t="$1" -v p="$2" '$1 == "run" && $2 == t && $3 == p && NF == 3 { f = 1 } END { exit !f }' "$3"; }

# 1. every target x every pass contract.sh knows, against this repository's matrix
cells=0
for t in native-memory native-pg worker-memory worker-cf native-sqlite; do
  for p in main sweeper csrfoff panic blast bench scale worker cron engine bindings e2e mail oracle budget nope; do
    if is_cell "$t" "$p" "$ALL"; then want=0; cells=$((cells + 1)); else want=1; fi
    expect "$t $p" "$want" "" "$t" "$p" 5 "$ALL"
  done
done
# the generated cases must have met every cell of the matrix, or the lists above are out of date
total="$(awk '$1 == "run" && NF == 3' "$ALL" | wc -l | tr -d ' ')"
n=$((n + 1))
if [ "$cells" -ge 1 ] && [ "$cells" = "$total" ]; then ok; else bad "the cases met $cells cells, the matrix has $total: add the new target or pass above"; fi

# 2. iteration counts
for it in 1 2 9 10 49 50; do expect "iterations $it" 0 "" native-memory main "$it" "$ALL"; done
for it in 0 00 05 51 99 100 -1 +5 "" " 5" "5 " abc 5x 1.5 99999999999999999999; do
  expect "iterations '$it'" 1 "::error::iterations" native-memory main "$it" "$ALL"
done
# a bad count is refused before the cell is looked at
expect "a bad count on a bad cell names the count" 1 "::error::iterations" native-memory nope 0 "$ALL"

# 3. usage
expect "two arguments" 2 "usage" native-memory main
expect "five arguments" 2 "usage" native-memory main 5 "$ALL" extra
expect "a missing matrix" 2 "not found" native-memory main 5 "$T/none.sh"
printf '#!/bin/sh\necho no cells here\n' > "$T/empty.sh"
expect "a matrix with no cells" 2 "parsed zero cells" native-memory main 5 "$T/empty.sh"

# 4. the matrix is read, not a list in the script: a pass added to it can be hunted, and one removed cannot
printf 'run() { :; }\nrun native-memory main\nrun native-memory flux\nrun worker-cf e2e\n#run native-memory panic\n  run native-memory cron\nrun native-memory mail extra\n' > "$T/m.sh"
expect "a new pass is hunted the day it is added" 0 "native-memory flux, 5 iterations" native-memory flux 5 "$T/m.sh"
expect "a pass the matrix dropped is refused" 1 "is not a cell" native-memory csrfoff 5 "$T/m.sh"
expect "a cell's pass on another target is refused" 1 "is not a cell" native-memory e2e 5 "$T/m.sh"
expect "a commented-out cell is not a cell" 1 "is not a cell" native-memory panic 5 "$T/m.sh"
expect "an indented line is not a cell" 1 "is not a cell" native-memory cron 5 "$T/m.sh"
expect "a line with a third word is not a cell" 1 "is not a cell" native-memory mail 5 "$T/m.sh"
expect "a prefix of a cell is not a cell" 1 "" native-memory mai 5 "$T/m.sh"
expect "a target and pass in one argument is not a cell" 1 "" "native-memory main" "" 5 "$T/m.sh"

# 5. the error says what the target does run
expect "the error names the target's passes" 1 "worker-cf runs: main sweeper panic cron mail worker bindings oracle e2e" worker-cf csrfoff 5 "$ALL"
expect "an unknown target is named as one" 1 "'native-sqlite' is not a target" native-sqlite main 5 "$ALL"

# 6. flake-hunt.yml offers exactly the matrix's targets and passes
choice_options() { # <workflow> <input>: the options of one `type: choice` input, sorted
  awk -v want="$2" '
    /^      [a-z][a-z-]*:$/ { inp = $1; sub(/:$/, "", inp); inopt = 0; next }
    inp == want && /^        options:$/ { inopt = 1; next }
    inopt && /^          - / { print $2; next }
    { inopt = 0 }
  ' "$1" | sort
}
matrix_field() { awk -v f="$2" '$1 == "run" && NF == 3 { print $f }' "$1" | sort -u; }
lists_match() { # <workflow> <matrix>: 0 when both choice lists equal the matrix's
  choice_options "$1" target > "$T/wt"; matrix_field "$2" 2 > "$T/mt"
  choice_options "$1" pass > "$T/wp"; matrix_field "$2" 3 > "$T/mp"
  [ -s "$T/wt" ] && [ -s "$T/wp" ] && cmp -s "$T/wt" "$T/mt" && cmp -s "$T/wp" "$T/mp"
}
WF="$HERE/.github/workflows/flake-hunt.yml"
n=$((n + 1))
if lists_match "$WF" "$ALL"; then ok; else
  bad "flake-hunt.yml's choices differ from scripts/contract-all.sh (< workflow, > matrix)"
  diff "$T/wt" "$T/mt" | sed 's/^/     | target /'; diff "$T/wp" "$T/mp" | sed 's/^/     | pass /'
fi
# controls: the comparison must fail on a workflow that lost a pass, gained one, lost a target, or has no choices at all
grep -v '^          - e2e$' "$WF" > "$T/lost.yml"
awk '{ print } $0 == "          - bindings" { print "          - blast" }' "$WF" > "$T/gained.yml"
grep -v '^          - native-pg$' "$WF" > "$T/target.yml"
grep -v '^          - ' "$WF" > "$T/none.yml"
for c in lost gained target none; do
  n=$((n + 1))
  if lists_match "$T/$c.yml" "$ALL"; then bad "control '$c': a changed workflow still matched the matrix"; else ok; fi
done
# and on a matrix that gained a pass the workflow does not offer
{ cat "$ALL"; echo "run native-memory flux"; } > "$T/more.sh"
n=$((n + 1))
if lists_match "$WF" "$T/more.sh"; then bad "control: a new matrix pass went unnoticed"; else ok; fi

# 7. one place prepares a pass: both workflows use the shared action, and neither installs uv or the browser itself
for w in flake-hunt.yml contract-cell.yml; do
  n=$((n + 1))
  if grep -q '^      - uses: \./\.github/actions/contract-pass-tools$' "$HERE/.github/workflows/$w"; then ok; else bad "$w does not use ./.github/actions/contract-pass-tools"; fi
  n=$((n + 1))
  if grep -qE 'astral-sh/setup-uv|playwright install' "$HERE/.github/workflows/$w"; then bad "$w prepares a pass itself instead of through the action"; else ok; fi
done
# flake-hunt validates before it builds anything
n=$((n + 1))
v="$(grep -n 'check-flake-hunt-cell.sh' "$WF" | head -1 | cut -d: -f1)"
s="$(grep -n 'uses: \./\.github/actions/setup-rust' "$WF" | head -1 | cut -d: -f1)"
if [ -n "$v" ] && [ -n "$s" ] && [ "$v" -lt "$s" ]; then ok; else bad "flake-hunt.yml must run check-flake-hunt-cell.sh before setup-rust (lines '$v', '$s')"; fi

echo "passed=$pass bad=$failn (of $n)"
[ "$failn" -eq 0 ] && [ "$pass" = "$n" ]
