#!/bin/sh
# Exercise scripts/check-migration-lint.sh (issue #195), the expand rule of the migrations, including the cases that must FAIL.
#
# Exhaustive over a small matrix: each construct that takes something away, in upper and lower case, written plainly, in a `--`
# comment, in a `/* */` comment and in a string, in a file numbered 0017 (before the rule), 0018 and 0018 as a contract migration. It
# must be refused exactly when it is written plainly in 0018 that is not a contract migration. Then constructs that only add, which must
# pass wherever they are; the real migrations, which must pass; and the calls that check nothing, which must not pass.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
# LINT points the test at a copy of the script, so that a broken copy can be shown to fail it without touching the real one.
LINT="${LINT:-$HERE/scripts/check-migration-lint.sh}"
# LINT_SHELL runs it under another shell than `sh` (on macOS `sh` is bash; CI's is dash: `LINT_SHELL=dash` runs it as CI does).
RUN="${LINT_SHELL:-sh}"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
pass=0; failn=0
ok() { pass=$((pass + 1)); }
bad() { failn=$((failn + 1)); echo "BAD  $*"; }

# lint_one <file name> <sql>: the lint over a directory holding that one file. Sets lint_rc and lint_out.
lint_one() {
  rm -rf "$T/d"; mkdir -p "$T/d"
  printf '%s\n' "$2" > "$T/d/$1"
  lint_out="$("$RUN" "$LINT" "$T/d" 2>&1)"
  lint_rc=$?
}

# ---- what takes something away ------------------------------------------------------------------------------------------------
# One per line: a construct the rule refuses, and the word the refusal must name.
cat > "$T/forbidden" <<'EOF'
DROP TABLE work_items_old|DROP
ALTER TABLE labels DROP COLUMN colour|DROP
ALTER TABLE labels RENAME COLUMN colour TO color|RENAME
ALTER TABLE labels RENAME TO tags|RENAME
ALTER TABLE labels ALTER COLUMN sort_order TYPE bigint|ALTER ... TYPE
ALTER TABLE labels ALTER sort_order SET DATA TYPE bigint|ALTER ... TYPE
ALTER TABLE labels ALTER COLUMN name SET NOT NULL|SET NOT NULL
DO $$ BEGIN DROP INDEX IF EXISTS labels_name; END $$|DROP
EOF

matrix_cases=0
while IFS='|' read -r construct reason; do
  for casing in upper lower; do
    sql="$construct"
    [ "$casing" = lower ] && sql="$(printf '%s' "$construct" | tr '[:upper:]' '[:lower:]')"
    for wrap in plain line block string; do
      case "$wrap" in
        plain) body="$sql;" ;;
        line) body="-- $sql;" ;;
        block) body="/* $sql; */" ;;
        string) body="COMMENT ON TABLE labels IS '$sql';" ;;
      esac
      for file in 0017_x.sql 0018_x.sql 0018_contract_x.sql; do
        matrix_cases=$((matrix_cases + 1))
        lint_one "$file" "CREATE TABLE IF NOT EXISTS keep (n int);
$body
CREATE INDEX IF NOT EXISTS keep_n ON keep (n);"
        if [ "$wrap" = plain ] && [ "$file" = 0018_x.sql ]; then
          if [ "$lint_rc" = 1 ] && printf '%s' "$lint_out" | grep -qF "$file: $reason"; then ok
          else bad "refused: '$body' in $file must be refused naming $reason (exit $lint_rc): $lint_out"; fi
        else
          if [ "$lint_rc" = 0 ]; then ok
          else bad "passed: '$body' in $file must pass (exit $lint_rc): $lint_out"; fi
        fi
      done
    done
  done
done < "$T/forbidden"
echo "ok   the matrix: $matrix_cases cases (8 constructs x 2 casings x 4 places x 3 files)"

# ---- what only adds: passes in any file ----------------------------------------------------------------------------------------
while IFS= read -r construct; do
  for file in 0018_x.sql 0018_contract_x.sql; do
    lint_one "$file" "$construct;"
    if [ "$lint_rc" = 0 ]; then ok; else bad "passed: '$construct' in $file must pass (exit $lint_rc): $lint_out"; fi
  done
done <<'EOF'
ALTER TABLE labels ADD COLUMN IF NOT EXISTS dropped_at timestamptz
ALTER TABLE labels ADD COLUMN IF NOT EXISTS renamed_from text
ALTER TABLE labels ADD COLUMN IF NOT EXISTS type text
CREATE TYPE mood AS ENUM ('sad', 'ok')
ALTER TYPE mood ADD VALUE IF NOT EXISTS 'happy'
CREATE INDEX IF NOT EXISTS labels_drop_idx ON labels (name)
ALTER TABLE labels ALTER COLUMN name SET DEFAULT ''
EOF
# DROP NOT NULL loosens a column, which an older version can live with, and the rule still refuses it: DROP is a word of it. Pinned here
# (an example the rule names: every DROP) so that allowing it would be a decision and not a drift.
lint_one 0018_x.sql "ALTER TABLE labels ALTER COLUMN name DROP NOT NULL;"
if [ "$lint_rc" = 1 ] && printf '%s' "$lint_out" | grep -qF "0018_x.sql: DROP"; then ok; else bad "DROP NOT NULL is refused today: $lint_out"; fi

# ---- statements that span lines, and two in one line -------------------------------------------------------------------------
lint_one 0018_x.sql "ALTER TABLE labels
  ALTER COLUMN name
  SET NOT NULL;"
if [ "$lint_rc" = 1 ] && printf '%s' "$lint_out" | grep -qF "SET NOT NULL"; then ok; else bad "a statement over three lines must be refused: $lint_out"; fi
lint_one 0018_x.sql "CREATE TABLE IF NOT EXISTS a (n int); DROP TABLE b;"
if [ "$lint_rc" = 1 ] && printf '%s' "$lint_out" | grep -qF "DROP in: drop table b"; then ok; else bad "the second statement of a line must be refused by itself: $lint_out"; fi
lint_one 0018_x.sql "COMMENT ON TABLE labels IS 'it''s safe to drop later'; CREATE INDEX IF NOT EXISTS i ON labels (name);"
if [ "$lint_rc" = 0 ]; then ok; else bad "a string with a quote in it ends where it ends: $lint_out"; fi

# ---- the real migrations -------------------------------------------------------------------------------------------------------
real_out="$("$RUN" "$LINT" 2>&1)"; real_rc=$?
if [ "$real_rc" = 0 ] && printf '%s' "$real_out" | grep -qE 'ok \([0-9]+ files'; then ok; else bad "the real migrations must pass: $real_out"; fi

# ---- calls that check nothing do not pass --------------------------------------------------------------------------------------
mkdir -p "$T/empty"
out="$("$RUN" "$LINT" "$T/empty" 2>&1)"; rc=$?
if [ "$rc" = 2 ] && printf '%s' "$out" | grep -qF "nothing was checked"; then ok; else bad "an empty directory must be exit 2, not a pass ($rc): $out"; fi
out="$("$RUN" "$LINT" "$T/no-such-dir" 2>&1)"; rc=$?
if [ "$rc" = 2 ]; then ok; else bad "a directory that is not there must be exit 2 ($rc): $out"; fi
lint_one 018_x.sql "CREATE TABLE IF NOT EXISTS keep (n int);"
if [ "$lint_rc" = 1 ] && printf '%s' "$lint_out" | grep -qF "four-digit number"; then ok; else bad "a file without a four-digit number must be refused: $lint_out"; fi

echo "passed=$pass bad=$failn"
[ "$failn" -eq 0 ]
