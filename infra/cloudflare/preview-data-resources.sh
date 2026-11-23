#!/usr/bin/env bash
# Reconcile the non-database data resources for an isolated Thrust Preview.
#
# Commands:
#   list-pr
#       Print every resource matching the reserved thrust-pr-<number> names.  This is read-only.
#   ensure-pr <number>
#       Create the exact R2 bucket and Queue for a PR, or verify that the complete pair exists.
#       A partial pair is deliberately an operator error: it may pre-date this controller.
#   verify-pr <number>
#       Read-only check that the exact bucket and queue pair exists exactly once.
#   drop-pr <number>
#       Delete a detached Queue and an empty bucket.  If the bucket is non-empty, install only
#       this controller's one-day expiry rule and report a pending cleanup.  A later invocation
#       deletes the bucket after R2 has emptied it.  It never replaces an unknown lifecycle rule.
#
# The API has no immutable owner field for R2 buckets or Queues.  Before enabling this controller,
# an operator must inventory and remove/migrate every resource under the `thrust-pr-` prefix.  The
# prefix is then reserved exclusively for this controller; `ensure-pr` refuses a partial pair so
# it never silently adopts one.  This script is intentionally not wired to Actions yet.
#
# Environment: CLOUDFLARE_ACCOUNT_ID and PREVIEW_DATA_API_TOKEN (or CLOUDFLARE_API_TOKEN).
# The token needs Workers R2 Storage Write and Queues Write.  It never needs Hyperdrive or Worker
# deployment permission.  Set CURL to a test double only in hermetic tests.
set -euo pipefail

API="${CLOUDFLARE_API_BASE:-https://api.cloudflare.com/client/v4}"
CURL="${CURL:-curl}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

die() { echo "preview-data-resources: $*" >&2; exit "${2:-2}"; }

need_env() {
  [[ "${CLOUDFLARE_ACCOUNT_ID:-}" =~ ^[0-9a-f]{32}$ ]] || die "CLOUDFLARE_ACCOUNT_ID must be 32 lowercase hex digits" 3
  TOKEN="${PREVIEW_DATA_API_TOKEN:-${CLOUDFLARE_API_TOKEN:-}}"
  [[ -n "$TOKEN" ]] || die "neither PREVIEW_DATA_API_TOKEN nor CLOUDFLARE_API_TOKEN is set" 3
  (umask 077 && printf 'Authorization: Bearer %s\n' "$TOKEN" >"$tmp/hdr")
}

valid_number() { [[ "$1" =~ ^[1-9][0-9]{0,8}$ ]]; }
bucket_name() { printf 'thrust-pr-%s-blobs' "$1"; }
queue_name() { printf 'thrust-pr-%s-events' "$1"; }
cleanup_rule() { printf 'thrust-preview-cleanup-pr-%s' "$1"; }

# Prints an API response body.  Keeping the bearer token in a 0600 header file and request data on
# stdin ensures neither is exposed in process arguments or a shell trace.
api() { # method path [JSON body]
  local method="$1" path="$2"
  if [[ -n "${3:-}" ]]; then
    printf '%s' "$3" | "$CURL" -sS --max-time 30 -X "$method" -H "@$tmp/hdr" \
      -H 'Content-Type: application/json' --data-binary @- "$API$path"
  else
    "$CURL" -sS --max-time 30 -X "$method" -H "@$tmp/hdr" "$API$path"
  fi
}

require_success() { # file label
  python3 - "$1" "$2" <<'PY'
import json, sys
try:
    value = json.load(open(sys.argv[1]))
    if value.get("success") is not True:
        raise ValueError("; ".join(str(e.get("message", "unknown error")) for e in value.get("errors", [])))
except (OSError, ValueError, TypeError, json.JSONDecodeError) as exc:
    raise SystemExit(f"preview-data-resources: {sys.argv[2]} failed or returned invalid JSON: {exc}")
PY
}

list_buckets() {
  local cursor="" url
  : >"$tmp/buckets.jsonl"
  while :; do
    # The API supports name_contains; never sweep unrelated account buckets merely to reconcile
    # Preview names.
    url="/accounts/$CLOUDFLARE_ACCOUNT_ID/r2/buckets?per_page=1000&name_contains=thrust-pr-"
    if [[ -n "$cursor" ]]; then
      cursor="$(CURSOR="$cursor" python3 -c 'import os, urllib.parse; print(urllib.parse.quote(os.environ["CURSOR"], safe=""))')"
      url+="&cursor=$cursor"
    fi
    api GET "$url" >"$tmp/response.json" || die "Cloudflare API unreachable while listing R2 buckets" 3
    require_success "$tmp/response.json" "R2 bucket list" || exit 3
    python3 - "$tmp/response.json" <<'PY' >>"$tmp/buckets.jsonl"
import json, sys
d = json.load(open(sys.argv[1]))
buckets = d.get("result", {}).get("buckets")
if not isinstance(buckets, list) or any(not isinstance(x, dict) or not isinstance(x.get("name"), str) for x in buckets):
    raise SystemExit("preview-data-resources: invalid R2 bucket list shape")
for bucket in buckets: print(json.dumps(bucket, sort_keys=True))
PY
    cursor="$(python3 - "$tmp/response.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1])); value = d.get("result_info", {}).get("cursor")
if value is not None and (not isinstance(value, str) or not value): raise SystemExit("preview-data-resources: invalid R2 bucket cursor")
print(value or "")
PY
)" || exit 3
    [[ -z "$cursor" ]] && break
  done
}

list_queues() {
  local page=1 pages
  : >"$tmp/queues.jsonl"
  while :; do
    api GET "/accounts/$CLOUDFLARE_ACCOUNT_ID/queues?per_page=100&page=$page" >"$tmp/response.json" || die "Cloudflare API unreachable while listing Queues" 3
    require_success "$tmp/response.json" "Queue list" || exit 3
    python3 - "$tmp/response.json" "$page" <<'PY' >"$tmp/queue-page.jsonl"
import json, sys
d = json.load(open(sys.argv[1])); page = int(sys.argv[2])
items, info = d.get("result"), d.get("result_info")
if not isinstance(items, list) or not isinstance(info, dict): raise SystemExit("preview-data-resources: invalid Queue list shape")
if info.get("page") != page or not isinstance(info.get("total_pages"), int) or info["total_pages"] < page: raise SystemExit("preview-data-resources: invalid Queue pagination")
for item in items:
    if (not isinstance(item, dict) or not isinstance(item.get("queue_name"), str) or not isinstance(item.get("queue_id"), str)
            or (item.get("consumers_total_count") is not None and (type(item["consumers_total_count"]) is not int or item["consumers_total_count"] < 0))
            or (item.get("producers_total_count") is not None and (type(item["producers_total_count"]) is not int or item["producers_total_count"] < 0))):
        raise SystemExit("preview-data-resources: invalid Queue entry")
    print(json.dumps(item, sort_keys=True))
PY
    pages="$(python3 - "$tmp/response.json" <<'PY'
import json, sys
value = json.load(open(sys.argv[1])).get("result_info", {}).get("total_pages")
if not isinstance(value, int) or value < 1: raise SystemExit("preview-data-resources: invalid Queue page count")
print(value)
PY
)" || exit 3
    cat "$tmp/queue-page.jsonl" >>"$tmp/queues.jsonl"
    ((page >= pages)) && break
    ((page++))
  done
}

inventory() {
  list_buckets
  list_queues
  python3 - "$tmp/buckets.jsonl" "$tmp/queues.jsonl" <<'PY'
import json, re, sys
pattern = re.compile(r"^thrust-pr-([1-9][0-9]{0,8})-(blobs|events)$")
resources = {}
for path, field in ((sys.argv[1], "bucket"), (sys.argv[2], "queue")):
    for line in open(path):
        item = json.loads(line); name = item["name"] if field == "bucket" else item["queue_name"]
        match = pattern.fullmatch(name)
        if not match or ((field == "bucket") != (match.group(2) == "blobs")): continue
        number = int(match.group(1)); entry = resources.setdefault(number, {"number": number})
        if field in entry: raise SystemExit(f"preview-data-resources: duplicate {field} for pr_{number}")
        entry[field] = name
        if field == "queue":
            entry["queue_id"] = item["queue_id"]
            entry["consumers_total_count"] = item.get("consumers_total_count")
            entry["producers_total_count"] = item.get("producers_total_count")
for number in sorted(resources): print(json.dumps(resources[number], sort_keys=True))
PY
}

find_one() { # number; writes JSON object to tmp/entry.json, 0 if found, 1 absent
  inventory >"$tmp/inventory.jsonl"
  python3 - "$tmp/inventory.jsonl" "$1" >"$tmp/entry.json" <<'PY'
import json, sys
items = [json.loads(line) for line in open(sys.argv[1])]
hits = [item for item in items if item["number"] == int(sys.argv[2])]
if len(hits) > 1: raise SystemExit("preview-data-resources: duplicate preview inventory entry")
if not hits: raise SystemExit(1)
print(json.dumps(hits[0], sort_keys=True))
PY
}

complete_entry() { # number expected bucket expected queue entry-file
  python3 - "$1" "$2" "$3" "$4" <<'PY'
import json, sys
entry = json.load(open(sys.argv[4])); n, bucket, queue = sys.argv[1:4]
if entry.get("number") != int(n) or entry.get("bucket") != bucket or entry.get("queue") != queue or not isinstance(entry.get("queue_id"), str) or not entry["queue_id"]:
    raise SystemExit("preview-data-resources: resource pair is partial or does not match the reserved Preview names; refuse adoption")
PY
}

create_bucket() {
  local bucket="$1" body
  body="$(NAME="$bucket" python3 -c 'import json,os; print(json.dumps({"name": os.environ["NAME"]}))')"
  api POST "/accounts/$CLOUDFLARE_ACCOUNT_ID/r2/buckets" "$body" >"$tmp/response.json" || die "Cloudflare API unreachable while creating R2 bucket" 3
  require_success "$tmp/response.json" "R2 bucket creation" || exit 3
}

create_queue() {
  local queue="$1" body
  body="$(NAME="$queue" python3 -c 'import json,os; print(json.dumps({"queue_name": os.environ["NAME"]}))')"
  api POST "/accounts/$CLOUDFLARE_ACCOUNT_ID/queues" "$body" >"$tmp/response.json" || die "Cloudflare API unreachable while creating Queue" 3
  require_success "$tmp/response.json" "Queue creation" || exit 3
}

delete_queue() {
  api DELETE "/accounts/$CLOUDFLARE_ACCOUNT_ID/queues/$1" >"$tmp/response.json" || die "Cloudflare API unreachable while deleting Queue" 3
  require_success "$tmp/response.json" "Queue deletion" || exit 3
}

delete_bucket() { # returns 0 if deleted, 1 if the API refused deletion (for a nonempty bucket)
  api DELETE "/accounts/$CLOUDFLARE_ACCOUNT_ID/r2/buckets/$1" >"$tmp/response.json" || die "Cloudflare API unreachable while deleting R2 bucket" 3
  python3 - "$tmp/response.json" <<'PY'
import json, sys
try: d = json.load(open(sys.argv[1]))
except (OSError, json.JSONDecodeError): raise SystemExit(3)
raise SystemExit(0 if d.get("success") is True else 1)
PY
}

stage_bucket_cleanup() { # number bucket
  local n="$1" bucket="$2" rule body
  rule="$(cleanup_rule "$n")"
  api GET "/accounts/$CLOUDFLARE_ACCOUNT_ID/r2/buckets/$bucket/lifecycle" >"$tmp/response.json" || die "Cloudflare API unreachable while reading R2 lifecycle" 3
  require_success "$tmp/response.json" "R2 lifecycle list" || exit 3
  body="$(python3 - "$tmp/response.json" "$rule" <<'PY'
import json, sys
d = json.load(open(sys.argv[1])); rule = sys.argv[2]
rules = d.get("result", {}).get("rules")
if not isinstance(rules, list) or any(not isinstance(x, dict) or not isinstance(x.get("id"), str) for x in rules):
    raise SystemExit("preview-data-resources: invalid R2 lifecycle response")
mine = [x for x in rules if x["id"] == rule]
if mine:
    if len(mine) != 1 or mine[0].get("enabled") is not True or mine[0].get("conditions") != {"prefix": ""} or mine[0].get("deleteObjectsTransition") != {"condition": {"type": "Age", "maxAge": 86400}}:
        raise SystemExit("preview-data-resources: cleanup lifecycle rule drifted")
    print(json.dumps({"rules": rules})); raise SystemExit(0)
if rules:
    raise SystemExit("preview-data-resources: bucket has lifecycle rules not owned by this controller; refusing to replace them")
print(json.dumps({"rules": [{"id": rule, "enabled": True, "conditions": {"prefix": ""}, "deleteObjectsTransition": {"condition": {"type": "Age", "maxAge": 86400}}}]}))
PY
)" || exit 3
  # Existing controller rule is already idempotent.  Sending its complete current list is safe;
  # unknown rules returned above fail closed rather than being overwritten by this PUT.
  api PUT "/accounts/$CLOUDFLARE_ACCOUNT_ID/r2/buckets/$bucket/lifecycle" "$body" >"$tmp/response.json" || die "Cloudflare API unreachable while staging R2 cleanup" 3
  require_success "$tmp/response.json" "R2 lifecycle update" || exit 3
}

cmd="${1:-}"; shift || true
case "$cmd" in
  list-pr)
    [[ $# -eq 0 ]] || die "list-pr takes no arguments"
    need_env; inventory | python3 -c 'import json,sys; print(json.dumps([json.loads(x) for x in sys.stdin if x.strip()], sort_keys=True))'
    ;;
  ensure-pr)
    n="${1:-}"
    if [[ $# -ne 1 ]] || ! valid_number "$n"; then die "ensure-pr needs one PR number between 1 and 999999999"; fi
    need_env; bucket="$(bucket_name "$n")"; queue="$(queue_name "$n")"
    if find_one "$n"; then complete_entry "$n" "$bucket" "$queue" "$tmp/entry.json"; echo "preview-data-resources: existing $bucket and $queue verified"; exit 0; fi
    # No matching names existed in the freshly read inventory; create the pair then re-read it.
    create_bucket "$bucket"
    if ! create_queue "$queue"; then
      # The just-created bucket is empty.  Attempt best-effort rollback; leave any failed cleanup
      # visible for the next inventory rather than pretending a partial pair is safe to adopt.
      delete_bucket "$bucket" || true
      exit 3
    fi
    find_one "$n" || die "created Preview resource pair did not appear in inventory" 3
    complete_entry "$n" "$bucket" "$queue" "$tmp/entry.json"
    echo "preview-data-resources: created $bucket and $queue"
    ;;
  verify-pr)
    n="${1:-}"
    if [[ $# -ne 1 ]] || ! valid_number "$n"; then die "verify-pr needs one PR number between 1 and 999999999"; fi
    need_env; find_one "$n" || die "Preview resource pair is absent" 1
    complete_entry "$n" "$(bucket_name "$n")" "$(queue_name "$n")" "$tmp/entry.json"
    cat "$tmp/entry.json"
    ;;
  drop-pr)
    n="${1:-}"
    if [[ $# -ne 1 ]] || ! valid_number "$n"; then die "drop-pr needs one PR number between 1 and 999999999"; fi
    need_env
    if ! find_one "$n"; then echo "preview-data-resources: pr_$n has no data resources"; exit 0; fi
    complete_entry "$n" "$(bucket_name "$n")" "$(queue_name "$n")" "$tmp/entry.json"
    qid="$(python3 - "$tmp/entry.json" <<'PY'
import json, sys
entry = json.load(open(sys.argv[1]))
for key in ("consumers_total_count", "producers_total_count"):
    if type(entry.get(key)) is not int or entry[key] < 0:
        raise SystemExit(f"preview-data-resources: Queue {key} is absent or invalid; refusing deletion")
if entry["consumers_total_count"] or entry["producers_total_count"]:
    raise SystemExit("preview-data-resources: Queue is still bound to a consumer or producer; detach the Preview before deletion")
print(entry["queue_id"])
PY
)"
    delete_queue "$qid"
    if delete_bucket "$(bucket_name "$n")"; then echo "preview-data-resources: deleted data resources for pr_$n"; exit 0; fi
    stage_bucket_cleanup "$n" "$(bucket_name "$n")"
    echo "preview-data-resources: bucket for pr_$n is nonempty; expiry cleanup staged and deletion remains pending"
    ;;
  *) die "usage: preview-data-resources.sh {list-pr|ensure-pr|verify-pr|drop-pr} [pr-number]" ;;
esac
