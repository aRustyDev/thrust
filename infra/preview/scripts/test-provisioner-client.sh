#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
mkdir "$tmp/bin"
cat >"$tmp/bin/curl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$@" >"$TRACE"
for arg in "$@"; do
  case "$arg" in
    @*/headers) cp "${arg#@}" "$CAPTURE_HEADERS" ;;
    @*/body) cp "${arg#@}" "$CAPTURE_BODY" ;;
  esac
done
printf '%s' "$RESPONSE"
SH
chmod +x "$tmp/bin/curl"
export PATH="$tmp/bin:$PATH" TRACE="$tmp/trace" CAPTURE_HEADERS="$tmp/headers" CAPTURE_BODY="$tmp/body"
export THRUST_PREVIEW_PROVISIONER_URL=https://provisioner.example.workers.dev
export PREVIEW_PROVISIONER_ACCESS_CLIENT_ID=client-id PREVIEW_PROVISIONER_ACCESS_CLIENT_SECRET=secret-value
export RESPONSE='[{"number":42,"schema":true,"login_role":true,"search_path":true,"pending":false}]'
client="$here/provisioner-client.sh"
bash "$client" list --json >"$tmp/inventory"
python3 - "$tmp/inventory" <<'PY'
import json, sys
assert json.load(open(sys.argv[1]))[0]['number'] == 42
PY
printf '0123456789abcdef0123456789abcdef' >"$tmp/password"
bash "$client" create 42 --password-file "$tmp/password"
python3 - "$tmp/body" <<'PY'
import json, sys
assert json.load(open(sys.argv[1])) == {'password': '0123456789abcdef0123456789abcdef'}
PY
grep -q 'CF-Access-Client-Secret: secret-value' "$tmp/headers"
if grep -Eq 'secret-value|client-id|0123456789abcdef' "$tmp/trace"; then echo 'credential leaked into argv' >&2; exit 1; fi
bash "$client" drop 42
if bash "$client" drop 042 >/dev/null 2>&1; then echo 'noncanonical number accepted' >&2; exit 1; fi
RESPONSE='{}' bash "$client" list >/dev/null 2>&1 && { echo 'invalid inventory accepted' >&2; exit 1; }
RESPONSE='[{"number":42,"schema":true,"login_role":true,"search_path":true,"pending":false},{"number":42,"schema":true,"login_role":true,"search_path":true,"pending":false}]' bash "$client" list >/dev/null 2>&1 && { echo 'duplicate inventory accepted' >&2; exit 1; }
echo 'provisioner client tests: PASS'
