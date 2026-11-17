#!/bin/sh
# Exercise scripts/deploy-edge.sh (ADR-0111, #195 stage E3) against fakes: wrangler holding the edge Worker's versions, a truthful smoke
# and truthful edge controls (edge_fakes.py). The render and the reading of what is live (active-version.py) are the real ones. Every
# case goes through `expect`: the exit status AND a fragment of the output, so a case cannot pass for the wrong reason. The passing
# cases are what give the failing ones meaning.
#
# What it shows:
#   - over a live edge, the deploy tags the version with the commit, sends the three secrets in a file only its owner can read and
#     that is gone afterwards, smokes through the edge (this edge, routed to production, production at the release), checks that the
#     smoke refuses an edge nobody deployed, runs the edge's controls, and changes nothing else;
#   - each secret reaches only the step that needs it, and no secret value is in any argv or in the output;
#   - the first deploy (nothing live) deploys and stops green with the smoke skipped: the edge has no hostname yet;
#   - a new edge that fails the smoke, a smoke that passes anything, or failed controls each roll back to the version that was live and
#     prove it answers again; the run is red every time. A rollback that fails, or that the smoke cannot confirm, says so; without
#     AUTO_ROLLBACK nothing is rolled back; an untagged live version is rolled back to by its id;
#   - a failed deploy, a split or unreadable live edge, a listing that keeps failing, and a render that is refused each stop before or
#     at the deploy, with nothing smoked; a listing that fails once is asked again;
#   - every usage error (each missing variable, a short pin key, the edge named as production, a bad base URL, version or switch, a
#     missing build) deploys nothing and calls nothing.
set -u
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
DEPLOY="$HERE/scripts/deploy-edge.sh"
FAKES="$HERE/.github/scripts/tests/edge_fakes.py"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
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

# ---- the fakes, and the edge's build beside its configuration -------------------------------------------------------------------
mkdir -p "$T/cfg/build/worker"
for f in worker/shim.mjs index.js index_bg.wasm; do echo "// edge build" > "$T/cfg/build/$f"; done
printf '#!/bin/sh\nexec python3 -I "%s" wrangler "$@"\n' "$FAKES" > "$T/wrangler"
printf '#!/bin/sh\nexec python3 -I "%s" smoke "$@"\n' "$FAKES" > "$T/smoke.sh"
printf '#!/bin/sh\nexec python3 -I "%s" controls "$@"\n' "$FAKES" > "$T/controls.sh"
chmod +x "$T/wrangler" "$T/smoke.sh" "$T/controls.sh"

SHA=0123456789abcdef0123456789abcdef01234567
OLD=fedcba9876543210fedcba9876543210fedcba98
PIN_KEY="pin-key-0123456789abcdef0123456789abcdef"
ACCESS_ID="0a1b2c3d4e5f60718293a4b5c6d7e8f9.access"
ACCESS_SECRET="access-secret-5f0e1d2c3b4a59687766554433221100"
CF_TOKEN="cf-tok-7d3e"
SMOKE_PW="smoke-pw-91aa"

# run_deploy [VAR=value ...]: a fresh fake, then the deploy with everything a release's edge deploy has, and the overrides
run_deploy() {
  rm -rf "$T/fake"; mkdir -p "$T/fake"; : > "$T/fake/calls.log"; : > "$T/out"; : > "$T/summary"
  env -i PATH="$PATH" HOME="${HOME:-/tmp}" \
    FAKE_DIR="$T/fake" FAKE_DEPLOY_SHA="$SHA" \
    FAKE_WANT_EDGE_PIN_KEY="$PIN_KEY" FAKE_WANT_PREVIEW_ACCESS_CLIENT_ID="$ACCESS_ID" FAKE_WANT_PREVIEW_ACCESS_CLIENT_SECRET="$ACCESS_SECRET" \
    WRANGLER="$T/wrangler" SMOKE_SCRIPT="$T/smoke.sh" CONTROLS_SCRIPT="$T/controls.sh" \
    DEPLOY_CONFIG_OUT="$T/cfg/.wrangler.deploy.run.toml" GITHUB_OUTPUT="$T/out" GITHUB_STEP_SUMMARY="$T/summary" \
    WRANGLER_READ_ATTEMPTS=2 WRANGLER_READ_RETRY_SECONDS=0 \
    DEPLOY_SHA="$SHA" THRUST_BASE_URL=https://thrust.example.test \
    THRUST_EDGE_WORKER_NAME=thrust-edge THRUST_WORKER_NAME=thrust-production \
    THRUST_FLAGSHIP_APP_ID=01234567-89ab-cdef-0123-456789abcdef \
    THRUST_ACCESS_TEAM_DOMAIN=example.cloudflareaccess.com \
    THRUST_ACCESS_AUD_RC=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    THRUST_ACCESS_AUD_DEV=fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210 \
    THRUST_WORKERS_DEV_SUBDOMAIN=team-4d9 \
    CLOUDFLARE_API_TOKEN="$CF_TOKEN" CLOUDFLARE_ACCOUNT_ID=acct-1 \
    EDGE_PIN_KEY="$PIN_KEY" PREVIEW_ACCESS_CLIENT_ID="$ACCESS_ID" PREVIEW_ACCESS_CLIENT_SECRET="$ACCESS_SECRET" \
    SMOKE_EMAIL=smoke@example.test SMOKE_PASSWORD="$SMOKE_PW" SMOKE_WORKSPACE=smoke-ws \
    "$@" sh "$DEPLOY"
}
calls() { cat "$T/fake/calls.log"; }
has_call() { grep -qF -- "$1" "$T/fake/calls.log"; }
no_call() { ! grep -qF -- "$1" "$T/fake/calls.log"; }
output_is() { grep -qx -- "$1" "$T/out"; }
# the order of the calls that matter, one word each
order() { grep -vE ' SEES |SECRETS-FILE' "$T/fake/calls.log" | sed -E 's/^wrangler deploy SECRETS.*/secrets-read/; s/^wrangler (deployments|versions) list.*/\1/; s/^wrangler deploy .*/deploy/; s/^wrangler rollback .*/rollback/; s/^smoke .*EDGE=not-a-deployed.*/smoke-control/; s/^smoke .*/smoke/; s/^controls .*/controls/' | tr '\n' ' '; }

# ---- over a live edge: the whole deploy -----------------------------------------------------------------------------------------
expect "live edge: deployed and proved" 0 "deployed and proved: $SHA is the edge on https://thrust.example.test" run_deploy FAKE_LIVE="$OLD"
got_order="$(order)"
check "live edge: listings, deploy, smoke, its control, the controls; nothing else (got: $got_order)" \
  test "$got_order" = "deployments versions deploy secrets-read smoke smoke-control controls "
check "live edge: the version is tagged with the commit and named in the message" has_call "deploy -c $T/cfg/.wrangler.deploy.run.toml --tag $SHA --message edge $SHA --secrets-file "
check "live edge: the secrets file held exactly the three secrets, with the values given, readable by its owner only" \
  has_call "deploy SECRETS EDGE_PIN_KEY,PREVIEW_ACCESS_CLIENT_ID,PREVIEW_ACCESS_CLIENT_SECRET MATCH EDGE_PIN_KEY,PREVIEW_ACCESS_CLIENT_ID,PREVIEW_ACCESS_CLIENT_SECRET MODE 0o600"
check "live edge: the secrets file is gone afterwards" sh -c "test -s '$T/fake/secrets-file-path' && ! test -e \"\$(cat '$T/fake/secrets-file-path')\""
check "live edge: the secrets file was already gone when the smoke ran (it lives no longer than the deploy)" \
  sh -c "grep -qx 'smoke SECRETS-FILE gone' '$T/fake/calls.log' && ! grep -q 'SECRETS-FILE present' '$T/fake/calls.log'"
check "live edge: the smoke asked for this edge, routed to production, production at the release, with the read" \
  has_call "smoke https://thrust.example.test $SHA EDGE=$SHA ROUTE=prod AUTH=on"
check "live edge: the smoke's control asked for an edge nobody deployed" has_call "EDGE=not-a-deployed-version-$SHA ROUTE=prod AUTH=off"
check "live edge: the controls asked for this edge" has_call "controls https://thrust.example.test $SHA"
check "live edge: outputs" sh -c "grep -qx 'deployed=true' '$T/out' && grep -qx 'smoke=passed' '$T/out' && grep -qx 'rolled_back=no' '$T/out' && grep -qx 'state=single' '$T/out' && grep -qx 'previous_version=ver-1' '$T/out' && grep -qx 'previous_tag=$OLD' '$T/out'"
check "live edge: no rollback" no_call "rollback"
check "live edge: the summary names the edge" grep -qF "is the edge on https://thrust.example.test" "$T/summary"
check "live edge: the rendered configuration is the edge's, bound to production" \
  sh -c "grep -qx 'name = \"thrust-edge\"' '$T/cfg/.wrangler.deploy.run.toml' && grep -qx 'service = \"thrust-production\"' '$T/cfg/.wrangler.deploy.run.toml'"

# ---- each secret reaches only its step -----------------------------------------------------------------------------------------
check "secrets: wrangler sees the Cloudflare token" has_call "wrangler SEES CLOUDFLARE_API_TOKEN"
for s in EDGE_PIN_KEY PREVIEW_ACCESS_CLIENT_ID PREVIEW_ACCESS_CLIENT_SECRET SMOKE_PASSWORD; do
  check "secrets: wrangler does not see $s (the edge's go in the file)" no_call "wrangler SEES $s"
done
check "secrets: the smoke sees its password" has_call "smoke SEES SMOKE_PASSWORD"
for s in CLOUDFLARE_API_TOKEN EDGE_PIN_KEY PREVIEW_ACCESS_CLIENT_ID PREVIEW_ACCESS_CLIENT_SECRET; do
  check "secrets: the smoke does not see $s" no_call "smoke SEES $s"
done
check "secrets: the controls see no secret" no_call "controls SEES"
for v in "$PIN_KEY" "$ACCESS_ID" "$ACCESS_SECRET" "$CF_TOKEN" "$SMOKE_PW"; do
  check "secrets: a secret value is in no argv and not in the output" sh -c "! grep -qF -- '$v' '$T/fake/calls.log' '$T/last.out' '$T/out' '$T/summary'"
done

# ---- the default production version, and a given one ------------------------------------------------------------------------
expect "production version: given, the smoke asks for it" 0 "deployed and proved" run_deploy FAKE_LIVE="$OLD" PRODUCTION_VERSION=rel-9 FAKE_PROD=rel-9
check "production version: given, the smoke asked for it" has_call "smoke https://thrust.example.test rel-9 EDGE=$SHA"
expect "production version: production not at the release fails the smoke, and rolls back" 1 "the smoke through the edge failed" \
  run_deploy FAKE_LIVE="$OLD" FAKE_PROD=rel-9

# ---- the first deploy ----------------------------------------------------------------------------------------------------------
expect "first deploy: deployed, the smoke skipped, green" 0 "FIRST DEPLOY: thrust-edge had no version" run_deploy FAKE_LIVE=none
check "first deploy: nothing smoked, nothing rolled back" sh -c "! grep -qE '^(smoke|controls) |rollback' '$T/fake/calls.log'"
check "first deploy: deployed with its secrets" has_call "deploy SECRETS EDGE_PIN_KEY,PREVIEW_ACCESS_CLIENT_ID,PREVIEW_ACCESS_CLIENT_SECRET MATCH"
check "first deploy: outputs" sh -c "grep -qx 'state=none' '$T/out' && grep -qx 'deployed=true' '$T/out' && grep -qx 'smoke=skipped' '$T/out' && grep -qx 'rolled_back=no' '$T/out'"

# ---- after the deploy: a failure rolls back ------------------------------------------------------------------------------------
expect "broken edge: the smoke fails, rolled back, red" 1 "the smoke through the edge failed for $SHA" run_deploy FAKE_LIVE="$OLD" FAKE_BROKEN="$SHA"
saw "broken edge: the previous edge answers again" "rolled back: the previous edge ($OLD) answers"
check "broken edge: rolled back to the version that was live" has_call "rollback ver-1 --name thrust-edge --message rollback of $SHA: the smoke through the edge failed for $SHA --yes"
check "broken edge: the rollback's smoke asked for the previous edge, production behind it, without the read" \
  has_call "smoke https://thrust.example.test $SHA EDGE=$OLD ROUTE=prod AUTH=off"
check "broken edge: no control after the smoke failed" sh -c "! grep -qE 'EDGE=not-a-deployed|^controls ' '$T/fake/calls.log'"
check "broken edge: outputs" sh -c "grep -qx 'smoke=failed' '$T/out' && grep -qx 'rolled_back=yes' '$T/out'"
expect "vacuous smoke: its control fails, rolled back, red" 1 "it cannot tell one edge from another" run_deploy FAKE_LIVE="$OLD" FAKE_SMOKE_ANY=1
check "vacuous smoke: rolled back" has_call "rollback ver-1 --name thrust-edge"
expect "controls fail: rolled back, red" 1 "the edge's controls failed (exit 1)" run_deploy FAKE_LIVE="$OLD" FAKE_CONTROLS=fail
check "controls fail: rolled back" has_call "rollback ver-1 --name thrust-edge"
check "controls fail: rolled_back=yes" output_is "rolled_back=yes"
expect "no auto rollback: red, nothing rolled back" 1 "NO AUTOMATIC ROLLBACK" run_deploy FAKE_LIVE="$OLD" FAKE_BROKEN="$SHA" AUTO_ROLLBACK=false
check "no auto rollback: no rollback call" no_call "rollback"
expect "rollback fails: red, and says to roll back by hand" 1 "ROLLBACK FAILED" run_deploy FAKE_LIVE="$OLD" FAKE_BROKEN="$SHA" FAKE_ROLLBACK=fail
check "rollback fails: rolled_back=failed" output_is "rolled_back=failed"
expect "rollback unverified: the previous edge is broken too" 1 "ROLLBACK NOT VERIFIED" run_deploy FAKE_LIVE="$OLD" FAKE_BROKEN="$SHA,$OLD"
check "rollback unverified: rolled_back=unverified" output_is "rolled_back=unverified"
expect "untagged live edge: rolled back to by its id, which it answers with" 1 "rolled back: the previous edge (ver-1) answers" \
  run_deploy FAKE_LIVE=untagged FAKE_BROKEN="$SHA"

# ---- stops before the smoke ------------------------------------------------------------------------------------------------------
expect "deploy fails: red, nothing smoked" 1 "wrangler deploy failed" run_deploy FAKE_LIVE="$OLD" FAKE_DEPLOY=fail
check "deploy fails: no smoke, no rollback" sh -c "! grep -qE '^(smoke|controls) |rollback' '$T/fake/calls.log'"
check "deploy fails: not reported deployed" sh -c "! grep -qx 'deployed=true' '$T/out'"
check "deploy fails: the secrets file wrangler was given is gone all the same" \
  sh -c "test -s '$T/fake/secrets-file-path' && ! test -e \"\$(cat '$T/fake/secrets-file-path')\""
expect "split live edge: nothing deployed" 1 "the live edge is 'split'" run_deploy FAKE_LIVE=split
check "split live edge: no deploy" no_call "deploy -c"
expect "listing keeps failing: nothing deployed" 1 "wrangler could not list the deployments of thrust-edge" run_deploy FAKE_LIVE="$OLD" FAKE_LIST=fail
check "listing keeps failing: asked twice (WRANGLER_READ_ATTEMPTS=2), then stopped" test "$(grep -c '^wrangler deployments list' "$T/fake/calls.log")" = 2
check "listing keeps failing: no deploy" no_call "deploy -c"
expect "listing fails once: asked again, then deployed" 0 "deployed and proved" run_deploy FAKE_LIVE="$OLD" FAKE_LIST=flaky
saw "listing fails once: said so" "answered on try 2 of 2"
expect "render refused: nothing deployed" 1 "the edge's configuration could not be rendered" run_deploy FAKE_LIVE="$OLD" THRUST_ACCESS_AUD_RC=
check "render refused: wrangler not called" test ! -s "$T/fake/calls.log"

# ---- usage errors: nothing called ----------------------------------------------------------------------------------------------
for v in DEPLOY_SHA THRUST_BASE_URL THRUST_EDGE_WORKER_NAME THRUST_WORKER_NAME CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID \
  PREVIEW_ACCESS_CLIENT_ID PREVIEW_ACCESS_CLIENT_SECRET EDGE_PIN_KEY; do
  expect "usage: $v empty" 2 "deploy-edge:" run_deploy FAKE_LIVE="$OLD" "$v="
  check "usage: $v empty: nothing called" test ! -s "$T/fake/calls.log"
done
expect "usage: a pin key of 31 bytes" 2 "EDGE_PIN_KEY is shorter than 32 bytes" run_deploy FAKE_LIVE="$OLD" EDGE_PIN_KEY=0123456789abcdef0123456789abcde \
  FAKE_WANT_EDGE_PIN_KEY=0123456789abcdef0123456789abcde
expect "usage: a pin key of 32 bytes is enough (the boundary)" 0 "deployed and proved" run_deploy FAKE_LIVE="$OLD" \
  EDGE_PIN_KEY=0123456789abcdef0123456789abcdef FAKE_WANT_EDGE_PIN_KEY=0123456789abcdef0123456789abcdef
expect "usage: the edge named as production" 2 "is production's name" run_deploy FAKE_LIVE="$OLD" THRUST_EDGE_WORKER_NAME=thrust-production
expect "usage: a base URL over http" 2 "must start with https://" run_deploy THRUST_BASE_URL=http://thrust.example.test
expect "usage: a base URL with a path" 2 "a host with no path" run_deploy THRUST_BASE_URL=https://thrust.example.test/x
expect "usage: a short sha" 2 "not a 40-digit commit sha" run_deploy DEPLOY_SHA=0123456
expect "usage: a production version with a space" 2 "PRODUCTION_VERSION has a character" run_deploy "PRODUCTION_VERSION=rel 1"
expect "usage: AUTO_ROLLBACK not a boolean" 2 "AUTO_ROLLBACK must be true or false" run_deploy AUTO_ROLLBACK=yes
expect "usage: no read attempts" 2 "WRANGLER_READ_ATTEMPTS" run_deploy WRANGLER_READ_ATTEMPTS=0
mv "$T/cfg/build/index_bg.wasm" "$T/index_bg.wasm.away"
expect "usage: the edge's build missing" 2 "the edge's build is not beside its configuration" run_deploy FAKE_LIVE="$OLD"
check "usage: the build missing: nothing called" test ! -s "$T/fake/calls.log"
mv "$T/index_bg.wasm.away" "$T/cfg/build/index_bg.wasm"

echo "passed=$pass bad=$failn"
[ "$failn" -eq 0 ]
