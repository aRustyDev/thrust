#!/bin/sh
# .github/scripts/classify-deploy.sh against a real git history (issue #49): every ordered pair of commits in a made-up history, with the
# answer worked out here, independently, from which commits touched which kind of file; then the cases it must treat as "cannot tell".
#
# The oracle is not the script: each commit of the made-up history is labelled by hand with the STATE of the migrations directory and of
# the `[[migrations]]` tables it leaves behind, and with its parent, and the expected answer for the pair (live i, deploy j) is
#   - "older" (both true) when j is a strict ancestor of i, by the hand-written parent table and not by git: a redeploy of older code;
#   - otherwise "the two labels differ", whether or not one descends from the other.
# The second half is the point of the history's side lines: release-plz force-updates the release-candidate branch, so the commit that is
# live is usually not an ancestor of the next one (sibling commits, diverged lines), and a classifier that sent every such pair to the
# manual gate would stop deploys that change nothing a person has to decide (the first two real staging runs after the drill).
# The history has commits that change neither state, one that only edits a comment inside a `[[migrations]]` table (not a lifecycle
# change), ones that do both, one that undoes an earlier lifecycle change (the net state is the old one, so nothing is different), and
# side lines that change one state, the other, or neither, so a classifier that compared the wrong thing disagrees somewhere.
#
#   sh .github/scripts/tests/test-classify-deploy.sh
#
# CLASSIFY_SCRIPT points the test at a copy of the script, so a deliberately broken copy can be shown to fail it.
# shellcheck disable=SC2034 # the commit ids S0..Se2 are read by name through eval, in sha_of
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
SCRIPT="${CLASSIFY_SCRIPT:-$HERE/.github/scripts/classify-deploy.sh}"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
pass=0; failn=0

# no user or system git configuration: this repository must not sign commits through a password manager, or ask for an identity
GIT_CONFIG_GLOBAL=/dev/null; GIT_CONFIG_SYSTEM=/dev/null
export GIT_CONFIG_GLOBAL GIT_CONFIG_SYSTEM

R="$T/repo"
mkdir -p "$R/crates/worker" "$R/crates/adapters/postgres/migrations"
cd "$R" || exit 1
git init -q -b main . || exit 1
git config user.name "classify test"
git config user.email "classify-test@example.invalid"
git config commit.gpgsign false

TEMPLATE=crates/worker/wrangler.deploy.toml
MIGDIR=crates/adapters/postgres/migrations

write_template() { # write_template <date> <extra migrations text> <comment inside the first migrations table>
  mkdir -p "$(dirname "$TEMPLATE")"
  cat > "$TEMPLATE" <<EOF
name = "__WORKER_NAME__"
compatibility_date = "$1"

[[durable_objects.bindings]]
name = "ROOM"
class_name = "RoomDo"

[[migrations]]
tag = "v1"            # $3
new_sqlite_classes = ["RoomDo"]
$2

[[ratelimits]]
name = "RL_V1"
EOF
}

commit() { git add -A && git commit -q -m "$1"; git rev-parse HEAD; }

# the history, with the state each commit leaves: the migrations directory (A0 .. A3, B1) and the [[migrations]] tables (v1, v1+v2a,
# v1+v2b, v1+v2c); the main line is 0 .. 8, a side line d1 d2 d3 leaves it after 2, and e1 and e2 are two siblings after 4
LABELS="0 1 2 3 4 5 6 7 8 d1 d2 d3 e1 e2"
DIRSTATE="0:A0 1:A0 2:A1 3:A1 4:A1 5:A2 6:A2 7:A3 8:A3 d1:B1 d2:B1 d3:B1 e1:A1 e2:A1"
LIFESTATE="0:v1 1:v1 2:v1 3:v1 4:v1+v2a 5:v1+v2b 6:v1+v2b 7:v1+v2b 8:v1 d1:v1 d2:v1 d3:v1+v2c e1:v1+v2a e2:v1+v2a"
# the parent of each commit, written out by hand
PARENT="1:0 2:1 3:2 4:3 5:4 6:5 7:6 8:7 d1:2 d2:d1 d3:d2 e1:4 e2:4"
DATE=2026-08-01; EXTRA=""; CMT="first"
write_template "$DATE" "$EXTRA" "$CMT"
echo "CREATE TABLE a (id int);" > "$MIGDIR/0001.sql"
S0="$(commit c0)"
echo "notes" > README.md;                                      S1="$(commit c1-other)"
echo "-- a comment" >> "$MIGDIR/0001.sql";                     S2="$(commit c2-migrations)"
DATE=2026-08-02; write_template "$DATE" "$EXTRA" "$CMT";       S3="$(commit c3-template-outside-migrations)"
EXTRA='
[[migrations]]
tag = "v2"
new_sqlite_classes = ["Other"]';  write_template "$DATE" "$EXTRA" "$CMT"
S4="$(commit c4-lifecycle)"
echo "CREATE TABLE b (id int);" > "$MIGDIR/0002.sql"
EXTRA='
[[migrations]]
tag = "v2"
new_sqlite_classes = ["Other", "More"]'; write_template "$DATE" "$EXTRA" "$CMT"
S5="$(commit c5-both)"
CMT="a different comment, no different meaning"; write_template "$DATE" "$EXTRA" "$CMT"
S6="$(commit c6-comment-in-migrations-table)"
echo "CREATE TABLE c (id int);" > "$MIGDIR/0003.sql";        S7="$(commit c7-migrations)"
EXTRA=""; write_template "$DATE" "$EXTRA" "$CMT";              S8="$(commit c8-lifecycle-removed)"

# a side line off c2: another file under the same number as c5's (a different directory state), then only a README, then a different
# lifecycle change
git checkout -q -b side-d "$S2"
echo "CREATE TABLE b2 (id int);" > "$MIGDIR/0002.sql";         Sd1="$(commit d1-other-0002)"
echo "notes d" > d.txt;                                        Sd2="$(commit d2-other)"
EXTRA='
[[migrations]]
tag = "v2"
new_sqlite_classes = ["Third"]'; write_template 2026-08-01 "$EXTRA" "first"; Sd3="$(commit d3-other-lifecycle)"
# two siblings off c4 that change neither state: what release-plz makes when it force-updates a branch
git checkout -q -b side-e1 "$S4"
echo e1 > e1.txt;                                              Se1="$(commit e1-release-commit)"
git checkout -q -b side-e2 "$S4"
echo e2 > e2.txt;                                              Se2="$(commit e2-release-commit)"
git checkout -q main

dirstate_of() { echo "$DIRSTATE" | tr ' ' '\n' | grep "^$1:" | cut -d: -f2; }
lifestate_of() { echo "$LIFESTATE" | tr ' ' '\n' | grep "^$1:" | cut -d: -f2; }
parent_of() { echo "$PARENT" | tr ' ' '\n' | grep "^$1:" | cut -d: -f2; }
sha_of() { eval "echo \$S$1"; }
# strictly_older <a> <b>: a is an ancestor of b, found by walking the hand-written parents
strictly_older() {
  x="$2"
  while p="$(parent_of "$x")"; [ -n "$p" ]; do
    [ "$p" = "$1" ] && return 0
    x="$p"
  done
  return 1
}

# run <prev> <head>: the script's stdout and its status. RUN_DIR is where it runs: the full repository, or a shallow clone of it
RUN_DIR="$R"
run_script() { ( cd "$RUN_DIR" && sh "$SCRIPT" "$1" "$2" 2> "$T/err" ); }
field() { echo "$1" | grep "^$2=" | cut -d= -f2-; }

check() { # check <label> <got> <want>
  if [ "$2" = "$3" ]; then pass=$((pass + 1)); else failn=$((failn + 1)); echo "BAD  $1: got '$2', wanted '$3'"; fi
}

# ---- every ordered pair of commits: the live one (i) and the one being deployed (j) -----------------------------------------------
n=0
for i in $LABELS; do
  for j in $LABELS; do
    if [ "$i" != "$j" ] && strictly_older "$j" "$i"; then
      want_m=true; want_l=true   # the deploy is older than what is live: a person decides
    else
      want_m=false; want_l=false
      [ "$(dirstate_of "$i")" != "$(dirstate_of "$j")" ] && want_m=true
      [ "$(lifestate_of "$i")" != "$(lifestate_of "$j")" ] && want_l=true
    fi
    out="$(run_script "$(sha_of "$i")" "$(sha_of "$j")")"
    check "live c$i, deploy c$j: migrations_changed" "$(field "$out" migrations_changed)" "$want_m"
    check "live c$i, deploy c$j: do_lifecycle" "$(field "$out" do_lifecycle)" "$want_l"
    n=$((n + 1))
  done
done
echo "ok   all $n ordered pairs of commits classified as the oracle says"

# the pair that matters most, said on its own: two release commits on the same parent, neither the ancestor of the other, differing in
# nothing the pipeline looks at. It must NOT go to the manual gate (that is what stopped the real runs 43796746005 and 43803593395)
out="$(run_script "$(sha_of e1)" "$(sha_of e2)")"
check "sibling release commits: migrations_changed" "$(field "$out" migrations_changed)" false
check "sibling release commits: do_lifecycle" "$(field "$out" do_lifecycle)" false
case "$(field "$out" reason)" in *"by content"*) ;; *) failn=$((failn + 1)); echo "BAD  sibling release commits: the reason '$(field "$out" reason)' does not say it was a comparison of content" ;; esac

# a short sha is what a tag might be too; and a short sha of the SAME commit is the same commit, not an older one
out="$(run_script "$(sha_of 2 | cut -c1-9)" "$(sha_of 5)")"
check "a short previous sha" "$(field "$out" migrations_changed)/$(field "$out" do_lifecycle)" "true/true"
out="$(run_script "$(sha_of 5 | cut -c1-9)" "$(sha_of 5)")"
check "a short previous sha of the same commit" "$(field "$out" migrations_changed)/$(field "$out" do_lifecycle)" "false/false"

# ---- the cases it cannot tell, and must not call "no change" ---------------------------------------------------------------
cannot_tell() { # cannot_tell <label> <prev> <head> <fragment of the reason>
  out="$(run_script "$2" "$3")"
  check "$1: migrations_changed" "$(field "$out" migrations_changed)" true
  check "$1: do_lifecycle" "$(field "$out" do_lifecycle)" true
  case "$(field "$out" reason)" in *"$4"*) ;; *) failn=$((failn + 1)); echo "BAD  $1: the reason '$(field "$out" reason)' lacks '$4'" ;; esac
}
cannot_tell "no active version" none "$(sha_of 8)" "first deploy"
cannot_tell "a tag that is not a sha" v1 "$(sha_of 8)" "not a commit sha"
cannot_tell "a sha that is not in this history" deadbeefdeadbeefdeadbeefdeadbeefdeadbeef "$(sha_of 8)" "not a commit in this checkout"
cannot_tell "a deploy of older code" "$(sha_of 8)" "$(sha_of 5)" "older than the active version"
cannot_tell "older code on another line of history" "$(sha_of d3)" "$(sha_of d1)" "older than the active version"

# a shallow checkout cannot say which of two commits is older, so it says nothing: the same pair that a full checkout calls "no change"
# (c0 then c1, which change neither state) goes to the gate
git config uploadpack.allowAnySHA1InWant true
git clone -q --depth 1 "file://$R" "$T/shallow" > /dev/null 2>&1
( cd "$T/shallow" && git fetch -q --depth 1 origin "$(sha_of 0)" "$(sha_of 1)" ) > /dev/null 2>&1
out="$(run_script "$(sha_of 0)" "$(sha_of 1)")"
check "control for the shallow case: the full checkout says c0 then c1 changes nothing" "$(field "$out" migrations_changed)/$(field "$out" do_lifecycle)" "false/false"
RUN_DIR="$T/shallow"
check "the shallow clone is shallow" "$(cd "$T/shallow" && git rev-parse --is-shallow-repository)" true
cannot_tell "a shallow checkout" "$(sha_of 0)" "$(sha_of 1)" "shallow"
RUN_DIR="$R"

# the template did not exist at the previous commit
git checkout -q --orphan notemplate
git rm -rfq . > /dev/null 2>&1
echo x > x.txt
NOTEMPLATE="$(commit no-template)"
git checkout -q main
# make a descendant of NOTEMPLATE that has the template
git checkout -q -b descendant "$NOTEMPLATE"
write_template 2026-08-01 "" "first"
mkdir -p "$MIGDIR"; echo "CREATE TABLE a (id int);" > "$MIGDIR/0001.sql"
DESC="$(commit with-template)"
git checkout -q main
cannot_tell "a template that did not exist at the previous commit" "$NOTEMPLATE" "$DESC" "did not exist"

# the head has no template: a usage error, not a classification
run_script "$(sha_of 1)" "$NOTEMPLATE" > /dev/null; rc=$?
check "a head without a template is a usage error" "$rc" 2
grep -q "does not exist" "$T/err" && pass=$((pass + 1)) || { failn=$((failn + 1)); echo "BAD  the usage error does not say the template is missing"; }

run_script "" "$(sha_of 8)" > /dev/null; rc=$?
check "an empty previous tag is a usage error" "$rc" 2
run_script none > /dev/null; rc=$?
check "a missing head is a usage error" "$rc" 2
run_script none notasha > /dev/null; rc=$?
check "a head that is not a commit is a usage error" "$rc" 2

# the same commit twice: nothing changes, and it says so
out="$(run_script "$(sha_of 5)" "$(sha_of 5)")"
check "the same commit: migrations_changed" "$(field "$out" migrations_changed)" false
check "the same commit: do_lifecycle" "$(field "$out" do_lifecycle)" false

echo "passed=$pass bad=$failn"
[ "$failn" = 0 ]
