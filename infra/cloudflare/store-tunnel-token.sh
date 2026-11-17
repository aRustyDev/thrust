#!/usr/bin/env bash
# Fetch the connector token of the dedicated thrust-pg tunnel from the Cloudflare API and store
# it in AWS Secrets Manager (/thrust/cloudflared-tunnel-token), from where ESO delivers it to the
# in-cluster connector. Run ONCE after `tofu apply` of this root created the tunnel, and again
# only to rotate the token.
#
# The token is a credential for the whole tunnel. It must reach Secrets Manager without touching
# state (this root has no token data source on purpose), a terminal, shell history, a log or a
# command line. So: API response -> a 0600 temp file -> `put-secret-value --secret-string
# file://...` -> temp file removed. The script prints the token's LENGTH, never the value.
#
#   store-tunnel-token.sh [--tunnel-id <id>] [--secret-id <name>] [--dry-run]
#
# Environment: CLOUDFLARE_API_TOKEN (needs Account > Cloudflare Tunnel > Read), CLOUDFLARE_ACCOUNT_ID,
# AWS credentials for Secrets Manager PutSecretValue (e.g. AWS_PROFILE=tf-admin-aiml-dev; the
# OpenTofu deploy principal is deliberately denied it, ADR 0004), AWS_REGION (default us-east-1).
# --tunnel-id defaults to `tofu output -raw thrust_tunnel_id` in this root.
# --dry-run resolves everything and prints the plan; it makes NO network call.
set -euo pipefail

API="${CLOUDFLARE_API_BASE:-https://api.cloudflare.com/client/v4}"
CURL="${CURL:-curl}"
AWS="${AWS:-aws}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
secret_id="/thrust/cloudflared-tunnel-token"
tunnel_id="${THRUST_TUNNEL_ID:-}"
dry=0

die() { echo "store-tunnel-token: $*" >&2; exit "${2:-2}"; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tunnel-id) tunnel_id="$2"; shift ;;
    --secret-id) secret_id="$2"; shift ;;
    --dry-run) dry=1 ;;
    -h | --help) sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown flag $1" ;;
  esac
  shift
done

[[ "$secret_id" =~ ^/thrust/[a-z0-9-]+$ ]] || die "--secret-id must be a /thrust/<name> container"
if [[ -z "$tunnel_id" ]]; then
  tunnel_id="$(tofu -chdir="$ROOT" output -raw thrust_tunnel_id 2>/dev/null)" ||
    die "no tunnel id: pass --tunnel-id or apply this root first" 3
fi
[[ "$tunnel_id" =~ ^[0-9a-f-]{36}$ ]] || die "tunnel id does not look like a UUID: '$tunnel_id'"

if [[ "$dry" -eq 1 ]]; then
  echo "would GET  $API/accounts/<account>/cfd_tunnel/$tunnel_id/token"
  echo "would PUT  Secrets Manager $secret_id (region ${AWS_REGION:-us-east-1}) from a 0600 temp file"
  echo "dry run: no network call was made and nothing was written"
  exit 0
fi

[[ -n "${CLOUDFLARE_ACCOUNT_ID:-}" ]] || die "CLOUDFLARE_ACCOUNT_ID is not set" 3
[[ -n "${CLOUDFLARE_API_TOKEN:-}" ]] || die "CLOUDFLARE_API_TOKEN is not set" 3

tmp="$(umask 077 && mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
(umask 077 && printf 'Authorization: Bearer %s\n' "$CLOUDFLARE_API_TOKEN" >"$tmp/hdr")

"$CURL" -sS --max-time 30 -H "@$tmp/hdr" \
  "$API/accounts/$CLOUDFLARE_ACCOUNT_ID/cfd_tunnel/$tunnel_id/token" >"$tmp/resp.json" ||
  die "Cloudflare API unreachable" 3

# Extract .result straight into the temp file, no trailing newline (a newline inside a token or
# a password would be stored and then break the connector).
python3 - "$tmp/resp.json" "$tmp/token" <<'PY' || die "could not read a token from the API response (wrong tunnel id, or a token without Tunnel Read?)" 3
import json, re, sys
d = json.load(open(sys.argv[1]))
t = d.get("result") if d.get("success") else None
if not isinstance(t, str) or len(t) < 40 or not re.fullmatch(r"[A-Za-z0-9+/=_-]+", t):
    sys.exit(1)
with open(sys.argv[2], "w") as fh:
    fh.write(t)
PY
chmod 600 "$tmp/token"

"$AWS" secretsmanager put-secret-value --region "${AWS_REGION:-us-east-1}" \
  --secret-id "$secret_id" --secret-string "file://$tmp/token" >/dev/null ||
  die "put-secret-value failed (is the /thrust/cloudflared-tunnel-token container present, and does this identity have PutSecretValue?)" 3

echo "stored a $(wc -c <"$tmp/token" | tr -d ' ')-character connector token in $secret_id"
