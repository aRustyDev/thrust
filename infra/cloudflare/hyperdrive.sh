#!/usr/bin/env bash
# Hyperdrive configurations for Thrust's preview control plane.
#
# WHY THIS IS A SCRIPT AND NOT A tofu RESOURCE: the provider's cloudflare_hyperdrive_config
# takes `origin.password` as a plain sensitive string (not write-only), so declaring it would
# write the database password into this root's R2 state. Secrets never enter state here.
# WHY IT IS NOT `wrangler hyperdrive create`: wrangler only takes --password on the COMMAND
# LINE (and --connection-string likewise), where `ps` shows it. This script builds the request
# body from the environment and sends it on curl's stdin, and keeps the token in a 0600 header
# file, so no secret is ever in argv. The equivalent wrangler form is in README.md.
#
#   hyperdrive.sh check  [--vpc-service-id <id> --dev-vpc-service-id <id>]
#       READ-ONLY and idempotent. Asserts the two standard configs (thrust-pg, thrust-pg-dev)
#       exist, route through the expected VPC service, database and user, AND have query caching
#       DISABLED (owner decision: strict read-after-write). Never writes.
#   hyperdrive.sh ensure <config-name> --vpc-service <thrust-pg|thrust-pg-dev> --user <role>
#                        [--database thrust] [--vpc-service-id <id>] [--origin-connection-limit N] [--caching-enabled]
#                        [--password-file <file> | --password-sm-key <name>] [--dry-run]
#       Creates the config if ABSENT; if it exists, reports drift and changes nothing. Query
#       CACHING IS DISABLED BY DEFAULT for every config created here (thrust-pg, thrust-pg-dev and
#       per-PR configs alike: strict read-after-write, and parity); --caching-enabled opts a config
#       in to Cloudflare's default caching, e.g. a SEPARATE config for a caching measurement.
#       PER-PR ROLES: for `--user pr_<digits>` (a role made by the preview provisioner,
#       whose CONNECTION LIMIT is THRUST_PR_ROLE_CONNECTION_LIMIT from infra/preview/pr-role-limits.sh)
#       --origin-connection-limit DEFAULTS to that constant and a LARGER value is refused (exit 2):
#       Hyperdrive's own default is 60 on a paid plan, and the role would reject every connection
#       above its limit under load. Any role: the limit must be >= 5 (Hyperdrive's documented
#       minimum). `thrust` roles are unchanged: no limit unless given (production 40, dev 20). The
#       password comes from $HYPERDRIVE_ORIGIN_PASSWORD, a 0600 file, or AWS Secrets Manager
#       (`aws secretsmanager get-secret-value`, read inside this process). It is never printed.
#       --dry-run prints the request with the password redacted and makes NO network call.
#   hyperdrive.sh rotate-password <config-name> [--password-file <file> | --password-sm-key <name>] [--dry-run]
#       Replaces ONLY the origin password of an existing VPC-service config (PUT with the live
#       origin rebuilt around the new password). A PUT RESETS any omitted caching block to
#       Cloudflare's default (enabled), so the live `caching` and connection limit are carried
#       over — rotating a password must never silently turn caching back on. Run it back-to-back
#       with the ALTER ROLE.
#   hyperdrive.sh list-pr
#       Prints a JSON array of only managed per-PR configs. Standard environment configs are not
#       in the reconciliation surface.
#   hyperdrive.sh drop-pr <pr-number>
#       Deletes only the exact config named thrust-pr-<pr-number>. Missing is success.
#
# Environment: HYPERDRIVE_API_TOKEN, else CLOUDFLARE_API_TOKEN (Account > Hyperdrive > Write for
# `ensure`/`rotate-password`, Read for `check`), CLOUDFLARE_ACCOUNT_ID. VPC service ids come from the flags, from
# $THRUST_VPC_SERVICE_ID / $THRUST_DEV_VPC_SERVICE_ID, or from `tofu output -json
# vpc_service_ids` in this root. Exit codes: 0 ok, 1 missing/drift, 2 usage, 3 could not
# evaluate (no token, API error) — never conflated with 0.
set -euo pipefail

API="${CLOUDFLARE_API_BASE:-https://api.cloudflare.com/client/v4}"
CURL="${CURL:-curl}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# The per-PR connection budget: ONE named constant shared with the provisioner bootstrap.
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../preview/pr-role-limits.sh
source "$ROOT/../preview/pr-role-limits.sh"
[[ "${THRUST_PR_ROLE_CONNECTION_LIMIT:-}" =~ ^[0-9]+$ ]] || {
  echo "hyperdrive: infra/preview/pr-role-limits.sh did not define THRUST_PR_ROLE_CONNECTION_LIMIT as a number" >&2
  exit 3
}
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

die() { echo "hyperdrive: $*" >&2; exit "${2:-2}"; }

need_env() {
  [[ -n "${CLOUDFLARE_ACCOUNT_ID:-}" ]] || die "CLOUDFLARE_ACCOUNT_ID is not set" 3
  # A dedicated Hyperdrive token wins, so the token that can edit tunnels need not also be able
  # to rewrite database credentials.
  TOKEN="${HYPERDRIVE_API_TOKEN:-${CLOUDFLARE_API_TOKEN:-}}"
  [[ -n "$TOKEN" ]] || die "neither HYPERDRIVE_API_TOKEN nor CLOUDFLARE_API_TOKEN is set" 3
}

# curl wrapper: token in a 0600 header FILE (-H @file), body on stdin. Prints the response body.
api() { # method path [stdin-body]
  local method="$1" path="$2"
  (umask 077 && printf 'Authorization: Bearer %s\n' "$TOKEN" >"$tmp/hdr")
  if [[ -n "${3:-}" ]]; then
    printf '%s' "$3" | "$CURL" -sS --max-time 30 -X "$method" -H "@$tmp/hdr" \
      -H 'Content-Type: application/json' --data-binary @- "$API$path"
  else
    "$CURL" -sS --max-time 30 -X "$method" -H "@$tmp/hdr" "$API$path"
  fi
}

vpc_service_id() { # name -> id
  local name="$1" var
  if [[ "$name" == "thrust-pg" ]]; then var="${THRUST_VPC_SERVICE_ID:-}"; else var="${THRUST_DEV_VPC_SERVICE_ID:-}"; fi
  if [[ -n "$var" ]]; then echo "$var"; return; fi
  command -v tofu >/dev/null || die "no VPC service id for $name: pass the flag or install tofu" 3
  tofu -chdir="$ROOT" output -json vpc_service_ids 2>/dev/null |
    python3 -c 'import json,sys; print(json.load(sys.stdin)[sys.argv[1]])' "$name" ||
    die "could not read vpc_service_ids[$name] from tofu output (apply the cloudflare root first)" 3
}

list_configs() {
  local page=1 total_pages total_count count=0
  : >"$tmp/pages.jsonl"
  while :; do
    api GET "/accounts/$CLOUDFLARE_ACCOUNT_ID/hyperdrive/configs?per_page=100&page=$page" >"$tmp/page.json" || die "Cloudflare API unreachable" 3
    total_count="$(python3 - "$tmp/page.json" "$page" <<'PY'
import json, sys
try:
    d = json.load(open(sys.argv[1]))
    info = d['result_info']
    page = int(sys.argv[2])
    if d.get('success') is not True or not isinstance(d['result'], list): raise ValueError()
    if type(info['total_count']) is not int or info['total_count'] < 0 or info['page'] != page or info['per_page'] != 100: raise ValueError()
    if len(d['result']) != min(100, max(0, info['total_count'] - (page - 1) * 100)): raise ValueError()
    print(info['total_count'])
except (KeyError, TypeError, ValueError, OSError):
    raise SystemExit('hyperdrive: invalid or incomplete pagination')
PY
)" || die "could not evaluate: the Hyperdrive list call failed" 3
    if [[ "$page" -eq 1 ]]; then count="$total_count"; fi
    [[ "$total_count" == "$count" ]] || die "Hyperdrive inventory changed while listing" 3
    ((count <= 100000)) || die "Hyperdrive inventory is unexpectedly large" 3
    python3 -c 'import json,sys; print(json.dumps(json.load(open(sys.argv[1]))["result"]))' "$tmp/page.json" >>"$tmp/pages.jsonl"
    total_pages=$(( (count + 99) / 100 ))
    [[ "$page" -ge "$total_pages" ]] && break
    page=$((page + 1))
  done
  python3 - "$tmp/pages.jsonl" <<'PY'
import json, sys
entries = [entry for line in open(sys.argv[1]) for entry in json.loads(line)]
ids = [entry.get('id') for entry in entries if isinstance(entry, dict)]
if len(ids) != len(entries) or any(not isinstance(i, str) or not i for i in ids) or len(set(ids)) != len(ids):
    raise SystemExit('hyperdrive: duplicate or invalid inventory entries')
json.dump(entries, sys.stdout)
PY
}

# Compare one config against expectation; prints a line; returns 0 ok, 1 missing/drift.
judge() { # name service-id user database caching-off(1|0)  (configs JSON on $tmp/configs.json)
  python3 - "$tmp/configs.json" "$1" "$2" "$3" "$4" "${5:-1}" <<'PY'
import json, sys
path, name, sid, user, db, want_off = sys.argv[1:7]
matches = [c for c in json.load(open(path)) if c.get("name") == name]
if not matches:
    print(f"  MISSING  {name}: no Hyperdrive config with this name"); sys.exit(1)
if len(matches) > 1:
    print(f"  DRIFT    {name}: {len(matches)} configs share this name"); sys.exit(1)
o = matches[0].get("origin", {})
bad = []
if o.get("service_id") != sid: bad.append(f"service_id {o.get('service_id')!r} != expected {sid!r}")
if o.get("user") != user: bad.append(f"user {o.get('user')!r} != {user!r}")
if o.get("database") != db: bad.append(f"database {o.get('database')!r} != {db!r}")
if o.get("scheme") not in ("postgres", "postgresql"): bad.append(f"scheme {o.get('scheme')!r}")
# Caching: "disabled" must be exactly true. A missing block means Cloudflare's default (ENABLED).
off = (matches[0].get("caching") or {}).get("disabled") is True
if want_off == "1" and not off: bad.append("query caching is ENABLED (expected disabled: strict read-after-write)")
if want_off == "0" and off: bad.append("query caching is disabled (this config was requested with --caching-enabled)")
if bad:
    print(f"  DRIFT    {name} ({matches[0].get('id')}): " + "; ".join(bad)); sys.exit(1)
print(f"  OK       {name} id={matches[0].get('id')} via VPC service {sid}, {user}@{db}, caching {'disabled' if off else 'enabled'}"); sys.exit(0)
PY
}

cmd="${1:-}"
shift || true
case "$cmd" in
  check)
    prod_id="" dev_id=""
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --vpc-service-id) prod_id="$2"; shift ;;
        --dev-vpc-service-id) dev_id="$2"; shift ;;
        *) die "unknown flag $1" ;;
      esac
      shift
    done
    need_env
    [[ -n "$prod_id" ]] && THRUST_VPC_SERVICE_ID="$prod_id"
    [[ -n "$dev_id" ]] && THRUST_DEV_VPC_SERVICE_ID="$dev_id"
    # Resolve ids into plain assignments FIRST: a failure inside `judge "$(...)"` would be
    # swallowed and read as a drift (exit 1) instead of "could not evaluate" (exit 3).
    prod_sid="$(vpc_service_id thrust-pg)"
    dev_sid="$(vpc_service_id thrust-pg-dev)"
    list_configs >"$tmp/configs.json"
    rc=0
    judge thrust-pg "$prod_sid" thrust thrust || rc=1
    judge thrust-pg-dev "$dev_sid" thrust thrust || rc=1
    exit "$rc"
    ;;
  ensure)
    name="${1:-}"
    [[ -n "$name" && "$name" != --* ]] || die "ensure needs a config name"
    shift
    service="" user="" database="thrust" sid="" limit="" pwfile="" smkey="" dry=0 nocache=1
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --vpc-service) service="$2"; shift ;;
        --user) user="$2"; shift ;;
        --database) database="$2"; shift ;;
        --vpc-service-id) sid="$2"; shift ;;
        --origin-connection-limit) limit="$2"; shift ;;
        --caching-enabled) nocache=0 ;;
        --password-file) pwfile="$2"; shift ;;
        --password-sm-key) smkey="$2"; shift ;;
        --dry-run) dry=1 ;;
        *) die "unknown flag $1" ;;
      esac
      shift
    done
    [[ "$service" == "thrust-pg" || "$service" == "thrust-pg-dev" ]] || die "--vpc-service must be thrust-pg or thrust-pg-dev"
    [[ -n "$user" ]] || die "--user is required"
    [[ "$name" =~ ^[A-Za-z0-9._-]{1,64}$ ]] || die "config name must match [A-Za-z0-9._-]{1,64}"
    [[ "$user" =~ ^[a-z_][a-z0-9_]*$ && "$database" =~ ^[a-z_][a-z0-9_]*$ ]] || die "user/database must be plain lowercase identifiers"
    [[ -z "$limit" || "$limit" =~ ^[0-9]+$ ]] || die "--origin-connection-limit must be a number"
    [[ -z "$limit" ]] || limit=$((10#$limit)) # "08" is not an octal error
    # Hyperdrive's documented floor: Create Hyperdrive API reference, origin_connection_limit "minimum 5".
    [[ -z "$limit" || "$limit" -ge 5 ]] || die "--origin-connection-limit must be >= 5 (Hyperdrive's documented minimum), got $limit"
    # A per-PR role has CONNECTION LIMIT = the shared constant at bootstrap. Anything above it
    # is rejected by Postgres under load, so default to the constant and refuse a larger value.
    if [[ "$user" =~ ^pr_[0-9]+$ ]]; then
      if [[ -z "$limit" ]]; then
        limit="$THRUST_PR_ROLE_CONNECTION_LIMIT"
      elif ((limit > THRUST_PR_ROLE_CONNECTION_LIMIT)); then
        die "--origin-connection-limit $limit exceeds the per-PR role's CONNECTION LIMIT of $THRUST_PR_ROLE_CONNECTION_LIMIT (infra/preview/pr-role-limits.sh): role $user would reject the excess connections under load. Use <= $THRUST_PR_ROLE_CONNECTION_LIMIT, or omit the flag."
      fi
    fi
    if [[ -z "$sid" ]]; then
      if [[ "$dry" -eq 1 ]]; then sid="<vpc-service-id>"; else sid="$(vpc_service_id "$service")"; fi
    fi
    # Password: env > file > Secrets Manager. Never echoed.
    if [[ -n "${HYPERDRIVE_ORIGIN_PASSWORD:-}" ]]; then pw="$HYPERDRIVE_ORIGIN_PASSWORD"
    elif [[ -n "$pwfile" ]]; then pw="$(<"$pwfile")"
    elif [[ -n "$smkey" && "$dry" -eq 0 ]]; then
      pw="$(aws secretsmanager get-secret-value --secret-id "$smkey" --query SecretString --output text)"
    elif [[ "$dry" -eq 1 ]]; then pw="<redacted>"
    else die "no password source: set HYPERDRIVE_ORIGIN_PASSWORD, or pass --password-file / --password-sm-key"; fi
    [[ -n "$pw" ]] || die "the password source is empty" 3
    body="$(NAME="$name" SID="$sid" DB="$database" USR="$user" PW="$pw" LIMIT="$limit" NOCACHE="$nocache" python3 -c '
import json, os
b = {"name": os.environ["NAME"],
     "origin": {"scheme": "postgresql", "database": os.environ["DB"], "user": os.environ["USR"],
                "password": os.environ["PW"], "service_id": os.environ["SID"]}}
if os.environ["LIMIT"]:
    b["origin_connection_limit"] = int(os.environ["LIMIT"])
if os.environ["NOCACHE"] == "1":
    b["caching"] = {"disabled": True}
print(json.dumps(b))
')"
    if [[ "$dry" -eq 1 ]]; then
      echo "POST /accounts/<account>/hyperdrive/configs   (dry run: no network call was made)"
      printf '%s' "$body" | python3 -c 'import json,sys; b=json.load(sys.stdin); b["origin"]["password"]="<redacted>"; print(json.dumps(b, indent=2))'
      exit 0
    fi
    need_env
    list_configs >"$tmp/configs.json"
    if python3 -c 'import json,sys; sys.exit(0 if any(c.get("name")==sys.argv[1] for c in json.load(open(sys.argv[2]))) else 1)' "$name" "$tmp/configs.json"; then
      echo "hyperdrive: a config named '$name' already exists; nothing was changed. Drift report:"
      judge "$name" "$sid" "$user" "$database" "$nocache" || exit 1
      exit 0
    fi
    resp="$(api POST "/accounts/$CLOUDFLARE_ACCOUNT_ID/hyperdrive/configs" "$body")" || die "Cloudflare API unreachable" 3
    printf '%s' "$resp" | python3 -c '
import json, sys
d = json.load(sys.stdin)
if not d.get("success"):
    print("hyperdrive: create FAILED:", [e.get("message") for e in d.get("errors", [])]); sys.exit(1)
print("hyperdrive: created", d["result"]["name"], "id =", d["result"]["id"])
'
    ;;
  rotate-password)
    name="${1:-}"
    [[ -n "$name" && "$name" != --* ]] || die "rotate-password needs a config name"
    shift
    pwfile="" smkey="" dry=0
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --password-file) pwfile="$2"; shift ;;
        --password-sm-key) smkey="$2"; shift ;;
        --dry-run) dry=1 ;;
        *) die "unknown flag $1" ;;
      esac
      shift
    done
    if [[ -n "${HYPERDRIVE_ORIGIN_PASSWORD:-}" ]]; then pw="$HYPERDRIVE_ORIGIN_PASSWORD"
    elif [[ -n "$pwfile" ]]; then pw="$(<"$pwfile")"
    elif [[ -n "$smkey" ]]; then pw="$(aws secretsmanager get-secret-value --secret-id "$smkey" --query SecretString --output text)"
    else die "no password source: set HYPERDRIVE_ORIGIN_PASSWORD, or pass --password-file / --password-sm-key"; fi
    [[ -n "$pw" ]] || die "the password source is empty" 3
    need_env
    list_configs >"$tmp/configs.json"
    # Keep everything about the existing origin except the password: a PUT replaces the whole
    # origin, so the body is rebuilt from what is live, never from what we think is live.
    if ! body="$(NAME="$name" PW="$pw" python3 -c '
import json, os, sys
hits = [c for c in json.load(open(sys.argv[1])) if c.get("name") == os.environ["NAME"]]
if len(hits) != 1:
    sys.stderr.write("hyperdrive: expected exactly one config named %s, found %d\n" % (os.environ["NAME"], len(hits)))
    sys.exit(1)
o = hits[0]["origin"]
if not o.get("service_id"):
    sys.stderr.write("hyperdrive: %s is not a VPC-service config; refusing to rebuild its origin\n" % os.environ["NAME"])
    sys.exit(1)
body = {"name": hits[0]["name"],
        "origin": {"scheme": o["scheme"], "database": o["database"], "user": o["user"],
                   "password": os.environ["PW"], "service_id": o["service_id"]}}
# A PUT RESETS omitted caching settings to Cloudflare defaults (caching ENABLED). Carry the live
# block over verbatim, or a password rotation would silently undo "caching disabled".
for key in ("caching", "origin_connection_limit"):
    if hits[0].get(key) is not None:
        body[key] = hits[0][key]
print(json.dumps({"id": hits[0]["id"], "body": body}))
' "$tmp/configs.json")"; then
      exit 1
    fi
    cfg_id="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])' <<<"$body")"
    put_body="$(python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin)["body"]))' <<<"$body")"
    if [[ "$dry" -eq 1 ]]; then
      echo "PUT /accounts/<account>/hyperdrive/configs/$cfg_id   (dry run: no write was made)"
      python3 -c 'import json,sys; b=json.load(sys.stdin); b["origin"]["password"]="<redacted>"; print(json.dumps(b, indent=2))' <<<"$put_body"
      exit 0
    fi
    resp="$(api PUT "/accounts/$CLOUDFLARE_ACCOUNT_ID/hyperdrive/configs/$cfg_id" "$put_body")" || die "Cloudflare API unreachable" 3
    printf '%s' "$resp" | python3 -c '
import json, sys
d = json.load(sys.stdin)
if not d.get("success"):
    print("hyperdrive: password rotation FAILED:", [e.get("message") for e in d.get("errors", [])]); sys.exit(1)
print("hyperdrive: password replaced on", d["result"]["name"], "(id", d["result"]["id"] + ")")
'
    ;;
  list-pr)
    [[ $# -eq 0 ]] || die "list-pr takes no arguments"
    need_env
    list_configs >"$tmp/configs.json"
    python3 -c '
import json, re, sys
out = []
for c in json.load(open(sys.argv[1])):
    m = re.fullmatch(r"thrust-pr-([0-9]{1,9})", c.get("name", ""))
    if m:
        out.append({"number": int(m.group(1)), "id": c.get("id"), "name": c["name"]})
print(json.dumps(sorted(out, key=lambda x: x["number"])))
' "$tmp/configs.json"
    ;;
  drop-pr)
    n="${1:-}"
    [[ "$n" =~ ^[0-9]{1,9}$ && $# -eq 1 ]] || die "drop-pr needs one 1-9 digit PR number"
    name="thrust-pr-$n"
    need_env
    list_configs >"$tmp/configs.json"
    cfg_id="$(NAME="$name" python3 - "$tmp/configs.json" <<'PY'
import json, os, sys
hits = [c for c in json.load(open(sys.argv[1])) if c.get("name") == os.environ["NAME"]]
if len(hits) > 1:
    sys.stderr.write("hyperdrive: more than one config has the managed name %s\\n" % os.environ["NAME"])
    sys.exit(1)
if not hits:
    sys.exit(4)
print(hits[0]["id"])
PY
    )" || rc=$?
    if [[ "${rc:-0}" -eq 4 ]]; then echo "hyperdrive: $name is already absent"; exit 0; fi
    [[ "${rc:-0}" -eq 0 ]] || exit "$rc"
    resp="$(api DELETE "/accounts/$CLOUDFLARE_ACCOUNT_ID/hyperdrive/configs/$cfg_id")" || die "Cloudflare API unreachable" 3
    printf '%s' "$resp" | python3 -c '
import json, sys
d = json.load(sys.stdin)
if not d.get("success"):
    print("hyperdrive: delete FAILED:", [e.get("message") for e in d.get("errors", [])])
    sys.exit(1)
print("hyperdrive: deleted", sys.argv[1])
' "$name"
    ;;
  *)
    sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d; s/^# \{0,1\}//' >&2
    exit 2
    ;;
esac
