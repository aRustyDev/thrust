#!/usr/bin/env bash
# Reconcile the database and Hyperdrive resources for the explicit desired PR set.
# GitHub discovery happens before this script and writes --desired. A missing or malformed
# inventory is an error, never an instruction to delete all previews.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCHEMA="${PR_SCHEMA_SH:-$HERE/provisioner-client.sh}"
HYPERDRIVE="${HYPERDRIVE_SH:-$HERE/../../cloudflare/hyperdrive.sh}"
MAX_PREVIEWS=50
desired_file=""
dry=0

usage() { echo "usage: reconcile-pr-previews.sh --desired <newline-delimited-pr-numbers> [--dry-run]" >&2; exit 2; }
while [[ $# -gt 0 ]]; do
  case "$1" in
    --desired) desired_file="${2:-}"; shift ;;
    --dry-run) dry=1 ;;
    *) usage ;;
  esac
  shift
done
[[ -n "$desired_file" && -f "$desired_file" ]] || { echo "reconcile-pr-previews: --desired must name a file" >&2; exit 3; }

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
python3 - "$desired_file" "$MAX_PREVIEWS" >"$tmp/desired" <<'PY'
import sys
values = []
for line in open(sys.argv[1]):
    value = line.strip()
    if not value:
        continue
    if not value.isdecimal() or not 1 <= len(value) <= 9:
        raise SystemExit("reconcile-pr-previews: desired inventory has an invalid PR number")
    values.append(int(value))
if len(values) != len(set(values)):
    raise SystemExit("reconcile-pr-previews: desired inventory has duplicate PR numbers")
if len(values) > int(sys.argv[2]):
    raise SystemExit("reconcile-pr-previews: desired inventory exceeds the preview cap")
print("\n".join(map(str, sorted(values))))
PY

"$SCHEMA" list --json >"$tmp/schemas.json"
"$HYPERDRIVE" list-pr >"$tmp/hyperdrive.json"
python3 - "$tmp/desired" "$tmp/schemas.json" "$tmp/hyperdrive.json" >"$tmp/actions" <<'PY'
import json, sys
desired = {int(x) for x in open(sys.argv[1]) if x.strip()}
schemas = {int(x["number"]): x for x in json.load(open(sys.argv[2]))}
hyperdrive = {int(x["number"]): x for x in json.load(open(sys.argv[3]))}
# Check every existing desired origin before any database or Cloudflare mutation.
for number in sorted(desired & set(hyperdrive)):
    print("verify", number)
for number in sorted(desired):
    s = schemas.get(number, {})
    if not (s.get("schema") and s.get("login_role") and s.get("search_path")) or s.get("pending", True) or number not in hyperdrive:
        print("ensure", number)
for number in sorted((set(schemas) | set(hyperdrive)) - desired, reverse=True):
    print("drop", number)
PY

while read -r action number; do
  [[ -n "${action:-}" ]] || continue
  if [[ "$action" == verify ]]; then
    "$HYPERDRIVE" verify-pr "$number" >/dev/null
    continue
  fi
  if [[ "$dry" -eq 1 ]]; then echo "$action pr_$number"; continue; fi
  if [[ "$action" == ensure ]]; then
    password="$tmp/pr-$number.password"
    "$SCHEMA" create "$number" --password-file "$password"
    if "$HYPERDRIVE" list-pr | python3 -c 'import json,sys; n=int(sys.argv[1]); raise SystemExit(0 if any(x["number"] == n for x in json.load(sys.stdin)) else 1)' "$number"; then
      "$HYPERDRIVE" rotate-password "thrust-pr-$number" --password-file "$password"
    else
      "$HYPERDRIVE" ensure "thrust-pr-$number" --vpc-service thrust-pg-dev --user "pr_$number" --password-file "$password"
    fi
    # Do not acknowledge password synchronization until the resulting config still matches.
    "$HYPERDRIVE" verify-pr "$number" >/dev/null
    "$SCHEMA" acknowledge "$number"
    rm -f "$password"
  else
    "$HYPERDRIVE" drop-pr "$number"
    "$SCHEMA" drop "$number"
  fi
done <"$tmp/actions"

"$HYPERDRIVE" list-pr
