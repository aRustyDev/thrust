#!/bin/sh
# .github/scripts/classify-deploy.sh against a real git history (issue #49): every pair of commits in a made-up history, with the answer
# worked out here, independently, from which commits touched which kind of file; then the cases it must treat as "cannot tell".
#
# The oracle is not the script: each commit of the made-up history is labelled by hand with the STATE of the migrations directory and of
# the `[[migrations]]` tables it leaves behind, and the expected answer for the pair (i, j) is "the two labels differ". The history has
# commits that change neither, one that only edits a comment inside a `[[migrations]]` table (not a lifecycle change), ones that do
# both, and one that undoes an earlier lifecycle change (the net state is the old one, so nothing is different), so a classifier that
# compared the wrong thing disagrees somewhere.
#
#   sh .github/scripts/tests/test-classify-deploy.sh
#
# CLASSIFY_SCRIPT points the test at a copy of the script, so a deliberately broken copy can be shown to fail it.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
SCRIPT="${CLASSIFY_SCRIPT:-$HERE/.github/scripts/classify-deploy.sh}"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
pass=0; failn=0

# no user or system git configuration: this repository must not sign commits through a password manager, or ask for an identity
GIT_CONFIG_GLOBAL=/dev/null; GIT_CONFIG_SYSTEM=/dev/null
export GIT_CONFIG_GLOBAL GIT_CONFIG_SYSTEM

R="$T/repo"
mkdir -p "$R/crates/thrust-worker" "$R/crates/thrust-adapters-postgres/migrations"
cd "$R" || exit 1
git init -q -b main . || exit 1
git config user.name "classify test"
git config user.email "classify-test@example.invalid"
git config commit.gpgsign false

TEMPLATE=crates/thrust-worker/wrangler.deploy.toml
MIGDIR=crates/thrust-adapters-postgres/migrations

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

# the history, with the state each commit leaves: the migrations directory (A0 .. A3) and the [[migrations]] tables (v1, v1+v2a, v1+v2b)
DIRSTATE="0:A0 1:A0 2:A1 3:A1 4:A1 5:A2 6:A2 7:A3 8:A3"
LIFESTATE="0:v1 1:v1 2:v1 3:v1 4:v1+v2a 5:v1+v2b 6:v1+v2b 7:v1+v2b 8:v1"
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

dirstate_of() { echo "$DIRSTATE" | tr ' ' '\n' | grep "^$1:" | cut -d: -f2; }
lifestate_of() { echo "$LIFESTATE" | tr ' ' '\n' | grep "^$1:" | cut -d: -f2; }
sha_of() { eval "echo \$S$1"; }

# run <prev> <head>: the script's stdout and its status
run_script() { sh "$SCRIPT" "$1" "$2" 2> "$T/err"; }
field() { echo "$1" | grep "^$2=" | cut -d= -f2-; }

check() { # check <label> <got> <want>
  if [ "$2" = "$3" ]; then pass=$((pass + 1)); else failn=$((failn + 1)); echo "BAD  $1: got '$2', wanted '$3'"; fi
}

# ---- every pair of commits ---------------------------------------------------------------------------------------------------
i=0
while [ "$i" -le 8 ]; do
  j=$i
  while [ "$j" -le 8 ]; do
    want_m=false; want_l=false
    [ "$(dirstate_of "$i")" != "$(dirstate_of "$j")" ] && want_m=true
    [ "$(lifestate_of "$i")" != "$(lifestate_of "$j")" ] && want_l=true
    out="$(run_script "$(sha_of "$i")" "$(sha_of "$j")")"
    check "c$i..c$j migrations_changed" "$(field "$out" migrations_changed)" "$want_m"
    check "c$i..c$j do_lifecycle" "$(field "$out" do_lifecycle)" "$want_l"
    j=$((j + 1))
  done
  i=$((i + 1))
done
echo "ok   all 45 pairs of commits classified as the oracle says"

# a short sha is what a tag might be too
out="$(run_script "$(sha_of 2 | cut -c1-9)" "$(sha_of 5)")"
check "a short previous sha" "$(field "$out" migrations_changed)/$(field "$out" do_lifecycle)" "true/true"

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

# a commit on another line of history: not an ancestor of the head
git checkout -q -b side "$(sha_of 2)"
echo "side" > side.txt
SIDE="$(commit side-commit)"
git checkout -q main
cannot_tell "a commit that is not an ancestor of the head" "$SIDE" "$(sha_of 8)" "not an ancestor"

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
