#!/usr/bin/env bash
# SC2015 (A && B || C) is benign in this file: `ok` cannot fail, so C only runs when A is false.
# shellcheck disable=SC2015
# Hermetic test for hyperdrive.sh and store-tunnel-token.sh: a fake curl serves canned Cloudflare
# responses and a fake aws records what it was given, so there is no network, no token, no AWS
# account and no Cloudflare account involved. Local only (not a pr-gates job).
#   bash infra/cloudflare/test-scripts.sh
# Each behaviour has a control that must FAIL: a checker that cannot say "missing", "drift" or
# "could not evaluate" would report success over nothing.
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# The per-PR connection budget comes from the ONE shared file, never a literal 8 in this test.
# shellcheck source-path=SCRIPTDIR
# shellcheck source=../preview/pr-role-limits.sh
source "$here/../preview/pr-role-limits.sh"
PRL="$THRUST_PR_ROLE_CONNECTION_LIMIT"
script="$here/hyperdrive.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
fail=0
ok() { printf '  PASS  %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=1; }

# Fake curl. GET -> $FAKE_LIST ; POST -> record stdin, answer success.
cat >"$tmp/curl" <<'FAKE'
#!/usr/bin/env bash
echo "$*" >>"$FAKE_CALLS"
method=GET
prev=""
for a in "$@"; do
  [[ "$prev" == "-X" ]] && method="$a"
  [[ "$a" == @* ]] && cp "${a#@}" "$FAKE_HDR"
  prev="$a"
done
if [[ "$method" == "POST" || "$method" == "PUT" ]]; then
  cat >"$FAKE_POST_BODY"
  printf '{"success":true,"errors":[],"result":{"id":"new-config-id","name":"created"}}'
elif [[ "$method" == "DELETE" ]]; then
  printf '{"success":true,"errors":[],"result":{"id":"deleted"}}'
else
  if [[ -n "${FAKE_PAGE_DIR:-}" ]]; then
    url="${!#}"; page="${url##*page=}"
    cat "$FAKE_PAGE_DIR/page$page.json"
  else
    python3 - "$FAKE_LIST" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
if isinstance(d.get('result'), list):
    n = len(d['result'])
    d['result_info'] = {'page': 1, 'per_page': 100, 'total_count': n}
print(json.dumps(d))
PY
  fi
fi
FAKE
chmod +x "$tmp/curl"
export CURL="$tmp/curl" FAKE_CALLS="$tmp/calls.log" FAKE_POST_BODY="$tmp/post.json" FAKE_HDR="$tmp/hdr.txt"
export CLOUDFLARE_ACCOUNT_ID=acct0000 CLOUDFLARE_API_TOKEN=tok-SECRET-0123456789
export THRUST_VPC_SERVICE_ID=vpc-prod THRUST_DEV_VPC_SERVICE_ID=vpc-dev

good='{"success":true,"errors":[],"result":[
 {"id":"h1","name":"thrust-pg","origin":{"scheme":"postgresql","database":"thrust","user":"thrust","service_id":"vpc-prod"},"caching":{"disabled":true},"origin_connection_limit":40},
 {"id":"h2","name":"thrust-pg-dev","origin":{"scheme":"postgresql","database":"thrust","user":"thrust","service_id":"vpc-dev"},"caching":{"disabled":true}}]}'
run() { : >"$FAKE_CALLS"; "$script" "$@" >"$tmp/out" 2>&1; echo $?; }

# --- check ----------------------------------------------------------------------------------
echo "$good" >"$tmp/list.json"; export FAKE_LIST="$tmp/list.json"
[[ "$(run check)" == 0 ]] && ok "check: both configs present and correct -> exit 0" || bad "check happy path"

mkdir "$tmp/pages"
python3 - "$tmp/pages" <<'PY'
import json, os, sys
root = sys.argv[1]
items = [{'id': f'id-{n}', 'name': f'thrust-pr-{n}'} for n in range(1, 102)]
for page in (1, 2):
    data = {'success': True, 'result': items[(page - 1) * 100:page * 100], 'result_info': {'page': page, 'per_page': 100, 'total_count': 101}}
    with open(os.path.join(root, f'page{page}.json'), 'w') as f: json.dump(data, f)
PY
rc="$(FAKE_PAGE_DIR="$tmp/pages" run list-pr)"
[[ "$rc" == 0 ]] && python3 - "$tmp/out" <<'PY' && ok 'pagination includes later pages' || bad 'pagination lost a page'
import json, sys
items = json.load(open(sys.argv[1]))
assert len(items) == 101 and items[-1]['number'] == 101
PY
printf '%s\n' '{"success":true,"result":[],"result_info":{"page":2,"per_page":100,"total_count":101}}' >"$tmp/pages/page2.json"
rc="$(FAKE_PAGE_DIR="$tmp/pages" run list-pr)"
[[ "$rc" == 3 ]] && ok 'control: incomplete pagination fails closed' || bad 'incomplete pagination accepted'

echo '{"success":true,"errors":[],"result":[{"id":"h1","name":"thrust-pg","origin":{"scheme":"postgresql","database":"thrust","user":"thrust","service_id":"vpc-prod"}}]}' >"$tmp/list.json"
rc="$(run check)"
[[ "$rc" == 1 ]] && grep -q "MISSING  thrust-pg-dev" "$tmp/out" && ok "control: a missing config -> exit 1 and says MISSING" || bad "control missing (rc=$rc)"

echo "${good//vpc-dev/vpc-WRONG}" >"$tmp/list.json"
rc="$(run check)"
[[ "$rc" == 1 ]] && grep -q "DRIFT    thrust-pg-dev" "$tmp/out" && ok "control: a config on the wrong VPC service -> exit 1 and says DRIFT" || bad "control drift (rc=$rc)"

# Owner decision: the standard configs have query caching DISABLED. Enabled - or a missing
# caching block, which means Cloudflare's default (enabled) - is drift.
on='"caching":{"disabled":false,"max_age":60}'
off='"caching":{"disabled":true},"origin_connection_limit":40'
echo "${good//$off/$on}" >"$tmp/list.json"
rc="$(run check)"
[[ "$rc" == 1 ]] && grep -q "DRIFT    thrust-pg .*caching is ENABLED" "$tmp/out" && ok "control: thrust-pg with caching ENABLED -> exit 1 and says so" || bad "control caching enabled (rc=$rc)"
nocache_block=',"caching":{"disabled":true}'
echo "${good//$nocache_block/}" >"$tmp/list.json"
rc="$(run check)"
[[ "$rc" == 1 ]] && [[ "$(grep -c 'caching is ENABLED' "$tmp/out")" == 2 ]] && ok "control: a MISSING caching block (Cloudflare default = enabled) is drift on both standard configs" || bad "control missing caching block (rc=$rc)"

echo '{"success":false,"errors":[{"message":"Authentication error"}],"result":null}' >"$tmp/list.json"
rc="$(run check)"
[[ "$rc" == 3 ]] && ok "control: an API failure is 'could not evaluate' (exit 3), never 0 or 1" || bad "control api failure (rc=$rc)"

echo "$good" >"$tmp/list.json"
rc="$(CLOUDFLARE_API_TOKEN='' run check)"
[[ "$rc" == 3 ]] && ok "control: no token -> exit 3" || bad "control no token (rc=$rc)"

rc="$(HYPERDRIVE_API_TOKEN=hd-ONLY-TOKEN-0123 run check)"
grep -q 'Bearer hd-ONLY-TOKEN-0123' "$FAKE_HDR" && ok "a dedicated HYPERDRIVE_API_TOKEN wins over CLOUDFLARE_API_TOKEN" || bad "token precedence (rc=$rc)"
rc="$(CLOUDFLARE_API_TOKEN='' HYPERDRIVE_API_TOKEN=hd-ONLY-TOKEN-0123 run check)"
[[ "$rc" == 0 ]] && ok "HYPERDRIVE_API_TOKEN alone is enough" || bad "hyperdrive token alone (rc=$rc)"

# --- ensure ---------------------------------------------------------------------------------
pw='correct-horse-battery-staple-0123456789'
export HYPERDRIVE_ORIGIN_PASSWORD="$pw"

rc="$(run ensure thrust-pr-12 --vpc-service thrust-pg-dev --user pr_12 --dry-run)"
if [[ "$rc" == 0 && ! -s "$FAKE_CALLS" ]] && grep -q '"password": "<redacted>"' "$tmp/out" && ! grep -qF "$pw" "$tmp/out"; then
  ok "ensure --dry-run: redacted body, password absent, ZERO network calls"
else bad "ensure dry-run (rc=$rc)"; fi

echo '{"success":true,"errors":[],"result":[]}' >"$tmp/list.json"
rc="$(run ensure thrust-pr-12 --vpc-service thrust-pg-dev --user pr_12)"
if [[ "$rc" == 0 ]] && grep -q " -X POST " "$FAKE_CALLS"; then ok "ensure creates a config that is absent"; else bad "ensure create (rc=$rc)"; fi
grep -qF "$pw" "$FAKE_CALLS" && bad "password reached a curl command line" || ok "password is not in curl's argv (body on stdin)"
grep -qF "tok-SECRET" "$FAKE_CALLS" && bad "API token reached a curl command line" || ok "API token is not in curl's argv (-H @file)"
python3 - "$FAKE_POST_BODY" "$pw" "$PRL" <<'PY' && ok "POST body: VPC service origin, user pr_12, the per-PR limit by default, password present" || bad "POST body shape"
import json, sys
b = json.load(open(sys.argv[1]))
o = b["origin"]
assert o["service_id"] == "vpc-dev" and o["user"] == "pr_12" and o["scheme"] == "postgresql"
assert o["database"] == "thrust" and o["password"] == sys.argv[2] and b["origin_connection_limit"] == int(sys.argv[3])
assert "host" not in o and "port" not in o  # VPC origin conflicts with host/port
PY

# --- per-PR connection limit: ONE constant shared with the provisioner bootstrap -----------------
# (a) a per-PR ensure WITHOUT the flag sends the constant (Hyperdrive's own default would be 60).
echo '{"success":true,"errors":[],"result":[]}' >"$tmp/list.json"
rc="$(run ensure thrust-pr-31 --vpc-service thrust-pg-dev --user pr_31)"
python3 - "$FAKE_POST_BODY" "$PRL" <<'PY' && ok "per-PR ensure WITHOUT --origin-connection-limit sends the shared per-PR limit ($PRL)" || bad "per-PR default limit (rc=$rc)"
import json, sys
assert json.load(open(sys.argv[1]))["origin_connection_limit"] == int(sys.argv[2])
PY
# an explicit value at or below the role's limit is honoured, not overwritten
rc="$(run ensure thrust-pr-32 --vpc-service thrust-pg-dev --user pr_32 --origin-connection-limit 5)"
python3 - "$FAKE_POST_BODY" <<'PY' && ok "per-PR ensure honours an explicit limit at or below the role's (5)" || bad "per-PR explicit limit (rc=$rc)"
import json, sys
assert json.load(open(sys.argv[1]))["origin_connection_limit"] == 5
PY
# (b) CONTROL: a per-PR ensure with a limit above the role's is REFUSED — exit 2, no POST, a clear message.
: >"$FAKE_CALLS"
rc="$(run ensure thrust-pr-33 --vpc-service thrust-pg-dev --user pr_33 --origin-connection-limit 60)"
[[ "$rc" == 2 ]] && ! grep -q " -X POST " "$FAKE_CALLS" && grep -q "exceeds the per-PR role's CONNECTION LIMIT of $PRL" "$tmp/out" \
  && ok "control: per-PR ensure with --origin-connection-limit 60 is REFUSED (exit 2, nothing sent, says why)" || bad "control per-PR 60 (rc=$rc)"
rc="$(run ensure thrust-pr-33 --vpc-service thrust-pg-dev --user pr_33 --origin-connection-limit $((PRL + 1)) --dry-run)"
[[ "$rc" == 2 ]] && ok "control: exactly one above the role's limit ($((PRL + 1))) is refused, also under --dry-run" || bad "control per-PR limit+1 (rc=$rc)"
rc="$(run ensure thrust-pr-34 --vpc-service thrust-pg-dev --user pr_34 --origin-connection-limit "0$PRL" --dry-run)"
[[ "$rc" == 0 ]] && ok "a zero-padded value equal to the limit (0$PRL) is parsed as decimal, not an octal error" || bad "control zero-padded (rc=$rc)"
# (d) CONTROL: non-PR roles keep their behaviour — no limit unless given, an explicit 40 / 20 untouched, 60 allowed.
echo '{"success":true,"errors":[],"result":[]}' >"$tmp/list.json"
rc="$(run ensure thrust-plain --vpc-service thrust-pg --user thrust)"
python3 - "$FAKE_POST_BODY" <<'PY' && ok "control: a thrust role with no flag sends NO origin_connection_limit (unchanged)" || bad "thrust role default (rc=$rc)"
import json, sys
assert "origin_connection_limit" not in json.load(open(sys.argv[1]))
PY
rc="$(run ensure thrust-forty --vpc-service thrust-pg --user thrust --origin-connection-limit 40)"
python3 - "$FAKE_POST_BODY" <<'PY' && ok "control: a thrust role keeps its documented explicit 40" || bad "thrust role 40 (rc=$rc)"
import json, sys
assert json.load(open(sys.argv[1]))["origin_connection_limit"] == 40
PY
rc="$(run ensure thrust-twenty --vpc-service thrust-pg-dev --user thrust --origin-connection-limit 20)"
python3 - "$FAKE_POST_BODY" <<'PY' && ok "control: thrust-pg-dev's owner role keeps its documented explicit 20" || bad "thrust role 20 (rc=$rc)"
import json, sys
assert json.load(open(sys.argv[1]))["origin_connection_limit"] == 20
PY
rc="$(run ensure thrust-sixty --vpc-service thrust-pg --user thrust --origin-connection-limit 60 --dry-run)"
[[ "$rc" == 0 ]] && ok "control: the per-PR cap does not apply to a thrust role (60 accepted)" || bad "thrust role 60 (rc=$rc)"
for u in pr_ pr_abc prod_1 xpr_1 pr_1x; do
  rc="$(run ensure thrust-notpr --vpc-service thrust-pg-dev --user "$u" --origin-connection-limit 60 --dry-run)"
  [[ "$rc" == 0 ]] && ok "control: '$u' is not a per-PR role name, so the cap does not apply" || bad "non-PR name $u (rc=$rc)"
done
# Hyperdrive's documented floor, for any role.
rc="$(run ensure thrust-low --vpc-service thrust-pg --user thrust --origin-connection-limit 4 --dry-run)"
[[ "$rc" == 2 ]] && grep -q "documented minimum" "$tmp/out" && ok "control: a limit below Hyperdrive's documented minimum of 5 is refused" || bad "control min 5 (rc=$rc)"
rc="$(run ensure thrust-low --vpc-service thrust-pg --user thrust --origin-connection-limit 5 --dry-run)"
[[ "$rc" == 0 ]] && ok "control: the documented minimum itself (5) is accepted" || bad "control min 5 boundary (rc=$rc)"

echo '{"success":true,"errors":[],"result":[]}' >"$tmp/list.json"
rc="$(run ensure thrust-default --vpc-service thrust-pg --user thrust)"
python3 - "$FAKE_POST_BODY" <<'PY' && ok "ensure DEFAULTS to caching disabled (thrust-pg)" || bad "default caching (rc=$rc)"
import json, sys
assert json.load(open(sys.argv[1]))["caching"] == {"disabled": True}
PY
rc="$(run ensure thrust-dev-default --vpc-service thrust-pg-dev --user thrust)"
python3 - "$FAKE_POST_BODY" <<'PY' && ok "ensure DEFAULTS to caching disabled for thrust-pg-dev too (parity)" || bad "dev default caching (rc=$rc)"
import json, sys
assert json.load(open(sys.argv[1]))["caching"] == {"disabled": True}
PY
rc="$(run ensure thrust-cache-experiment --vpc-service thrust-pg --user thrust --caching-enabled --origin-connection-limit 10)"
python3 - "$FAKE_POST_BODY" <<'PY' && ok "control: --caching-enabled sends NO caching block (Cloudflare default), so spike 0002 can measure it on a separate config" || bad "caching-enabled (rc=$rc)"
import json, sys
b = json.load(open(sys.argv[1]))
assert "caching" not in b and b["origin_connection_limit"] == 10
PY
rc="$(run ensure thrust-default --vpc-service thrust-pg --user thrust --caching-disabled)"
[[ "$rc" == 2 ]] && ok "control: the retired --caching-disabled flag is refused, not silently ignored" || bad "retired flag (rc=$rc)"
rc="$(run ensure thrust-default --vpc-service thrust-pg --user thrust --dry-run)"
grep -q '"disabled": true' "$tmp/out" && ok "ensure --dry-run shows the caching block that would be sent" || bad "dry-run caching (rc=$rc)"

echo "${good//\"thrust-pg-dev\"/\"thrust-pr-12\"}" >"$tmp/list.json"
rc="$(run ensure thrust-pr-12 --vpc-service thrust-pg-dev --user pr_12)"
grep -q " -X POST " "$FAKE_CALLS" && bad "control: ensure POSTed although the config exists" || ok "control: an existing config is never re-created (no POST)"

# --- managed per-PR inventory and deletion ----------------------------------------------------
prconfigs='{"success":true,"errors":[],"result":[
 {"id":"h1","name":"thrust-pg","origin":{"scheme":"postgresql","database":"thrust","user":"thrust","service_id":"vpc-prod"}},
 {"id":"h12","name":"thrust-pr-12","origin":{"scheme":"postgresql","database":"thrust","user":"pr_12","service_id":"vpc-dev"}},
 {"id":"other","name":"unmanaged-pr-12","origin":{"scheme":"postgresql","database":"thrust","user":"thrust","service_id":"vpc-dev"}}]}'
echo "$prconfigs" >"$tmp/list.json"
rc="$(run list-pr)"
[[ "$rc" == 0 ]] && grep -q '"number": 12' "$tmp/out" && ! grep -q 'unmanaged' "$tmp/out" && ok "list-pr exposes only exact managed names" || bad "list-pr inventory (rc=$rc)"

# The non-secret deployment receipt must reject each form of origin drift. Generate different
# PR numbers and independently change one field at a time, including absent and duplicate configs.
python3 - "$script" "$FAKE_LIST" "$PRL" <<'PY' && ok 'verify-pr rejects drift and emits a receipt only for a matching origin' || bad 'verify-pr isolation controls'
import copy, json, os, subprocess, sys
script, fixture, max_limit = sys.argv[1], sys.argv[2], int(sys.argv[3])
for number in (1, 12, 999999999):
    name = f"thrust-pr-{number}"
    item = {"id": "12" * 16, "name": name, "origin": {"scheme": "postgresql", "database": "thrust",
            "user": f"pr_{number}", "service_id": "vpc-dev"}, "caching": {"disabled": True}, "origin_connection_limit": max_limit}
    def run(items, selector=str(number)):
        with open(fixture, "w") as f:
            json.dump({"success": True, "result": items}, f)
        return subprocess.run([script, "verify-pr", selector], capture_output=True, text=True, env=os.environ)
    good = run([item])
    assert good.returncode == 0, good.stderr
    assert json.loads(good.stdout) == {"number": number, "id": item["id"], "name": name}
    cases = []
    for key, value in (("scheme", "mysql"), ("database", "other"), ("user", "pr_999"), ("service_id", "vpc-prod")):
        wrong = copy.deepcopy(item); wrong["origin"][key] = value; cases.append([wrong])
    for key, value in (("id", "0" * 32), ("caching", {}), ("origin_connection_limit", max_limit + 1),
                       ("origin_connection_limit", None), ("origin_connection_limit", True)):
        wrong = copy.deepcopy(item); wrong[key] = value; cases.append([wrong])
    duplicate = copy.deepcopy(item); duplicate["id"] = "34" * 16
    cases += [[], [item, duplicate]]
    for case in cases:
        result = run(case)
        assert result.returncode != 0 and not result.stdout, (number, case, result.stdout, result.stderr)
    for selector in ("0", "01", "1x", "1000000000"):
        result = run([item], selector)
        assert result.returncode != 0 and not result.stdout, selector
PY
echo "$prconfigs" >"$tmp/list.json"
rc="$(run drop-pr 12)"
[[ "$rc" == 0 ]] && grep -q '/hyperdrive/configs/h12' "$FAKE_CALLS" && ok "drop-pr deletes only the exact managed config id" || bad "drop-pr delete (rc=$rc): $(<"$tmp/out")"
echo "$good" >"$tmp/list.json"
rc="$(run drop-pr 12)"
[[ "$rc" == 0 ]] && ! grep -q ' -X DELETE ' "$FAKE_CALLS" && ok "drop-pr treats an absent managed config as idempotent" || bad "drop-pr absent (rc=$rc)"
rc="$(run drop-pr '12;DELETE')"
[[ "$rc" == 2 ]] && ok "control: drop-pr refuses a non-numeric resource selector" || bad "drop-pr selector control (rc=$rc)"

# --- rotate-password ---------------------------------------------------------------------------
echo "$good" >"$tmp/list.json"
export HYPERDRIVE_ORIGIN_PASSWORD="rotated-0123456789-abcdefghijkl"
rc="$(run rotate-password thrust-pg)"
if [[ "$rc" == 0 ]] && grep -q " -X PUT .*/hyperdrive/configs/h1" "$FAKE_CALLS"; then ok "rotate-password PUTs to the id of the named config"; else bad "rotate put (rc=$rc)"; fi
grep -qF "$HYPERDRIVE_ORIGIN_PASSWORD" "$FAKE_CALLS" && bad "rotated password reached argv" || ok "rotated password is not in argv"
python3 - "$FAKE_POST_BODY" "$HYPERDRIVE_ORIGIN_PASSWORD" <<'PY' && ok "PUT body keeps the live service_id/user/database and swaps only the password" || bad "PUT body shape"
import json, sys
b = json.load(open(sys.argv[1]))
o = b["origin"]
assert b["name"] == "thrust-pg" and o["service_id"] == "vpc-prod" and o["user"] == "thrust"
assert o["database"] == "thrust" and o["scheme"] == "postgresql" and o["password"] == sys.argv[2]
PY
python3 - "$FAKE_POST_BODY" <<'PY' && ok "PUT body CARRIES the live caching block and connection limit (a PUT would otherwise reset caching to enabled)" || bad "PUT keeps caching"
import json, sys
b = json.load(open(sys.argv[1]))
assert b["caching"] == {"disabled": True} and b["origin_connection_limit"] == 40
PY
custom='"caching":{"disabled":false,"max_age":30,"stale_while_revalidate":5}'
echo "${good//$off/$custom}" >"$tmp/list.json"
rc="$(run rotate-password thrust-pg)"
python3 - "$FAKE_POST_BODY" <<'PY' && ok "control: an operator-chosen caching block is carried verbatim, not normalised" || bad "PUT carries a custom caching block (rc=$rc)"
import json, sys
assert json.load(open(sys.argv[1]))["caching"] == {"disabled": False, "max_age": 30, "stale_while_revalidate": 5}
PY
echo "$good" >"$tmp/list.json"
rc="$(run rotate-password thrust-pg --dry-run)"
[[ "$rc" == 0 ]] && grep -q '<redacted>' "$tmp/out" && ! grep -q " -X PUT " "$FAKE_CALLS" && ok "rotate-password --dry-run writes nothing" || bad "rotate dry-run (rc=$rc)"
rc="$(run rotate-password no-such-config)"
[[ "$rc" == 1 ]] && ! grep -q " -X PUT " "$FAKE_CALLS" && ok "control: rotating an unknown config -> exit 1, no write" || bad "control unknown config (rc=$rc)"
export HYPERDRIVE_ORIGIN_PASSWORD="$pw"

must_fail() { # label expected-rc cmd...
  local what="$1" want="$2"
  shift 2
  local rc
  rc="$(run "$@")"
  [[ "$rc" == "$want" ]] && ok "control: $what -> exit $want" || bad "control: $what returned $rc, expected $want"
}
must_fail "unknown VPC service" 2 ensure x --vpc-service thrust-pg-staging --user u --dry-run
must_fail "SQL-ish user name" 2 ensure x --vpc-service thrust-pg --user 'a;b' --dry-run
must_fail "non-numeric connection limit" 2 ensure x --vpc-service thrust-pg --user u --origin-connection-limit ten --dry-run
unset HYPERDRIVE_ORIGIN_PASSWORD
must_fail "no password source" 2 ensure x --vpc-service thrust-pg --user u
must_fail "unknown subcommand" 2 frobnicate

# --- store-tunnel-token.sh ---------------------------------------------------------------------
store="$here/store-tunnel-token.sh"
tok='eyJhIjoiMDAwMDAwMDAwMDAwMDAwMCIsInQiOiIwMDAwMDAwMC0wMDAwLTAwMDAtMDAwMC0wMDAwMDAwMDAwMDAifQ=='
uuid='11111111-2222-3333-4444-555555555555'
cat >"$tmp/aws" <<'FAKE'
#!/usr/bin/env bash
echo "$*" >>"$FAKE_AWS_CALLS"
for a in "$@"; do
  case "$a" in file://*) cp "${a#file://}" "$FAKE_AWS_VALUE" ;; esac
done
FAKE
chmod +x "$tmp/aws"
export AWS="$tmp/aws" FAKE_AWS_CALLS="$tmp/aws.log" FAKE_AWS_VALUE="$tmp/aws.value"
sreset() { : >"$FAKE_AWS_CALLS"; : >"$FAKE_CALLS"; rm -f "$FAKE_AWS_VALUE"; }
srun() { sreset; "$store" "$@" >"$tmp/out" 2>&1; echo $?; }

printf '{"success":true,"errors":[],"result":"%s"}' "$tok" >"$tmp/list.json"
rc="$(srun --tunnel-id "$uuid")"
[[ "$rc" == 0 && "$(<"$FAKE_AWS_VALUE")" == "$tok" ]] && ok "store-tunnel-token: the token reaches Secrets Manager byte-for-byte (no trailing newline)" || bad "store token (rc=$rc)"
grep -qF "$tok" "$FAKE_AWS_CALLS" "$FAKE_CALLS" && bad "token appeared in a command line" || ok "token is in no command line (only a file:// path)"
grep -qF "$tok" "$tmp/out" && bad "token was printed" || ok "token is never printed; the script reports its length"
grep -q "secret-id /thrust/cloudflared-tunnel-token" "$FAKE_AWS_CALLS" && ok "stored in /thrust/cloudflared-tunnel-token" || bad "wrong secret id"
grep -qF "tok-SECRET" "$FAKE_CALLS" && bad "Cloudflare API token on a command line" || ok "Cloudflare API token is not on a command line (-H @file)"

echo '{"success":false,"errors":[{"message":"Authentication error"}],"result":null}' >"$tmp/list.json"
rc="$(srun --tunnel-id "$uuid")"
[[ "$rc" == 3 && ! -s "$FAKE_AWS_CALLS" ]] && ok "control: an API failure -> exit 3 and Secrets Manager is NOT touched" || bad "control api failure (rc=$rc)"

echo '{"success":true,"errors":[],"result":"short"}' >"$tmp/list.json"
rc="$(srun --tunnel-id "$uuid")"
[[ "$rc" == 3 && ! -s "$FAKE_AWS_CALLS" ]] && ok "control: an implausibly short token is refused, nothing stored" || bad "control short token (rc=$rc)"

rc="$(srun --tunnel-id "$uuid" --secret-id /agentic-platform/other)"
[[ "$rc" == 2 && ! -s "$FAKE_AWS_CALLS" ]] && ok "control: it refuses to write outside /thrust/*" || bad "control foreign secret id (rc=$rc)"
rc="$(srun --tunnel-id not-a-uuid)"
[[ "$rc" == 2 ]] && ok "control: a malformed tunnel id -> exit 2" || bad "control tunnel id (rc=$rc)"
rc="$(CLOUDFLARE_API_TOKEN='' srun --tunnel-id "$uuid")"
[[ "$rc" == 3 && ! -s "$FAKE_AWS_CALLS" ]] && ok "control: no Cloudflare token -> exit 3" || bad "control no token (rc=$rc)"
printf '{"success":true,"errors":[],"result":"%s"}' "$tok" >"$tmp/list.json"
rc="$(srun --tunnel-id "$uuid" --dry-run)"
[[ "$rc" == 0 && ! -s "$FAKE_CALLS" && ! -s "$FAKE_AWS_CALLS" ]] && ok "--dry-run makes no network call and writes nothing" || bad "dry-run (rc=$rc)"

echo
if ((fail)); then echo "script tests: FAIL"; exit 1; fi
echo "script tests: PASS"
