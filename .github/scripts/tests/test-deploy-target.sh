#!/bin/sh
# Exercise scripts/deploy-target.sh (ADR-0111, #195) against fakes: wrangler's Previews, the opt-in and the smoke (target_fakes.py),
# and the real scripts/flagship.py against the fake Flagship (flagship_fake.py). The render, the derivation (preview-config.py) and
# the reading of wrangler's answer (preview-result.py) are the real ones. Every case goes through `expect`: the exit status AND a
# fragment of the output, so a case cannot pass for the wrong reason. The passing cases are what give the failing ones meaning.
#
# What it shows:
#   - each target kind deploys, with the policy's audience for CI: the Preview exists, its flag holds the value the edge needs
#     (the URL, the deployment id wrangler reported, the commit, the kind, shared data, the expiry), the steps ran in order, the
#     smoke went through the edge with the pin and the route, and its control asked for a version nobody deployed;
#   - each step saw only its own secret;
#   - every failure from the Preview on (wrangler failing half way, another Worker's URL, a binding missing, rooms in the Preview's
#     own namespace, a variable not as given, a bad deployment id, a flag that does not stick, the opt-in, the smoke, a smoke that
#     passes anything) ends with exit 1 and NOTHING left: no flag, no Preview;
#   - a cleanup that fails says what it left;
#   - every usage error (each missing variable, bad names, kinds, audiences the policy does not open to CI, a bad expiry) deploys
#     nothing and calls nothing;
#   - a second deploy of a live target replaces its Preview and its flag.
set -u
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
DEPLOY="$HERE/scripts/deploy-target.sh"
FAKES="$HERE/.github/scripts/tests/target_fakes.py"
FS_FAKE="$HERE/.github/scripts/tests/flagship_fake.py"
T="$(mktemp -d)"
fs_pid=""
cleanup() { [ -n "$fs_pid" ] && kill "$fs_pid" 2>/dev/null; rm -rf "$T"; }
trap cleanup EXIT INT TERM
pass=0; failn=0
expect() { # expect <label> <want-exit> <want-fragment> <cmd...>
  label="$1"; want="$2"; frag="$3"; shift 3
  "$@" > "$T/last.out" 2>&1
  got=$?
  if [ "$got" = "$want" ] && grep -qF -- "$frag" "$T/last.out"; then
    pass=$((pass + 1)); echo "ok   $label"
  else
    failn=$((failn + 1)); echo "BAD  $label: wanted exit $want + '$frag', got $got"; sed 's/^/     | /' "$T/last.out" | tail -25
  fi
}
saw() { # saw <label> <fragment>: the last `expect`'s output has it (read, not rerun: `expect` would overwrite that output)
  if grep -qF -- "$2" "$T/last.out"; then pass=$((pass + 1)); echo "ok   $1"
  else failn=$((failn + 1)); echo "BAD  $1: '$2' is not in the output"; sed 's/^/     | /' "$T/last.out" | tail -15; fi
}
check() { # check <label> <command that must succeed...>
  label="$1"; shift
  if "$@" > "$T/check.out" 2>&1; then pass=$((pass + 1)); echo "ok   $label"
  else failn=$((failn + 1)); echo "BAD  $label"; sed 's/^/     | /' "$T/check.out" | tail -15; fi
}
command -v python3 > /dev/null 2>&1 || { echo "BAD  python3 is needed"; exit 1; }

# ---- the fakes --------------------------------------------------------------------------------------------------------------
mkdir -p "$T/fake" "$T/web" "$T/cfg"
echo '<!doctype html>' > "$T/web/index.html"
printf '#!/bin/sh\nexec python3 -I "%s" wrangler "$@"\n' "$FAKES" > "$T/wrangler"
printf '#!/bin/sh\nexec python3 -I "%s" optin "$@"\n' "$FAKES" > "$T/optin.sh"
printf '#!/bin/sh\nexec python3 -I "%s" smoke "$@"\n' "$FAKES" > "$T/smoke.sh"
chmod +x "$T/wrangler" "$T/optin.sh" "$T/smoke.sh"
FS_STATE="$T/fs.json"
seed_flagship() { # seed_flagship <flags JSON> [modes JSON]
  printf '{"token": "fs-tok-91c", "account": "acct-1", "app": "app-1", "flags": %s, "modes": %s}\n' "$1" "${2:-{\}}" > "$FS_STATE"
  : > "$T/fs.calls"
}
seed_flagship '{}'
python3 -I "$FS_FAKE" "$FS_STATE" "$T/fs.port" "$T/fs.calls" &
fs_pid=$!
i=0; while [ ! -s "$T/fs.port" ] && [ "$i" -lt 100 ]; do i=$((i + 1)); sleep 0.1; done
[ -s "$T/fs.port" ] || { echo "BAD  the fake Flagship did not start"; exit 1; }
FS_BASE="http://127.0.0.1:$(cat "$T/fs.port")/client/v4"

SHA=0123456789abcdef0123456789abcdef01234567
SHA2=fedcba9876543210fedcba9876543210fedcba98
SUFFIX=-thrust-production.team-4d9.workers.dev
reset() { rm -f "$T/fake/state.json"; : > "$T/fake/calls.log"; seed_flagship '{}'; }

# deploy [VAR=value ...]: run deploy-target.sh with the good environment, these changes on top
deploy() {
  env -i PATH="$PATH" HOME="${HOME:-/tmp}" \
    TARGET_NAME=v0.3.0 TARGET_KIND=rc TARGET_AUDIENCE=rc TARGET_EXPIRES=2026-12-01T00:00:00Z DEPLOY_SHA="$SHA" \
    THRUST_BASE_URL=https://thrust.example-shared.dev NEW_ASSETS_DIR="$T/web" \
    THRUST_WORKER_NAME=thrust-production THRUST_HYPERDRIVE_ID=0123456789abcdef0123456789abcdef \
    THRUST_R2_BUCKET=thrust-production-blobs THRUST_QUEUE=thrust-production-events THRUST_QUEUE_DLQ=thrust-production-events-dlq \
    THRUST_RL_SIGNIN_ID=93002 THRUST_RL_V1_ID=93003 THRUST_CSRF_ENFORCE=on THRUST_EMAIL_FROM=no-reply@notify.thrust-staging.dev \
    CLOUDFLARE_API_TOKEN=cf-tok-55a CLOUDFLARE_ACCOUNT_ID=acct-1 FLAGSHIP_API_TOKEN=fs-tok-91c FLAGSHIP_APP_ID=app-1 \
    FLAGSHIP_API_BASE="$FS_BASE" PREVIEW_HOST_SUFFIX="$SUFFIX" EDGE_CI_CLIENT_ID=ci-id-1 EDGE_CI_CLIENT_SECRET=ci-sec-2b \
    SMOKE_EMAIL=smoke@example.test SMOKE_PASSWORD=smoke-pw-77 SMOKE_WORKSPACE=smoke-ws \
    WRANGLER="$T/wrangler" FLAGSHIP_SCRIPT="$FAKES" OPT_IN_SCRIPT="$T/optin.sh" SMOKE_SCRIPT="$T/smoke.sh" \
    DEPLOY_CONFIG_OUT="$T/cfg/target.toml" ASSETS_OUT_DIR="$T/cfg/.assets" \
    FAKE_DIR="$T/fake" FAKE_FLAGSHIP_STATE="$FS_STATE" FAKE_SUBDOMAIN=team-4d9 \
    "$@" sh "$DEPLOY"
}
# nothing_left <name>: no Preview and no flag of that target
nothing_left() {
  python3 -I - "$T/fake/state.json" "$FS_STATE" "$1" <<'EOF'
import json, os, sys
previews = json.load(open(sys.argv[1]))["previews"] if os.path.exists(sys.argv[1]) else {}
flags = json.load(open(sys.argv[2]))["flags"]
label, key = sys.argv[3].replace(".", "-"), "target-" + sys.argv[3].replace(".", "_")
left = [f"the Preview {label}"] * (label in previews) + [f"the flag {key}"] * (key in flags)
print("left:", left)
sys.exit(1 if left else 0)
EOF
}
no_flag_written() { ! grep -q '^flagship put' "$T/fake/calls.log"; }
# deployed_right <name> <kind> <sha>: the Preview exists and the flag holds what the edge needs
deployed_right() {
  python3 -I - "$T/fake/state.json" "$FS_STATE" "$1" "$2" "$3" "$SUFFIX" <<'EOF'
import json, sys
previews, flags = json.load(open(sys.argv[1]))["previews"], json.load(open(sys.argv[2]))["flags"]
name, kind, sha, suffix = sys.argv[3:7]
label = name.replace(".", "-")
p, f = previews[label], flags["target-" + name.replace(".", "_")]
want = {"url": f"https://{label}{suffix}", "deployment": p["deployment"], "release": sha, "kind": kind, "data": "shared",
        "expires": "2026-12-01T00:00:00Z"}
got = f["variations"]["allowed"]
print("flag:", got, "want:", want, "preview tag:", p["tag"])
assert got == want and p["tag"] == sha and f["enabled"] is True and f["default_variation"] == "denied"
EOF
}
# order_and_secrets: the steps ran in order, each with only its own secret
order_and_secrets() {
  python3 -I - "$T/fake/calls.log" <<'EOF'
import sys
lines = open(sys.argv[1]).read().splitlines()
steps = [l.split(" ", 2)[:2] for l in lines if " SEES " not in l]
names = [s[0] + ("-" + s[1] if s[0] in ("wrangler", "flagship") else "") for s in steps]
want = ["wrangler-preview", "flagship-put", "optin", "smoke", "smoke"]
print("steps:", names)
assert names == want, names
sees, current = {}, None
for l in lines:
    tool, rest = l.split(" ", 1)
    if rest.startswith("SEES "):
        sees.setdefault(current, set()).add(rest[5:])
    else:
        current = (tool, len(sees) + sum(1 for _ in []))
        current = tool + ":" + str(lines.index(l))
        sees.setdefault(current, set())
by_tool = {}
for k, v in sees.items():
    by_tool.setdefault(k.split(":")[0], []).append(v)
print("sees:", by_tool)
assert all(v == {"CLOUDFLARE_API_TOKEN"} for v in by_tool["wrangler"])
assert all(v == {"FLAGSHIP_API_TOKEN"} for v in by_tool["flagship"])
assert all(v == {"EDGE_CI_CLIENT_ID", "EDGE_CI_CLIENT_SECRET"} for v in by_tool["optin"])
assert by_tool["smoke"] == [{"SMOKE_PASSWORD"}, set()], by_tool["smoke"]
smokes = [l for l in lines if l.startswith("smoke ") and " SEES " not in l]
assert "route=v0.3.0 auth=on" in smokes[0] and "not-deployed-" in smokes[1] and "auth=off" in smokes[1], smokes
EOF
}

# ---- deploys that pass ------------------------------------------------------------------------------------------------------
for case in "v0.3.0 rc rc" "v0.3.0 release rc" "staging-1 rc dev" "pr-42 dev dev" "a release dev"; do
  # shellcheck disable=SC2086
  set -- $case
  reset
  expect "pass: $1 as $2 through $3" 0 "deploy-target: PASS: $1 is deployed" deploy TARGET_NAME="$1" TARGET_KIND="$2" TARGET_AUDIENCE="$3"
  check "pass: $1 as $2: its Preview and its flag are right" deployed_right "$1" "$2" "$SHA"
  saw "pass: $1 as $2: the outputs" "deployment_id="
done
reset
deploy > "$T/order.out" 2>&1
check "pass: the steps ran in order, each with only its own secret" order_and_secrets
check "pass: the outputs name the target's URL" grep -qF "target_url=https://v0-3-0$SUFFIX" "$T/order.out"

# a second deploy of the live target replaces both
reset
deploy > /dev/null 2>&1
expect "redeploy: a live target deployed again" 0 "deploy-target: PASS" deploy DEPLOY_SHA="$SHA2"
check "redeploy: the Preview and the flag are the new commit's" deployed_right v0.3.0 rc "$SHA2"
check "redeploy: the flag was replaced, not created again" grep -qF "PUT " "$T/fs.calls"

# ---- failures, each leaving nothing -----------------------------------------------------------------------------------------
for mode in "FAKE_PREVIEW=fail|wrangler preview failed" \
  "FAKE_PREVIEW_URL=other|the Preview has no URL https://v0-3-0$SUFFIX" \
  "FAKE_ENV_DROP=HYPERDRIVE|the binding HYPERDRIVE is not in what Cloudflare applied" \
  "FAKE_ENV_DROP=ROOM|the binding ROOM is not in what Cloudflare applied" \
  "FAKE_ENV_DROP=CF_VERSION_METADATA|the binding CF_VERSION_METADATA is not in" \
  "FAKE_ROOM_OWN=1|the binding ROOM: script_name is None, it was given 'thrust-production'" \
  "FAKE_VAR=THRUST_PUBLIC_BASE_URL=https://elsewhere.example|the binding THRUST_PUBLIC_BASE_URL: text is 'https://elsewhere.example'" \
  "FAKE_DEPLOYMENT=Not_An_Id|the deployment id 'Not_An_Id' is not one the edge accepts" \
  "FAKE_OPTIN=fail|the edge did not let CI opt in to v0.3.0" \
  "FAKE_SMOKE=fail|the smoke of v0.3.0 through the edge failed" \
  "FAKE_SMOKE_ANY=1|the smoke's control: asked for a version nobody deployed, the smoke exited 0"; do
  setting="${mode%%|*}"; frag="${mode#*|}"
  reset
  expect "fail: $setting" 1 "$frag" deploy "$setting"
  saw "fail: $setting: the run says it cleaned up" "cleaned=yes"
  check "fail: $setting: no Preview and no flag are left" nothing_left v0.3.0
  # A Preview that is not the one asked for must stop the deploy THERE: the edge must never be pointed at it, not even for a
  # moment before a later step happens to fail.
  case "$setting" in
    FAKE_OPTIN=*|FAKE_SMOKE=*|FAKE_SMOKE_ANY=*) ;;
    *) check "fail: $setting: it stopped before any flag was written" no_flag_written ;;
  esac
done
# a flag that does not stick: the fake Flagship keeps the old one on PUT, so the read-back fails
reset
seed_flagship '{"target-v0_3_0": {"key": "target-v0_3_0", "enabled": true, "default_variation": "denied", "variations": {"denied": {}, "allowed": {}}, "rules": []}}' '{"put_noop": true}'
expect "fail: a flag that does not stick" 1 "the target's flag was not written" deploy
check "fail: a flag that does not stick: no Preview and no flag are left" nothing_left v0.3.0

# a cleanup that fails says so
reset
expect "fail: a cleanup that fails says what it left" 1 "LEFT BEHIND, remove by hand:  the Preview v0-3-0" deploy FAKE_SMOKE=fail FAKE_DELETE=fail
saw "fail: a cleanup that fails: cleaned=failed" "cleaned=failed"

# ---- usage errors: nothing deployed, nothing called -------------------------------------------------------------------------
usage_case() { # usage_case <label> <fragment> <VAR=value...>
  label="$1"; frag="$2"; shift 2
  reset
  expect "usage: $label" 2 "$frag" deploy "$@"
  check "usage: $label: nothing was called" test ! -s "$T/fake/calls.log"
}
for v in TARGET_NAME TARGET_KIND TARGET_AUDIENCE TARGET_EXPIRES DEPLOY_SHA THRUST_BASE_URL NEW_ASSETS_DIR CLOUDFLARE_API_TOKEN \
  CLOUDFLARE_ACCOUNT_ID FLAGSHIP_API_TOKEN FLAGSHIP_APP_ID PREVIEW_HOST_SUFFIX EDGE_CI_CLIENT_ID EDGE_CI_CLIENT_SECRET; do
  usage_case "$v missing" "$v is not set" "$v="
done
usage_case "a name starting with a capital" "starts with a lowercase letter or a digit" TARGET_NAME=V0.3.0
usage_case "a name with a capital in it" "TARGET_NAME is lowercase" TARGET_NAME=v0.A
usage_case "a name with _" "TARGET_NAME is lowercase" TARGET_NAME=a_b
usage_case "a name starting with -" "starts with a lowercase letter or a digit" TARGET_NAME=-x
usage_case "a name ending with ." "may not end with" TARGET_NAME=v0.
usage_case "a short sha" "DEPLOY_SHA is a commit" DEPLOY_SHA=0123abc
usage_case "a sha in capitals" "DEPLOY_SHA is a commit" DEPLOY_SHA=0123456789ABCDEF0123456789abcdef01234567
usage_case "an http origin" "THRUST_BASE_URL is https" THRUST_BASE_URL=http://thrust.example-shared.dev
usage_case "a kind the policy lacks" "is not one of the policy's target kinds" TARGET_KIND=qa
usage_case "an audience the policy does not open to CI for dev" "does not let CI reach a 'dev' target through 'rc'" TARGET_KIND=dev TARGET_AUDIENCE=rc
usage_case "an expiry that is not a time" "TARGET_EXPIRES 'tomorrow'" TARGET_EXPIRES=tomorrow
usage_case "an expiry that is not a date" "TARGET_EXPIRES '2026-13-40T00:00:00Z'" TARGET_EXPIRES=2026-13-40T00:00:00Z
mkdir -p "$T/noweb"
usage_case "web files without index.html" "NEW_ASSETS_DIR has no index.html" NEW_ASSETS_DIR="$T/noweb"
usage_case "production's configuration cannot render" "could not be rendered" THRUST_HYPERDRIVE_ID=00000000000000000000000000000000

echo "passed=$pass bad=$failn"
[ "$failn" -eq 0 ]
