#!/bin/sh
# The expand rule of the migrations (issue #195): a new migration may only ADD to the schema, unless it says it takes something away.
#
#   scripts/check-migration-lint.sh [<migrations directory>]     (default: crates/thrust-adapters-postgres/migrations)
#
# Why. Once more than one version of the code runs over the same data (a release candidate, an older release kept beside the new one),
# the schema has to serve all of them, so a migration that drops, renames or tightens something breaks the older ones while they still
# run. Such a migration has to be a contract migration, named `NNNN_contract_<what>.sql`: `migrate --no-contract` refuses it, and the
# deploy runs it only in production and only when no older shared target is live (thrust_adapters_postgres::ledger).
#
# What it refuses, in a file numbered above 0017 that is not a contract file, case-insensitively, after SQL comments (`--` to the end of
# the line, `/* */`) and single-quoted strings are taken out, statement by statement (split at `;`):
#   * DROP, RENAME, as words (`dropped_at` is not one);
#   * ALTER [COLUMN] <column> [SET DATA] TYPE: a column's type changed (`CREATE TYPE` and `ALTER TYPE <enum> ADD VALUE` are not that);
#   * SET NOT NULL.
# Files 0001 to 0017 were written before the rule and are what every database already has; they are not read.
#
# Exit status: 0 nothing refused; 1 a file was refused (each statement is named); 2 called wrongly, or no file was read at all.
set -u
LC_ALL=C
export LC_ALL

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIR="${1:-$ROOT/crates/thrust-adapters-postgres/migrations}"
[ $# -le 1 ] || { echo "check-migration-lint: usage: check-migration-lint.sh [<migrations directory>]" >&2; exit 2; }
[ -d "$DIR" ] || { echo "check-migration-lint: $DIR is not a directory" >&2; exit 2; }

# One statement per line: comments and single-quoted strings out, lowercase, whitespace made single spaces, split at `;`.
statements() {
  awk '
    BEGIN { mode = "code"; out = "" }
    {
      line = $0 "\n"; n = length(line); i = 1
      while (i <= n) {
        c = substr(line, i, 1); d = substr(line, i, 2)
        if (mode == "code") {
          if (d == "--") { mode = "line"; i += 2; continue }
          if (d == "/*") { mode = "block"; i += 2; continue }
          if (c == "\047") { mode = "string"; out = out " "; i++; continue }
          out = out c; i++; continue
        }
        if (mode == "line") { if (c == "\n") { mode = "code"; out = out " " } i++; continue }
        if (mode == "block") { if (d == "*/") { mode = "code"; out = out " "; i += 2; continue } i++; continue }
        if (mode == "string") {
          if (d == "\047\047") { i += 2; continue }
          if (c == "\047") { mode = "code" }
          i++; continue
        }
      }
    }
    END {
      out = tolower(out)
      gsub(/[ \t\r\n]+/, " ", out)
      k = split(out, parts, ";")
      for (j = 1; j <= k; j++) { s = parts[j]; sub(/^ /, "", s); sub(/ $/, "", s); if (s != "") print s }
    }
  ' "$1"
}

T="$(mktemp -d)" || { echo "check-migration-lint: cannot make a temporary directory" >&2; exit 2; }
trap 'rm -rf "$T"' EXIT INT TERM
W='[^a-z0-9_]'
read_files=0
refused=0
for f in "$DIR"/*.sql; do
  [ -f "$f" ] || continue
  read_files=$((read_files + 1))
  name="$(basename "$f")"
  number="$(printf '%s' "$name" | cut -c1-4)"
  case "$number" in [0-9][0-9][0-9][0-9]) ;; *) echo "check-migration-lint: $name does not start with a four-digit number"; refused=1; continue ;; esac
  [ "$number" -le 17 ] && continue
  case "$name" in [0-9][0-9][0-9][0-9]_contract_?*.sql) continue ;; esac
  statements "$f" > "$T/statements" || { echo "check-migration-lint: cannot read $name" >&2; exit 2; }
  while IFS= read -r stmt; do
    why=""
    printf ' %s ' "$stmt" | grep -qE "${W}drop${W}" && why="DROP"
    printf ' %s ' "$stmt" | grep -qE "${W}rename${W}" && why="${why:+$why, }RENAME"
    printf ' %s ' "$stmt" | grep -qE "${W}alter( column)? (\"[^\"]+\"|[a-z0-9_]+)( set data)? type${W}" && why="${why:+$why, }ALTER ... TYPE"
    printf ' %s ' "$stmt" | grep -qE "${W}set not null${W}" && why="${why:+$why, }SET NOT NULL"
    if [ -n "$why" ]; then
      echo "check-migration-lint: $name: $why in: $stmt"
      refused=1
    fi
  done < "$T/statements"
done

[ "$read_files" -gt 0 ] || { echo "check-migration-lint: no .sql file in $DIR: nothing was checked" >&2; exit 2; }
if [ "$refused" -ne 0 ]; then
  echo "check-migration-lint: FAIL: a migration that takes something away must be a contract migration (NNNN_contract_<what>.sql)"
  exit 1
fi
echo "check-migration-lint: ok ($read_files files, those above 0017 that are not contract migrations only add)"
