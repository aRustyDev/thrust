#!/bin/sh
# scripts/mutants.sh as a matrix of shards (scripts/lib/mutants-shard.sh). What has to hold is that the shards TOGETHER run every row exactly once: a
# row lost passes for a suite that was never shown to fail, a row run twice only costs time, and neither shows in a green job. So:
#   1. the library, exhaustively: for 1 to 8 shards and every index, the rows a shard selects are exactly the positions k with (k - 1) mod shards = index
#      (an awk one-liner says that independently), the list mode prints the same, and `shard_finish` returns 0 only when the rows selected were all judged;
#   2. the real script, with `scripts/contract.sh` and `scripts/rs` replaced by stubs that record what they were asked to run (a copy of the tree in a
#      temp dir): the shards of 2, 4 and 7 together run what one run does, once each; the list names every row that runs; the guard stays quiet;
#   3. the guards: a shard with no row fails, an ungated row is caught by the verdict count, a bad index or count is refused (exit 2);
#   4. the controls: deliberately broken copies (a wrong modulus, a `want` that always selects, a row with no `want`, each guard removed), each of which
#      the checks above must reject. A check that cannot fail is not evidence (AGENTS.md rule 8).
#
#   test-mutants-shards.sh
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
LIB="$HERE/scripts/lib/mutants-shard.sh"
MUTANTS="$HERE/scripts/mutants.sh"
VERDICT="$HERE/scripts/lib/mutants-verdict.sh"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
pass=0; failn=0; n=0
ok() { pass=$((pass + 1)); }
bad() { failn=$((failn + 1)); echo "BAD  $*"; }
# check <label> <command...>: passes when the command succeeds; its output is shown when it does not
check() {
  label="$1"; shift
  n=$((n + 1))
  if out="$("$@" 2>&1)"; then ok; else bad "$label: $out"; fi
}
# control <label> <command...>: passes when the command FAILS, which is what a deliberately broken copy must make it do
control() {
  label="$1"; shift
  n=$((n + 1))
  if out="$("$@" 2>&1)"; then bad "control '$label' was not caught (the check passed on a broken copy)"; else ok; if [ -n "${VERBOSE:-}" ]; then printf 'caught  %s\n          %s\n' "$label" "$(printf '%s' "$out" | head -n 1)"; fi; fi
}

# 1. the library
lib_positions_ok() {
  lp_lib="$1"
  for lp_n in 1 2 3 4 5 6 7 8; do
    lp_i=0
    while [ "$lp_i" -lt "$lp_n" ]; do
      got="$(MUTANTS_SHARDS=$lp_n MUTANTS_SHARD=$lp_i sh -c '. "$1"; shard_init || exit 2; k=0; while [ "$k" -lt 60 ]; do k=$((k + 1)); if want "r$k" >/dev/null; then printf "%s " "$k"; fi; done; echo "| $ROWS_SELECTED"' _ "$lp_lib")"
      expect="$(awk -v n="$lp_n" -v i="$lp_i" 'BEGIN { c = 0; for (k = 1; k <= 60; k++) if ((k - 1) % n == i) { printf "%s ", k; c++ } print "| " c }')"
      if [ "$got" != "$expect" ]; then echo "$lp_n shards, index $lp_i: selected '$got', wanted '$expect'"; return 1; fi
      lp_i=$((lp_i + 1))
    done
  done
}
lib_list_ok() {
  got="$(MUTANTS_SHARDS=3 MUTANTS_SHARD=0 MUTANTS_LIST=1 sh -c '. "$1"; shard_init; for r in a b c d e f g; do want "row $r"; done' _ "$1")"
  expect="$(awk 'BEGIN { split("a b c d e f g", r, " "); for (k = 1; k <= 7; k++) print k " " (k - 1) % 3 " row " r[k] }')"
  [ "$got" = "$expect" ] || { echo "the list says '$got', wanted '$expect'"; return 1; }
}
# `shard_finish <verdicts>` returns 5 unless something was selected and every selected row was judged, whatever the counts
lib_finish_ok() {
  lf_s=0
  while [ "$lf_s" -le 3 ]; do
    lf_v=0
    while [ "$lf_v" -le 4 ]; do
      got="$(sh -c '. "$1"; MUTANTS_SHARDS=2 MUTANTS_SHARD=1 shard_init; ROW_N=10; ROWS_SELECTED=$2; shard_finish "$3" 2>/dev/null; echo $?' _ "$1" "$lf_s" "$lf_v")"
      expect=0; if [ "$lf_s" -eq 0 ] || [ "$lf_v" -ne "$lf_s" ]; then expect=5; fi
      if [ "$got" != "$expect" ]; then echo "selected $lf_s, judged $lf_v: shard_finish returned $got, wanted $expect"; return 1; fi
      lf_v=$((lf_v + 1))
    done
    lf_s=$((lf_s + 1))
  done
}
check "every shard count and index selects the positions it should" lib_positions_ok "$LIB"
check "the list mode prints each row with its shard" lib_list_ok "$LIB"
check "shard_finish returns 0 only when something ran and every row selected was judged" lib_finish_ok "$LIB"

# 2. the real script against stubs
mk_tree() {
  # <dir> <mutants.sh to use> <shard library to use>
  mkdir -p "$1/scripts/lib"
  cp "$2" "$1/scripts/mutants.sh"
  cp "$VERDICT" "$1/scripts/lib/mutants-verdict.sh"
  cp "$3" "$1/scripts/lib/mutants-shard.sh"
  printf '%s\n' '#!/bin/sh' 'echo "contract $* features=${THRUST_SERVER_FEATURES:-} csrf=${THRUST_SERVER_CSRF_ENFORCE:-}" >>"$RAN_LOG"' 'exit 0' >"$1/scripts/contract.sh"
  printf '%s\n' '#!/bin/sh' 'echo "adapter $*" >>"$RAN_LOG"' 'exit 0' >"$1/scripts/rs"
  chmod +x "$1/scripts/contract.sh" "$1/scripts/rs"
}
run_shard() {
  # <tree> <shards> <index> <tag>: what ran goes to $T/<tag>-<shards>-<index>.ran, stderr to .err, the exit status to .status
  rs_f="$T/$4-$2-$3"
  : >"$rs_f.ran"
  RAN_LOG="$rs_f.ran" MUTANTS_SHARDS="$2" MUTANTS_SHARD="$3" sh "$1/scripts/mutants.sh" >"$rs_f.out" 2>"$rs_f.err"
  echo $? >"$rs_f.status"
}
partition_ok() {
  # <tree> <tag>: one run is the baseline, the list names every row that runs, and 2, 4 and 7 shards together run the baseline's rows once each
  po_tree="$1"; po_tag="$2"
  run_shard "$po_tree" 1 0 "$po_tag"
  sort "$T/$po_tag-1-0.ran" >"$T/$po_tag.all"
  rows="$(wc -l <"$T/$po_tag.all" | tr -d ' ')"
  if [ "$rows" -lt 1 ]; then echo "no row ran at all"; return 1; fi
  listed="$(RAN_LOG=/dev/null MUTANTS_SHARDS=1 MUTANTS_SHARD=0 MUTANTS_LIST=1 sh "$po_tree/scripts/mutants.sh" 2>/dev/null | wc -l | tr -d ' ')"
  if [ "$listed" != "$rows" ]; then echo "the list names $listed rows and $rows run"; return 1; fi
  for po_n in 2 4 7; do
    : >"$T/$po_tag-$po_n.cat"
    po_i=0
    while [ "$po_i" -lt "$po_n" ]; do
      run_shard "$po_tree" "$po_n" "$po_i" "$po_tag"
      cat "$T/$po_tag-$po_n-$po_i.ran" >>"$T/$po_tag-$po_n.cat"
      if grep -q 'VACUITY GUARD' "$T/$po_tag-$po_n-$po_i.err"; then echo "the guard fired on shard $po_i of $po_n: $(cat "$T/$po_tag-$po_n-$po_i.err")"; return 1; fi
      po_i=$((po_i + 1))
    done
    if ! sort "$T/$po_tag-$po_n.cat" | cmp -s - "$T/$po_tag.all"; then echo "$po_n shards did not run the rows of one run once each"; return 1; fi
  done
}
empty_shard_fails() {
  # more shards than rows: the last shard has none, and a run that ran nothing has killed nothing. The count of rows is the script's own (the
  # list mode names each), so that a row added to mutants.sh does not turn "more shards than rows" into a shard that has one
  ee_rows="$(RAN_LOG=/dev/null MUTANTS_SHARDS=1 MUTANTS_SHARD=0 MUTANTS_LIST=1 sh "$1/scripts/mutants.sh" 2>/dev/null | wc -l | tr -d ' ')"
  ee_n=$((ee_rows + 1))
  ee_i=$((ee_n - 1))
  ee_f="$T/$2-empty-$ee_n-$ee_i"
  run_shard "$1" "$ee_n" "$ee_i" "$2-empty"
  if [ "$(cat "$ee_f.status")" -eq 0 ]; then echo "a shard with no row exited 0"; return 1; fi
  grep -q 'selected no row' "$ee_f.err" || { echo "it did not say why: $(cat "$ee_f.err")"; return 1; }
  if [ -s "$ee_f.ran" ]; then echo "it ran something"; return 1; fi
}
guard_fires() {
  # a row outside `want` runs on every shard, so a shard judges more rows than it selected
  run_shard "$1" 4 1 "$2-fires"
  if ! grep -q 'VACUITY GUARD.*recorded' "$T/$2-fires-4-1.err"; then echo "the verdict count did not catch it: $(cat "$T/$2-fires-4-1.err")"; return 1; fi
}
progress_ok() {
  # a row's own output goes to a file, so the job log names each row the shard starts, once, and only the shard's own (shard 1 of 4: positions 2, 6, 10, ...)
  run_shard "$1" 4 1 "$2-progress"
  got="$(awk '/^row [0-9]+: / { k = $2 + 0; if ((k - 1) % 4 != 1) wrong = 1; c++ } END { print c + 0, wrong + 0 }' "$T/$2-progress-4-1.out")"
  ran="$(wc -l <"$T/$2-progress-4-1.ran" | tr -d ' ')"
  if [ "$got" != "$ran 0" ]; then echo "the log names '$got' (rows, of another shard) for $ran rows run"; return 1; fi
}
fail_fast_ok() {
  # the discard-noop row ends at its first failure, and no other adapter row does: the others look for a needle that one aimed test prints, and a test
  # terminated because another failed first would turn their kill into UNCLEAR
  run_shard "$1" 1 0 "$2-ff"
  ff_adapters="$(grep '^adapter ' "$T/$2-ff-1-0.ran")"
  ff_with="$(printf '%s\n' "$ff_adapters" | grep -c -- '--max-fail=1:immediate')"
  if [ "$ff_with" != 1 ]; then echo "$ff_with adapter rows end at their first failure, wanted 1"; return 1; fi
  if ! printf '%s\n' "$ff_adapters" | grep -- '--max-fail=1:immediate' | grep -q -- '--features mutant_discard_noop '; then echo "the row that does is not discard-noop"; return 1; fi
}
bad_settings_refused() {
  # shards:index pairs that are not a shard of a split, each refused with exit 2 and before a row runs
  for bs_pair in 4:4 0:0 4:x 04:1 4: :1 4:-1 3:7; do
    run_shard "$1" "${bs_pair%%:*}" "${bs_pair#*:}" "$2-bad"
    bs_f="$T/$2-bad-${bs_pair%%:*}-${bs_pair#*:}"
    if [ "$(cat "$bs_f.status")" -ne 2 ] || [ -s "$bs_f.ran" ]; then echo "'$bs_pair' was not refused (exit $(cat "$bs_f.status"))"; return 1; fi
  done
}

mk_tree "$T/real" "$MUTANTS" "$LIB"
check "the shards together run every row once, the list names those rows, the guard stays quiet" partition_ok "$T/real" real
check "a shard with no row fails and says so" empty_shard_fails "$T/real" real
check "a count or an index that is not a shard of a split is refused before a row runs" bad_settings_refused "$T/real" real
check "the log names each row the shard starts, and only its own" progress_ok "$T/real" real
check "the discard-noop row ends at its first failure, and no other adapter row does" fail_fast_ok "$T/real" real

# 3. the controls: each broken copy must be rejected by the check aimed at it
variant() {
  # <out file> <source file> <sed expression>: a copy that differs from the source (else the control would prove nothing)
  sed "$3" "$2" >"$1"
  if cmp -s "$1" "$2"; then bad "the control '$3' changed nothing in $2"; fi
}
variant "$T/k1.sh" "$LIB" 's/% SHARD_N))/% (SHARD_N + 1)))/'
variant "$T/k2.sh" "$LIB" 's/if \[ "\$w_shard" -eq "\$SHARD_I" \]; then/if true; then/'
variant "$T/k4.sh" "$LIB" 's/if \[ "\$ROWS_SELECTED" -eq 0 \]; then/if false; then/'
variant "$T/k5.sh" "$LIB" 's/if \[ "\$1" -ne "\$ROWS_SELECTED" \]; then/if false; then/'
variant "$T/k7.sh" "$LIB" 's/\[ "\$SHARD_I" -ge "\$SHARD_N" \]/false/'
variant "$T/k8.sh" "$LIB" '/^    echo "row \$ROW_N: \$1"$/d'
variant "$T/k3-mutants.sh" "$MUTANTS" '/want "cloudflare email adapter + \$name" || continue/d'
variant "$T/k9-mutants.sh" "$MUTANTS" "s/ --max-fail=1:immediate\$//"
variant "$T/k10-mutants.sh" "$MUTANTS" 's/-E "\$ar_set" "\$@"/-E "$ar_set" --max-fail=1:immediate "$@"/'

control "a wrong modulus: positions" lib_positions_ok "$T/k1.sh"
control "a want that always selects: positions" lib_positions_ok "$T/k2.sh"
control "no check that something was selected: shard_finish" lib_finish_ok "$T/k4.sh"
control "no check that every selected row was judged: shard_finish" lib_finish_ok "$T/k5.sh"
mk_tree "$T/k1" "$MUTANTS" "$T/k1.sh"
control "a wrong modulus: the shards together" partition_ok "$T/k1" k1
mk_tree "$T/k2" "$MUTANTS" "$T/k2.sh"
control "a want that always selects: the shards together" partition_ok "$T/k2" k2
mk_tree "$T/k3" "$T/k3-mutants.sh" "$LIB"
control "a row with no want: the list and the shards together" partition_ok "$T/k3" k3
check "a row with no want: the verdict count catches it" guard_fires "$T/k3" k3
mk_tree "$T/k5" "$T/k3-mutants.sh" "$T/k5.sh"
control "a row with no want and no verdict count: nothing catches it, so the check is what did" guard_fires "$T/k5" k5
mk_tree "$T/k4" "$MUTANTS" "$T/k4.sh"
control "no check that something was selected: an empty shard" empty_shard_fails "$T/k4" k4
mk_tree "$T/k7" "$MUTANTS" "$T/k7.sh"
control "an index past the last shard accepted" bad_settings_refused "$T/k7" k7
mk_tree "$T/k8" "$MUTANTS" "$T/k8.sh"
control "a row that is not named in the log when it starts" progress_ok "$T/k8" k8
mk_tree "$T/k9" "$T/k9-mutants.sh" "$LIB"
control "the discard-noop row no longer ends at its first failure" fail_fast_ok "$T/k9" k9
mk_tree "$T/k10" "$T/k10-mutants.sh" "$LIB"
control "every adapter row ends at its first failure" fail_fast_ok "$T/k10" k10

echo "passed=$pass bad=$failn (of $n)"
[ "$failn" -eq 0 ] && [ "$pass" = "$n" ]
