#!/bin/sh
# scripts/deploy-worker.sh against a fake Cloudflare that keeps state (issue #49): the release-engineering controls of ADR-0110 round 29,
# run before there is a staging to run them on. The fakes (deploy_fakes.py) model what the edge SERVES, and the fake smoke is truthful
# about it, so a rollback is verified only when it really made the previous version answer, and the deploy's own control (the smoke must
# refuse a version nobody deployed) passes or fails for the real reason.
#
# What each case shows is named in its label. The ones that matter most:
#   * a failing smoke ends with the PREVIOUS version serving and the run RED (issue #49 acceptance 1), including the failure an operator
#     injects on purpose;
#   * a smoke that accepts any version is caught by the control, and the run is rolled back and red (acceptance 2);
#   * the first deploy, a Durable Object lifecycle change and a live state nobody can read STOP at the manual gate, deploy nothing and
#     never roll back by themselves (acceptance 3);
#   * each secret reaches only the process that needs it.
#
#   sh .github/scripts/tests/test-deploy-worker.sh
#
# DEPLOY_WORKER_SCRIPT points the test at a copy of scripts/deploy-worker.sh, so a deliberately broken copy can be shown to fail it.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
SCRIPT_UNDER_TEST="${DEPLOY_WORKER_SCRIPT:-$HERE/scripts/deploy-worker.sh}"
FAKES="$HERE/.github/scripts/tests/deploy_fakes.py"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
pass=0; failn=0

GIT_CONFIG_GLOBAL=/dev/null; GIT_CONFIG_SYSTEM=/dev/null
export GIT_CONFIG_GLOBAL GIT_CONFIG_SYSTEM

# ---- a repository with the scripts under test, the real template, and a small history ------------------------------------------
R="$T/repo"
mkdir -p "$R/scripts" "$R/.github/scripts" "$R/crates/thrust-worker" "$R/crates/thrust-adapters-postgres/migrations"
cp "$SCRIPT_UNDER_TEST" "$R/scripts/deploy-worker.sh"
cp "$HERE/scripts/render-wrangler.sh" "$R/scripts/render-wrangler.sh"
cp "$HERE/.github/scripts/classify-deploy.sh" "$HERE/.github/scripts/active-version.py" "$R/.github/scripts/"
cp "$HERE/crates/thrust-worker/wrangler.deploy.toml" "$R/crates/thrust-worker/wrangler.deploy.toml"
cd "$R" || exit 1
git init -q -b main . || exit 1
git config user.name "deploy test"; git config user.email "deploy-test@example.invalid"; git config commit.gpgsign false
commit() { git add -A && git commit -q -m "$1" && git rev-parse HEAD; }
echo "CREATE TABLE a (id int);" > crates/thrust-adapters-postgres/migrations/0001.sql
H0="$(commit c0-base)"
echo notes > README.md;                                                                   H1="$(commit c1-other)"
echo "CREATE TABLE b (id int);" > crates/thrust-adapters-postgres/migrations/0002.sql;      H2="$(commit c2-migrations)"
printf '\n[[migrations]]\ntag = "v2"\nnew_sqlite_classes = ["More"]\n' >> crates/thrust-worker/wrangler.deploy.toml
H3="$(commit c3-durable-object-lifecycle)"
echo more >> README.md;                                                                   H4="$(commit c4-other)"
git checkout -q -b side "$H1"; echo side > side.txt; HS="$(commit side)"; git checkout -q main

# ---- the fakes ---------------------------------------------------------------------------------------------------------------------
mkdir -p "$T/bin"
for tool in wrangler smoke; do
  printf '#!/bin/sh\nexec python3 "%s" %s "$@"\n' "$FAKES" "$tool" > "$T/bin/$tool"
done
printf '#!/bin/sh\nexec python3 "%s" migrate "$@"\n' "$FAKES" > "$T/bin/migrate"
chmod +x "$T/bin/wrangler" "$T/bin/smoke" "$T/bin/migrate"
FAKE="$T/fake"
mkdir -p "$FAKE"

seed_live() { # seed_live <tag or empty>: the Worker exists, one version, live and served
  rm -f "$FAKE/state.json" "$FAKE/calls.log"
  python3 - "$FAKE/state.json" "$1" <<'EOF'
import json, sys
tag = sys.argv[2] or None
json.dump({"versions": [{"id": "ver-1", "tag": tag}],
           "deployments": [{"id": "dep-1", "created_on": "2026-11-14T10:00:00Z", "version": "ver-1"}],
           "serving": "ver-1"}, open(sys.argv[1], "w"))
EOF
}
seed_none() { rm -f "$FAKE/state.json" "$FAKE/calls.log"; }

GOOD="THRUST_BASE_URL=https://staging.example.test THRUST_WORKER_NAME=thrust-staging THRUST_HYPERDRIVE_ID=0123456789abcdef0123456789abcdef
THRUST_R2_BUCKET=thrust-staging-blobs THRUST_QUEUE=thrust-staging-events THRUST_QUEUE_DLQ=thrust-staging-events-dlq
THRUST_RL_SIGNIN_ID=2102 THRUST_RL_V1_ID=2103 THRUST_CSRF_ENFORCE=off
CLOUDFLARE_API_TOKEN=cf-secret-token CLOUDFLARE_ACCOUNT_ID=acct123 DATABASE_URL=postgres://u:pw@127.0.0.1:15432/thrust
TUNNEL_SERVICE_TOKEN_ID=id.access TUNNEL_SERVICE_TOKEN_SECRET=tunnel-secret SMOKE_PASSWORD=smoke-password
WRANGLER=$T/bin/wrangler SMOKE_SCRIPT=$T/bin/smoke MIGRATE_CI_SCRIPT=$T/bin/migrate DEPLOY_CONFIG_OUT=$T/rendered.toml FAKE_DIR=$FAKE"

# deploy <sha> [VAR=value ...]: run the script in a clean environment; RC is its status, $T/log its output, $T/outputs its outputs
deploy() {
  sha="$1"; shift
  rm -f "$FAKE/calls.log" "$T/outputs" "$T/summary.md" "$T/rendered.toml"
  # shellcheck disable=SC2086
  env -i PATH="$PATH" HOME="$T" GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null $GOOD DEPLOY_ENV=staging DEPLOY_SHA="$sha" \
    GITHUB_OUTPUT="$T/outputs" GITHUB_STEP_SUMMARY="$T/summary.md" "$@" sh "$R/scripts/deploy-worker.sh" > "$T/log" 2>&1
  RC=$?
}
out_of() { grep "^$1=" "$T/outputs" 2> /dev/null | tail -1 | cut -d= -f2-; }
calls() { cat "$FAKE/calls.log" 2> /dev/null; }
check() { # check <label> <got> <want>
  if [ "$2" = "$3" ]; then pass=$((pass + 1)); else failn=$((failn + 1)); echo "BAD  $1: got '$2', wanted '$3'"; fi
}
has_call() { calls | grep -qE "$1"; }
check_call() { # check_call <label> <regex> <yes|no>
  if has_call "$2"; then got=yes; else got=no; fi
  check "$1" "$got" "$3"
}
serving() { python3 -c 'import json,sys; s=json.load(open(sys.argv[1])); print([v for v in s["versions"] if v["id"]==s["serving"]][0].get("tag") or s["serving"])' "$FAKE/state.json"; }
dump() { echo "     ---- output:"; sed 's/^/     | /' "$T/log"; echo "     ---- calls:"; calls | sed 's/^/     | /'; }
# the secrets: each may be seen by exactly these processes and no others
secrets_scoped() { # secrets_scoped <label>
  bad=""
  for line in $(calls | grep ' SEES ' | tr ' ' ':'); do
    case "$line" in
      wrangler:SEES:CLOUDFLARE_API_TOKEN) ;;
      migrate:SEES:DATABASE_URL|migrate:SEES:TUNNEL_SERVICE_TOKEN_ID|migrate:SEES:TUNNEL_SERVICE_TOKEN_SECRET) ;;
      smoke:SEES:SMOKE_PASSWORD) ;;
      *) bad="$bad $line" ;;
    esac
  done
  check "$1: no secret reached a process that does not need it" "$bad" ""
}

# =================================================================================================================================
# A. an ordinary deploy: nothing but code changed
seed_live "$H3"
deploy "$H4"
check "ordinary deploy: exit" "$RC" 0; [ "$RC" = 0 ] || dump
check "ordinary deploy: deployed" "$(out_of deployed)" true
check "ordinary deploy: not deferred" "$(out_of deferred)" false
check "ordinary deploy: no rollback" "$(out_of rolled_back)" no
check "ordinary deploy: the new version is served" "$(serving)" "$H4"
check_call "ordinary deploy: tagged with the commit" "wrangler deploy tag=$H4" yes
check_call "ordinary deploy: the real smoke asked for the new version" "smoke expected=$H4 auth=on" yes
check_call "ordinary deploy: the control asked for a version nobody deployed" "smoke expected=not-a-deployed-version-$H4 auth=off" yes
check_call "ordinary deploy: no expand migration" "migrate RUN" no
check_call "ordinary deploy: no rollback call" "wrangler rollback" no
secrets_scoped "ordinary deploy"
check "ordinary deploy: the rendered configuration has no marker left" "$(grep -c '__[A-Z]' "$T/rendered.toml")" 0

# B. migration files changed: the expand step runs, and BEFORE the deploy
seed_live "$H1"
deploy "$H2"
check "migrations changed: exit" "$RC" 0
check_call "migrations changed: the expand step ran" "migrate RUN" yes
check "migrations changed: it ran before the deploy" "$(calls | grep -nE 'migrate RUN|wrangler deploy tag=' | head -1 | grep -c migrate)" 1
check "migrations changed: reported" "$(out_of migrations_changed)" true
secrets_scoped "migrations changed"

# C. the expand step fails: nothing is deployed, and the run is red
seed_live "$H1"
deploy "$H2" FAKE_MIGRATE=fail
check "expand step fails: red" "$RC" 1
check_call "expand step fails: nothing deployed" "wrangler deploy tag=" no
check "expand step fails: the live version is untouched" "$(serving)" "$H1"

# D. the smoke fails because the new version never answers (a stale edge): rollback, and the previous version is serving
seed_live "$H3"
deploy "$H4" FAKE_SERVE_STICKY=1
check "smoke fails: the run is red" "$RC" 1
check "smoke fails: rolled back" "$(out_of rolled_back)" yes
check_call "smoke fails: rolled back to the previous version id" "wrangler rollback id=ver-1" yes
check_call "smoke fails: the probe after the rollback asked for the previous version, without the authenticated phase" "smoke expected=$H3 auth=off" yes
check "smoke fails: the previous version is the one serving" "$(serving)" "$H3"

# E. the smoke's real read fails: the app is broken (header right, data wrong)
seed_live "$H3"
deploy "$H4" FAKE_BAD_TAGS="$H4"
check "broken app: the run is red" "$RC" 1
check "broken app: rolled back" "$(out_of rolled_back)" yes
check "broken app: the previous version is serving again" "$(serving)" "$H3"

# F. an injected failure (staging, by hand): every real check passes, then it fails on purpose, and the rollback runs
seed_live "$H3"
deploy "$H4" INJECT_SMOKE_FAILURE=true
check "injected failure: the run is red" "$RC" 1
check "injected failure: rolled back" "$(out_of rolled_back)" yes
check "injected failure: the previous version is serving, and it is the one the job ended on" "$(serving)" "$H3"
check_call "injected failure: the smoke was told to fail" "smoke expected=$H4 auth=on force=1" yes
check_call "injected failure: the control did not run (the real smoke had already failed)" "not-a-deployed-version" no

# G. the control: a smoke that accepts any version proves nothing, and the run must fail on it and roll back
seed_live "$H3"
deploy "$H4" FAKE_SMOKE_ANY=1
check "a smoke that accepts anything: the run is red" "$RC" 1
check "a smoke that accepts anything: rolled back" "$(out_of rolled_back)" yes
check_call "a smoke that accepts anything: it was asked for a version nobody deployed" "not-a-deployed-version" yes
grep -q "cannot tell one version from another" "$T/log" && pass=$((pass + 1)) || { failn=$((failn + 1)); echo "BAD  the control's failure does not say why"; }

# G2. a smoke that cannot run (exit 2) is not a pass of the control either
seed_live "$H3"
deploy "$H4" FAKE_SMOKE_USAGE=1
check "a smoke called wrongly: the run is red" "$RC" 1

# H. the rollback itself fails: loud, red, and the output says so
seed_live "$H3"
deploy "$H4" FAKE_SERVE_STICKY=1 FAKE_ROLLBACK=fail
check "rollback fails: red" "$RC" 1
check "rollback fails: reported" "$(out_of rolled_back)" failed
grep -q "ROLLBACK FAILED" "$T/log" && pass=$((pass + 1)) || { failn=$((failn + 1)); echo "BAD  a failed rollback is not shouted"; }

# H2. the rollback "succeeds" and the previous version does not answer: not verified, red
seed_live "$H3"
deploy "$H4" FAKE_SERVE_STICKY=1 FAKE_ROLLBACK_NOOP=1 FAKE_BAD_TAGS="x"
check "an unverified rollback: red" "$RC" 1
check "an unverified rollback: reported" "$(out_of rolled_back)" yes
# (the sticky edge still serves the previous version, so here the probe passes: the control below makes it fail for real)
seed_live "$H3"
deploy "$H4" FAKE_ROLLBACK_NOOP=1 FAKE_BAD_TAGS="$H4"
check "a rollback that changes nothing: red" "$RC" 1
check "a rollback that changes nothing is NOT reported as done" "$(out_of rolled_back)" unverified
check "a rollback that changes nothing: the bad version is still serving, and the job says it did not verify" "$(serving)" "$H4"

# I. a Durable Object lifecycle change stops at the manual gate: nothing deployed, nothing migrated
seed_live "$H2"
deploy "$H3"
check "lifecycle change without the gate: exit 0 (it is a deferral, not a failure)" "$RC" 0
check "lifecycle change without the gate: deferred" "$(out_of deferred)" true
check "lifecycle change without the gate: not deployed" "$(out_of deployed)" false
check_call "lifecycle change without the gate: no deploy" "wrangler deploy tag=" no
check_call "lifecycle change without the gate: no expand step" "migrate RUN" no
check_call "lifecycle change without the gate: no smoke" "smoke expected" no
check "lifecycle change without the gate: the live version is untouched" "$(serving)" "$H2"
# ... and through the gate it deploys, and never rolls back by itself, even when the smoke fails
seed_live "$H2"
deploy "$H3" ALLOW_DO_LIFECYCLE=true
check "lifecycle change through the gate: exit" "$RC" 0
check "lifecycle change through the gate: deployed" "$(out_of deployed)" true
check "lifecycle change through the gate: not deferred" "$(out_of deferred)" false
seed_live "$H2"
deploy "$H3" ALLOW_DO_LIFECYCLE=true FAKE_BAD_TAGS="$H3"
check "lifecycle change, smoke fails: red" "$RC" 1
check "lifecycle change, smoke fails: NO automatic rollback" "$(out_of rolled_back)" no
check_call "lifecycle change, smoke fails: no rollback call" "wrangler rollback" no
grep -q "NO AUTOMATIC ROLLBACK" "$T/log" && pass=$((pass + 1)) || { failn=$((failn + 1)); echo "BAD  the missing rollback is not said"; }

# J. the first deploy: no live version, so it is a lifecycle change and a migration, and it stops at the gate
seed_none
deploy "$H4"
check "first deploy without the gate: exit 0" "$RC" 0
check "first deploy without the gate: deferred" "$(out_of deferred)" true
check_call "first deploy without the gate: nothing deployed" "wrangler deploy tag=" no
check "first deploy without the gate: the state is none" "$(out_of state)" none
seed_none
deploy "$H4" ALLOW_DO_LIFECYCLE=true
check "first deploy through the gate: exit" "$RC" 0; [ "$RC" = 0 ] || dump
check "first deploy through the gate: the schema is made first" "$(calls | grep -nE 'migrate RUN|wrangler deploy tag=' | head -1 | grep -c migrate)" 1
check "first deploy through the gate: no rollback is possible, and none is attempted" "$(out_of rolled_back)" no
check_call "first deploy through the gate: the smoke ran" "smoke expected=$H4 auth=on" yes
seed_none
deploy "$H4" ALLOW_DO_LIFECYCLE=true SKIP_SMOKE=true
check "first deploy, smoke skipped (no hostname yet): exit" "$RC" 0
check_call "first deploy, smoke skipped: no smoke call at all" "smoke expected" no
check "first deploy, smoke skipped: deployed" "$(out_of deployed)" true
grep -q "SMOKE SKIPPED" "$T/log" && pass=$((pass + 1)) || { failn=$((failn + 1)); echo "BAD  a skipped smoke is not said loudly"; }
seed_none
deploy "$H4" ALLOW_DO_LIFECYCLE=true FAKE_BAD_TAGS="$H4"
check "first deploy, smoke fails: red" "$RC" 1
check "first deploy, smoke fails: NO rollback (there is nothing to go back to)" "$(out_of rolled_back)" no

# K. SKIP_SMOKE is for the first deploy and no other, and is refused before anything happens (even before a wait for approval)
seed_live "$H3"
deploy "$H4" SKIP_SMOKE=true
check "skip_smoke on an existing Worker: red" "$RC" 1
check_call "skip_smoke on an existing Worker: nothing deployed" "wrangler deploy tag=" no
seed_live "$H2"
deploy "$H3" SKIP_SMOKE=true
check "skip_smoke on an existing Worker with a lifecycle change: refused, not deferred" "$RC" 1
check "skip_smoke on an existing Worker with a lifecycle change: no deferral output" "$(out_of deferred)" ""

# L. a live state nobody can read: manual gate, no rollback
seed_live "$H3"
deploy "$H4" FAKE_DEPLOYMENTS_RAW='[{"id":"d","created_on":"2026-11-14T10:00:00Z","versions":[{"version_id":"ver-1","percentage":60},{"version_id":"ver-2","percentage":40}]}]'
check "a split live state: deferred, not deployed" "$(out_of deferred)/$(out_of deployed)" "true/false"
seed_live "$H3"
deploy "$H4" FAKE_DEPLOYMENTS_RAW='something that is not JSON'
check "an unreadable live state: deferred, not deployed" "$(out_of deferred)/$(out_of deployed)" "true/false"
seed_live "$H3"
deploy "$H4" ALLOW_DO_LIFECYCLE=true FAKE_DEPLOYMENTS_RAW='something that is not JSON' FAKE_BAD_TAGS="$H4"
check "an unreadable live state, through the gate, smoke fails: red, no rollback" "$RC/$(out_of rolled_back)" "1/no"

# M. a version with no tag, or one that is not a commit here, counts as a change and goes through the gate
seed_live ""
deploy "$H4"
check "an untagged live version: deferred" "$(out_of deferred)" true
seed_live "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
deploy "$H4"
check "a live tag that is not in this history: deferred" "$(out_of deferred)" true
seed_live "$HS"
deploy "$H4"
check "a live commit that is not an ancestor: deferred" "$(out_of deferred)" true

# N. wrangler cannot list: nothing is deployed
seed_live "$H3"
deploy "$H4" FAKE_LIST_ERROR=1
check "wrangler cannot list deployments: red" "$RC" 1
check_call "wrangler cannot list deployments: nothing deployed" "wrangler deploy tag=" no
check_call "wrangler cannot list deployments: it is not mistaken for a Worker that does not exist" "migrate RUN" no

# O. a deploy that fails changes nothing and does not roll back
seed_live "$H3"
deploy "$H4" FAKE_DEPLOY=fail
check "wrangler deploy fails: red" "$RC" 1
check "wrangler deploy fails: not reported as deployed" "$(out_of deployed)" ""
check_call "wrangler deploy fails: no rollback (nothing new was live)" "wrangler rollback" no
check "wrangler deploy fails: the previous version still serves" "$(serving)" "$H3"

# P. automatic rollback switched off: red, no rollback, and the way back is printed
seed_live "$H3"
deploy "$H4" AUTO_ROLLBACK=false FAKE_BAD_TAGS="$H4"
check "auto-rollback off, smoke fails: red" "$RC" 1
check "auto-rollback off, smoke fails: no rollback" "$(out_of rolled_back)" no
grep -q "wrangler rollback ver-1" "$T/log" && pass=$((pass + 1)) || { failn=$((failn + 1)); echo "BAD  the manual way back is not printed"; }

# Q. refusals before anything is touched
seed_live "$H3"
deploy "$H4" DEPLOY_ENV=production INJECT_SMOKE_FAILURE=true
check "an injected failure in production: refused (exit 2)" "$RC" 2
check_call "an injected failure in production: nothing was called" "wrangler" no
deploy "$H4" INJECT_SMOKE_FAILURE=true SKIP_SMOKE=true
check "inject and skip together: refused" "$RC" 2
deploy "$H4" DEPLOY_ENV=dev
check "an unknown environment: refused" "$RC" 2
deploy "$H4" THRUST_BASE_URL=http://staging.example.test
check "a base URL that is not https: refused" "$RC" 2
deploy "$H4" THRUST_BASE_URL=https://staging.example.test/path
check "a base URL with a path: refused" "$RC" 2
deploy "not-a-sha"
check "a sha that is not a sha: refused" "$RC" 2
deploy "$H4" AUTO_ROLLBACK=maybe
check "a flag that is not true or false: refused" "$RC" 2
deploy "$H4" CLOUDFLARE_API_TOKEN=
check "no Cloudflare token: refused" "$RC" 2
check_call "no Cloudflare token: nothing was called" "wrangler" no

# R. a variable of the environment is missing: the renderer refuses, and nothing else happens (not even a look at what is live)
seed_live "$H3"
deploy "$H4" THRUST_HYPERDRIVE_ID=
check "a missing Hyperdrive id: red" "$RC" 1
check_call "a missing Hyperdrive id: wrangler was never called" "wrangler" no
grep -q "THRUST_HYPERDRIVE_ID is not set" "$T/log" && pass=$((pass + 1)) || { failn=$((failn + 1)); echo "BAD  the missing variable is not named"; }
deploy "$H4" THRUST_HYPERDRIVE_ID=00000000000000000000000000000000
check "an all-zero Hyperdrive id: red, wrangler never called" "$RC/$(calls | grep -c wrangler)" "1/0"

echo "passed=$pass bad=$failn"
[ "$failn" = 0 ]
