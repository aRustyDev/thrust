#!/usr/bin/env bash
# Client for the Access-protected preview provisioner. Credentials and request bodies stay in
# private files, never in curl's arguments. The caller owns the password file for Hyperdrive.
set -euo pipefail
umask 077

die() { printf 'provisioner-client: %s\n' "$1" >&2; exit 2; }
[[ -n "${THRUST_PREVIEW_PROVISIONER_URL:-}" ]] || die 'missing provisioner URL'
[[ "$THRUST_PREVIEW_PROVISIONER_URL" =~ ^https://[a-zA-Z0-9.-]+\.workers\.dev/?$ ]] || die 'URL must be an HTTPS workers.dev origin'
[[ -n "${PREVIEW_PROVISIONER_ACCESS_CLIENT_ID:-}" && -n "${PREVIEW_PROVISIONER_ACCESS_CLIENT_SECRET:-}" ]] || die 'missing Access credentials'
[[ "$PREVIEW_PROVISIONER_ACCESS_CLIENT_ID" != *$'\n'* && "$PREVIEW_PROVISIONER_ACCESS_CLIENT_SECRET" != *$'\n'* && "$PREVIEW_PROVISIONER_ACCESS_CLIENT_ID" != *$'\r'* && "$PREVIEW_PROVISIONER_ACCESS_CLIENT_SECRET" != *$'\r'* ]] || die 'invalid Access credentials'

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
printf 'CF-Access-Client-Id: %s\nCF-Access-Client-Secret: %s\n' "$PREVIEW_PROVISIONER_ACCESS_CLIENT_ID" "$PREVIEW_PROVISIONER_ACCESS_CLIENT_SECRET" >"$tmp/headers"
url="${THRUST_PREVIEW_PROVISIONER_URL%/}/v1/previews"
case "${1:-}" in
  list)
    [[ $# == 1 || ( $# == 2 && "$2" == --json ) ]] || die 'usage: list [--json]'
    curl --silent --show-error --fail --max-time 30 --header "@$tmp/headers" "$url" >"$tmp/response"
    python3 - "$tmp/response" <<'PY'
import json, sys
with open(sys.argv[1]) as f:
    data = json.load(f)
if not isinstance(data, list):
    raise SystemExit('provisioner-client: invalid inventory')
seen = set()
for item in data:
    if not isinstance(item, dict) or type(item.get('number')) is not int or not 1 <= item['number'] <= 999999999:
        raise SystemExit('provisioner-client: invalid inventory entry')
    if item['number'] in seen or any(type(item.get(k)) is not bool for k in ('schema', 'login_role', 'search_path', 'pending')):
        raise SystemExit('provisioner-client: invalid inventory entry')
    seen.add(item['number'])
print(json.dumps(data))
PY
    ;;
  create)
    [[ $# == 4 && "$3" == --password-file ]] || die 'usage: create <number> --password-file <file>'
    [[ "$2" =~ ^[1-9][0-9]{0,8}$ ]] || die 'invalid PR number'
    if [[ ! -s "$4" ]]; then openssl rand -hex 24 | tr -d '\n' >"$4"; chmod 600 "$4"; fi
    python3 - "$4" "$tmp/body" <<'PY'
import json, re, sys
with open(sys.argv[1]) as f:
    password = f.read()
if not re.fullmatch(r'[A-Za-z0-9_-]{24,128}', password):
    raise SystemExit('provisioner-client: invalid password file')
with open(sys.argv[2], 'w') as f:
    json.dump({'password': password}, f)
PY
    curl --silent --show-error --fail --max-time 30 --request PUT --header "@$tmp/headers" --header 'Content-Type: application/json' --data-binary "@$tmp/body" "$url/$2" >"$tmp/response"
    ;;
  drop)
    [[ $# == 2 && "$2" =~ ^[1-9][0-9]{0,8}$ ]] || die 'usage: drop <number>'
    curl --silent --show-error --fail --max-time 30 --request DELETE --header "@$tmp/headers" "$url/$2" >"$tmp/response"
    ;;
  acknowledge)
    [[ $# == 2 && "$2" =~ ^[1-9][0-9]{0,8}$ ]] || die 'usage: acknowledge <number>'
    curl --silent --show-error --fail --max-time 30 --request POST --header "@$tmp/headers" "$url/$2" >"$tmp/response"
    ;;
  *) die 'usage: list | create <number> --password-file <file> | acknowledge <number> | drop <number>' ;;
esac
