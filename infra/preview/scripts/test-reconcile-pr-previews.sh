#!/usr/bin/env bash
# Hermetic controls for the per-PR preview desired set and reconciler.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DESIRED="$HERE/pr-preview-desired.py"
RECONCILE="$HERE/reconcile-pr-previews.sh"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
fail=0
ok() { echo "  PASS  $1"; }
bad() { echo "  FAIL  $1"; fail=1; }

cat >"$T/pulls.json" <<'JSON'
[
 {"number": 9, "created_at": "2026-11-15T00:00:00Z", "updated_at": "2026-11-15T00:00:00Z", "head": {"repo": {"full_name": "aRustyDev/thrust"}}},
 {"number": 8, "created_at": "2026-10-01T00:00:00Z", "updated_at": "2026-10-01T00:00:00Z", "head": {"repo": {"full_name": "aRustyDev/thrust"}}},
 {"number": 7, "created_at": "2026-11-15T00:00:00Z", "updated_at": "2026-11-15T00:00:00Z", "head": {"repo": {"full_name": "fork/thrust"}}}
]
JSON
cat >"$T/events.json" <<'JSON'
[{"type":"PullRequestEvent","created_at":"2026-11-14T00:00:00Z","payload":{"action":"synchronize","number":8}}]
JSON
got="$(python3 "$DESIRED" --repository aRustyDev/thrust --pulls "$T/pulls.json" --events "$T/events.json" --now 2026-11-16T00:00:00Z)"
[[ "$got" == $'9\n8' ]] && ok "desired set keeps recent same-repository pushes and rejects forks" || bad "desired set was '$got'"
printf 'not-json' >"$T/events.bad"
if python3 "$DESIRED" --repository aRustyDev/thrust --pulls "$T/pulls.json" --events "$T/events.bad" --now 2026-11-16T00:00:00Z >/dev/null 2>&1; then bad "control: malformed GitHub inventory passed"; else ok "control: malformed GitHub inventory fails closed"; fi
printf '%s\n' '[{"number":11,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-11-15T00:00:00Z","head":{"repo":{"full_name":"aRustyDev/thrust"}}}]' >"$T/recent-update.json"
printf '%s\n' '[]' >"$T/empty-events.json"
got="$(python3 "$DESIRED" --repository aRustyDev/thrust --pulls "$T/recent-update.json" --events "$T/empty-events.json" --now 2026-11-16T00:00:00Z)"
[[ "$got" == 11 ]] && ok 'recent PR update prevents premature deletion if the events feed is truncated' || bad 'recent update ignored'
printf '%s\n' '[{"number":11,"created_at":"2026-01-01T00:00:00Z","head":{"repo":{"full_name":"aRustyDev/thrust"}}}]' >"$T/missing-update.json"
if python3 "$DESIRED" --repository aRustyDev/thrust --pulls "$T/missing-update.json" --events "$T/empty-events.json" --now 2026-11-16T00:00:00Z >/dev/null 2>&1; then bad 'missing update timestamp accepted'; else ok 'missing update timestamp fails closed'; fi

cat >"$T/schema" <<'SH'
#!/bin/sh
if [ "$1" = list ]; then
  if [ -n "${SCHEMA_INVENTORY:-}" ]; then printf '%s\n' "$SCHEMA_INVENTORY"; else printf '%s\n' '[{"number":8,"schema":true,"login_role":true,"search_path":true,"pending":false},{"number":3,"schema":true,"login_role":true,"search_path":true,"pending":false}]'; fi
else echo "schema $*" >>"$CALLS"; fi
SH
cat >"$T/hyperdrive" <<'SH'
#!/bin/sh
if [ "$1" = list-pr ]; then printf '%s\n' '[{"number":8,"id":"h8","name":"thrust-pr-8"},{"number":4,"id":"h4","name":"thrust-pr-4"}]'; else
  echo "hyperdrive $*" >>"$CALLS"
  if [ "$1" != verify-pr ]; then [ "${FAIL_HYPERDRIVE:-0}" != 1 ] || exit 1; fi
  if [ "$1" = verify-pr ]; then
    [ "${FAIL_VERIFY:-0}" != 1 ] || exit 1
    count=$(grep -c '^hyperdrive verify-pr ' "$CALLS")
    [ "$count" != "${FAIL_VERIFY_AT:-0}" ] || exit 1
  fi
fi
SH
chmod +x "$T/schema" "$T/hyperdrive"
printf '8\n9\n' >"$T/desired"
CALLS="$T/calls" PR_SCHEMA_SH="$T/schema" HYPERDRIVE_SH="$T/hyperdrive" "$RECONCILE" --desired "$T/desired" --dry-run >"$T/out"
grep -qx 'ensure pr_9' "$T/out" && grep -qx 'drop pr_4' "$T/out" && ! grep -q 'pr_8' "$T/out" && ok "reconciler creates only incomplete desired resources and drops only undesired resources" || bad "reconciler actions"
printf '8\n' >"$T/pending-desired"
inventory='[{"number":8,"schema":true,"login_role":true,"search_path":true,"pending":true}]'
CALLS="$T/calls" SCHEMA_INVENTORY="$inventory" PR_SCHEMA_SH="$T/schema" HYPERDRIVE_SH="$T/hyperdrive" "$RECONCILE" --desired "$T/pending-desired" --dry-run >"$T/out"
grep -qx 'ensure pr_8' "$T/out" && ok 'pending password synchronization is retried' || bad 'pending retry'
: >"$T/calls"
if CALLS="$T/calls" FAIL_VERIFY=1 PR_SCHEMA_SH="$T/schema" HYPERDRIVE_SH="$T/hyperdrive" "$RECONCILE" --desired "$T/desired" >/dev/null 2>&1; then bad 'drifted existing Hyperdrive passed'; else
  if grep -qE 'schema (create|drop|acknowledge)|hyperdrive (ensure|rotate-password|drop-pr)' "$T/calls"; then bad 'drift caused a mutation'; else ok 'drift refuses reconciliation before any mutation'; fi
fi
: >"$T/calls"
if CALLS="$T/calls" SCHEMA_INVENTORY="$inventory" FAIL_VERIFY_AT=2 PR_SCHEMA_SH="$T/schema" HYPERDRIVE_SH="$T/hyperdrive" "$RECONCILE" --desired "$T/pending-desired" >/dev/null 2>&1; then bad 'post-update drift passed'; else
  if grep -q 'schema acknowledge 8' "$T/calls"; then bad 'post-update drift acknowledged'; else ok 'post-update drift leaves password synchronization pending'; fi
fi
: >"$T/calls"
if CALLS="$T/calls" SCHEMA_INVENTORY="$inventory" FAIL_HYPERDRIVE=1 PR_SCHEMA_SH="$T/schema" HYPERDRIVE_SH="$T/hyperdrive" "$RECONCILE" --desired "$T/pending-desired" >/dev/null 2>&1; then bad 'failed Hyperdrive update passed'; else
  if grep -q 'schema acknowledge 8' "$T/calls"; then bad 'failure acknowledged'; else ok 'failed Hyperdrive update is not acknowledged'; fi
fi
printf '8\n8\n' >"$T/duplicate"
if CALLS="$T/calls" PR_SCHEMA_SH="$T/schema" HYPERDRIVE_SH="$T/hyperdrive" "$RECONCILE" --desired "$T/duplicate" --dry-run >/dev/null 2>&1; then bad "control: duplicate desired inventory passed"; else ok "control: malformed desired inventory fails before mutation"; fi

python3 -I "$HERE/test_migration_artifact.py"
python3 -I "$HERE/test_migration_authorization.py"
python3 -I "$HERE/test_verify_pr_head.py"
if ((fail)); then exit 1; fi
echo "preview reconciliation tests: PASS"
