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
# The v1 oracle is a CI cell on each Thrust target it was accepted on (#58). The control: the same plan without those lines must fail.
plan_has_oracle() { # plan_has_oracle <cells.txt>
  for t in native-memory native-pg worker-cf; do
    grep -qx "$t oracle" "$1" || { echo "the plan lacks the $t oracle cell"; return 1; }
  done
  echo "plan ok: the oracle cells are planned"
}
expect "plan: the oracle cells are in the real plan" 0 "plan ok: the oracle cells" plan_has_oracle "$T/plan/cells.txt"
mkdir -p "$T/plan2/scripts" && grep -v ' oracle$' "$HERE/scripts/contract-all.sh" > "$T/plan2/scripts/contract-all.sh"
plan_in "$T/plan2" > /dev/null
expect "plan: a plan without the oracle cells is refused" 1 "the plan lacks the native-memory oracle cell" plan_has_oracle "$T/plan2/cells.txt"
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
expect "classify: the oracle's harness alone runs the oracle cells" 0 "rust=true" classify 'tools/oracle-v1/harness/run_conformance.py\n'
expect "classify: the oracle's register alone" 0 "rust=true" classify 'tools/oracle-v1/exceptions.yaml\n'
expect "classify: another tool's directory is not the oracle's" 0 "rust=false" classify 'tools/oracle-v1-notes.md\ntools/spike-0006/README.md\n'
# The lint and deny jobs read these three files, so a change to only one of them must still run the Rust jobs (issue #40):
# classified as docs, `lint` and `deny` would be skipped and the gate would pass without running them.
expect "classify: deny.toml" 0 "rust=true" classify 'deny.toml\n'
expect "classify: clippy.toml" 0 "rust=true" classify 'clippy.toml\n'
expect "classify: rustfmt.toml" 0 "rust=true" classify 'rustfmt.toml\n'
expect "classify: a config look-alike or a nested copy is not the root config" 0 "rust=false" classify 'deny.toml.md\ndocs/clippy.toml\ndocs/src/rustfmt.toml\n'

# ---- assert-gate.sh ------------------------------------------------------------------------------
gatew() { # gatew <dir> <needs-json> <rust> <web> <planned-json> <reported markers...>
  d="$T/gate/$1"; mkdir -p "$d/reported"; needs="$2"; rust="$3"; web="$4"; planned="$5"; shift 5
  for m in "$@"; do printf '%s\n' "$m" > "$d/reported/$(echo "$m" | tr ' ' '-').ok"; done
  ( cd "$d" && NEEDS_JSON="$needs" RUST_CHANGED="$rust" WEB_CHANGED="$web" PLANNED="$planned" REPORTED_DIR="$d/reported" "$S/assert-gate.sh" )
}
gate() { # gate <dir> <needs-json> <rust> <planned-json> <reported markers...>   (web/ unchanged: WEB_CHANGED=false)
  gdir="$1"; gneeds="$2"; grust="$3"; shift 3
  gatew "$gdir" "$gneeds" "$grust" false "$@"
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
LINTF='{"changes":{"result":"success"},"plan":{"result":"success"},"deps":{"result":"success"},"lint":{"result":"failure"},"deny":{"result":"success"},"contract":{"result":"success"}}'
expect "gate: lint failed though every cell passed" 1 "lint failure" gate g13 "$LINTF" true "$P" "native-memory main" "worker-cf bindings"
DENYSK='{"changes":{"result":"success"},"plan":{"result":"success"},"deps":{"result":"success"},"lint":{"result":"success"},"deny":{"result":"skipped"},"contract":{"result":"success"}}'
expect "gate: rust changed but deny SKIPPED (vacuous)" 1 "deny skipped" gate g14 "$DENYSK" true "$P" "native-memory main" "worker-cf bindings"

# ---- the web job (issue #44): judged by WEB_CHANGED, apart from the Rust rules -----------------------------------
WJ() { # WJ <web job result> <rust jobs result>: needs-json with the `web` job; the Rust jobs all <rust jobs result>
  printf '{"changes":{"result":"success"},"plan":{"result":"%s"},"deps":{"result":"%s"},"contract":{"result":"%s"},"web":{"result":"%s"}}' "$2" "$2" "$2" "$1"
}
expect "gate: only web/ changed and the web job passed" 0 "legitimately skipped" gatew w1 "$(WJ success skipped)" false true "[]"
expect "gate: web/ changed but the web job was SKIPPED (vacuous)" 1 "web changed but the web job result is 'skipped'" gatew w2 "$(WJ skipped skipped)" false true "[]"
expect "gate: web/ changed and the web job failed" 1 "web changed but the web job result is 'failure'" gatew w3 "$(WJ failure skipped)" false true "[]"
expect "gate: web/ changed and the web job was cancelled" 1 "web changed but the web job result is 'cancelled'" gatew w4 "$(WJ cancelled skipped)" false true "[]"
expect "gate: web/ changed but the needs have no web job at all" 1 "web changed but the web job result is 'missing'" gatew w5 "$OKN" false true "[]"
# The job is skipped when only Rust changed. Were that a failure, every Rust-only change would fail the gate.
expect "gate: only rust changed, the web job skipped as it should be" 0 "2 of 2 planned cells reported success" gatew w6 "$(WJ skipped success)" true false "$P" "native-memory main" "worker-cf bindings"
expect "gate: rust and web/ both changed and both passed" 0 "2 of 2 planned cells reported success" gatew w7 "$(WJ success success)" true true "$P" "native-memory main" "worker-cf bindings"
expect "gate: web/ unchanged but the web job failed" 1 "the web job result is 'failure'" gatew w8 "$(WJ failure success)" true false "$P" "native-memory main" "worker-cf bindings"
expect "gate: web/ unchanged and no rust change, the web job cancelled" 1 "the web job result is 'cancelled'" gatew w9 "$(WJ cancelled skipped)" false false "[]"
expect "gate: rust changed, web/ changed, and the web job skipped" 1 "web changed but the web job result is 'skipped'" gatew w10 "$(WJ skipped success)" true true "$P" "native-memory main" "worker-cf bindings"
no_web_env() { # the gate run without WEB_CHANGED at all: a workflow that does not pass it must fail, not read it as false
  d="$T/gate/w11"; mkdir -p "$d/reported"
  ( cd "$d" && env -u WEB_CHANGED NEEDS_JSON="$OKN" RUST_CHANGED=false PLANNED="[]" REPORTED_DIR="$d/reported" "$S/assert-gate.sh" )
}
expect "gate: a gate run that is not told WEB_CHANGED fails" 1 "WEB_CHANGED" no_web_env

# ---- classify-changes.sh, the web output --------------------------------------------------------------------------
classify_is() { # classify_is <paths> <want rust> <want web>: the whole output, in order
  # (not `out` or `want`: `expect` keeps its own in globals, and a shell function has no locals)
  cgot="$(printf '%b' "$1" | "$S/classify-changes.sh" 2> /dev/null)"
  cexp="$(printf 'rust=%s\nweb=%s' "$2" "$3")"
  if [ "$cgot" = "$cexp" ]; then echo "classify ok: rust=$2 web=$3"; else echo "classify gave '$cgot', wanted rust=$2 web=$3"; return 1; fi
}
expect "classify: web/ alone is a web change and not a rust change" 0 "classify ok: rust=false web=true" classify_is 'web/apps/web/components/account/auth-forms/password.tsx\n' false true
expect "classify: a crate alone is not a web change" 0 "classify ok: rust=true web=false" classify_is 'crates/thrust-core/src/lib.rs\n' true false
expect "classify: docs alone are neither" 0 "classify ok: rust=false web=false" classify_is 'docs/src/intro.md\n' false false
expect "classify: an empty list fails open for both" 0 "classify ok: rust=true web=true" classify_is '' true true
expect "classify: a workflow change runs both" 0 "classify ok: rust=true web=true" classify_is '.github/workflows/ci.yml\n' true true
expect "classify: the Q-R2 gate script runs both" 0 "classify ok: rust=true web=true" classify_is 'scripts/check-web-diff.sh\n' true true
expect "classify: a web look-alike is not web/" 0 "classify ok: rust=false web=false" classify_is 'web-notes.md\ndocs/web/x.md\nwebsite/index.html\n' false false
echo "passed=$pass bad=$failn"
[ "$failn" -eq 0 ]
