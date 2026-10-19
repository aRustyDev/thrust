#!/bin/sh
# Exercise plan-cells.sh, classify-changes.sh and assert-gate.sh, including the cases that must FAIL.
# Every case goes through `expect`, which checks the exit status AND a message fragment, so a case cannot
# pass for the wrong reason and none is merely printed.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
S="$HERE/.github/scripts"
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

# ---- plan-cells.sh -------------------------------------------------------------------------------
plan_in() { # plan_in <dir>   (runs plan-cells.sh against <dir>/scripts/contract-all.sh)
  ( cd "$1" && "$S/plan-cells.sh" )
}
mkdir -p "$T/plan/scripts" && cp "$HERE/scripts/contract-all.sh" "$T/plan/scripts/contract-all.sh"
plan_real() { # every `run <target> <pass>` line of the real script must become exactly one cell
  plan_in "$T/plan" > "$T/plan/out.txt" || return 1
  n_cells="$(wc -l < "$T/plan/cells.txt" | tr -d ' ')"
  n_runs="$(grep -cE '^run[[:space:]]' "$HERE/scripts/contract-all.sh")"
  [ "$n_cells" -ge 1 ] && [ "$n_cells" = "$n_runs" ] && grep -q '^matrix=' "$T/plan/out.txt" && grep -q '^cells=' "$T/plan/out.txt" \
    && echo "plan ok: every run line became a cell"
}
expect "plan: every run line of the real contract-all.sh becomes a cell" 0 "plan ok" plan_real
mkdir -p "$T/plan0/scripts" && printf '#!/bin/sh\necho nothing\n' > "$T/plan0/scripts/contract-all.sh"
expect "plan: a script with no run lines fails" 1 "parsed zero cells" plan_in "$T/plan0"
mkdir -p "$T/plan1/scripts" && cp "$HERE/scripts/contract-all.sh" "$T/plan1/scripts/contract-all.sh" && printf 'run native-memory main extra\n' >> "$T/plan1/scripts/contract-all.sh"
expect "plan: a run line with a changed format fails" 1 "the format changed" plan_in "$T/plan1"

# ---- classify-changes.sh -------------------------------------------------------------------------
classify() { printf '%b' "$1" | "$S/classify-changes.sh"; }
expect "classify: docs only" 0 "rust=false" classify 'docs/src/intro.md\nREADME.md\n'
expect "classify: a crate change" 0 "rust=true" classify 'docs/src/intro.md\ncrates/thrust-core/src/lib.rs\n'
expect "classify: the lockfile" 0 "rust=true" classify 'Cargo.lock\n'
expect "classify: the nextest config" 0 "rust=true" classify '.config/nextest.toml\n'
expect "classify: a workflow change" 0 "rust=true" classify '.github/workflows/ci.yml\n'
expect "classify: a helper script change" 0 "rust=true" classify '.github/scripts/assert-gate.sh\n'
expect "classify: an empty list fails open" 0 "rust=true" classify ''
expect "classify: a look-alike path is not a crate" 0 "rust=false" classify 'crates-notes.md\n'

# ---- assert-gate.sh ------------------------------------------------------------------------------
gate() { # gate <dir> <needs-json> <rust> <planned-json> <reported markers...>
  d="$T/gate/$1"; mkdir -p "$d/reported"; needs="$2"; rust="$3"; planned="$4"; shift 4
  for m in "$@"; do printf '%s\n' "$m" > "$d/reported/$(echo "$m" | tr ' ' '-').ok"; done
  ( cd "$d" && NEEDS_JSON="$needs" RUST_CHANGED="$rust" PLANNED="$planned" REPORTED_DIR="$d/reported" "$S/assert-gate.sh" )
}
OKN='{"changes":{"result":"success"},"plan":{"result":"success"},"deps":{"result":"success"},"contract":{"result":"success"}}'
P='["native-memory main","worker-cf bindings"]'
expect "gate: all planned cells reported" 0 "2 of 2 planned cells reported success" gate g1 "$OKN" true "$P" "native-memory main" "worker-cf bindings"
expect "gate: one planned cell never reported" 1 "planned cells that never reported success: worker-cf bindings" gate g2 "$OKN" true "$P" "native-memory main"
expect "gate: marker for an unplanned cell" 1 "not in the planned set: native-pg main" gate g3 "$OKN" true "$P" "native-memory main" "worker-cf bindings" "native-pg main"
expect "gate: empty planned set" 1 "the planned cell set is empty" gate g4 "$OKN" true "[]" "native-memory main"
expect "gate: no markers at all" 1 "no cell markers found" gate g5 "$OKN" true "$P"
SKIP='{"changes":{"result":"success"},"plan":{"result":"success"},"deps":{"result":"skipped"},"contract":{"result":"success"}}'
expect "gate: rust changed but a needed job SKIPPED (vacuous)" 1 "deps skipped" gate g6 "$SKIP" true "$P" "native-memory main" "worker-cf bindings"
BADJ='{"changes":{"result":"success"},"plan":{"result":"success"},"deps":{"result":"failure"},"contract":{"result":"success"}}'
expect "gate: a needed job failed" 1 "deps failure" gate g7 "$BADJ" true "$P" "native-memory main" "worker-cf bindings"
CHF='{"changes":{"result":"failure"},"plan":{"result":"skipped"},"deps":{"result":"skipped"},"contract":{"result":"skipped"}}'
expect "gate: changes job failed" 1 "changes job result is" gate g8 "$CHF" false "[]"
NOR='{"changes":{"result":"success"},"plan":{"result":"skipped"},"deps":{"result":"skipped"},"contract":{"result":"skipped"}}'
expect "gate: docs-only change, heavy jobs skipped" 0 "legitimately skipped" gate g9 "$NOR" false "[]"
NORF='{"changes":{"result":"success"},"plan":{"result":"skipped"},"deps":{"result":"cancelled"},"contract":{"result":"skipped"}}'
expect "gate: docs-only but a job was cancelled" 1 "deps cancelled" gate g10 "$NORF" false "[]"
CANC='{"changes":{"result":"success"},"plan":{"result":"success"},"deps":{"result":"success"},"contract":{"result":"cancelled"}}'
expect "gate: matrix cancelled" 1 "contract cancelled" gate g11 "$CANC" true "$P" "native-memory main" "worker-cf bindings"
HERF='{"changes":{"result":"success"},"plan":{"result":"success"},"deps":{"result":"success"},"hermetic":{"result":"failure"},"contract":{"result":"success"}}'
expect "gate: hermetic tests failed though every cell passed" 1 "hermetic failure" gate g12 "$HERF" true "$P" "native-memory main" "worker-cf bindings"
echo "passed=$pass bad=$failn"
[ "$failn" -eq 0 ]
