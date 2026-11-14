#!/bin/sh
# .github/scripts/plan-deploy.sh (issue #49): the decision "does this run deploy, and what" over EVERY combination of the inputs that
# have a finite set of values, with the expected answer written out here as a separate rule, not read from the script.
#
#   sh .github/scripts/tests/test-plan-deploy.sh
#
# PLAN_SCRIPT points the test at a copy of the script, so a deliberately broken copy can be shown to fail it.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
SCRIPT="${PLAN_SCRIPT:-$HERE/.github/scripts/plan-deploy.sh}"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
pass=0; failn=0

GIT_CONFIG_GLOBAL=/dev/null; GIT_CONFIG_SYSTEM=/dev/null
export GIT_CONFIG_GLOBAL GIT_CONFIG_SYSTEM

R="$T/repo"
mkdir -p "$R"
cd "$R" || exit 1
git init -q -b main . || exit 1
git config user.name "plan test"; git config user.email "plan-test@example.invalid"; git config commit.gpgsign false
echo 1 > f; git add -A; git commit -q -m m1; M1="$(git rev-parse HEAD)"
echo 2 > f; git add -A; git commit -q -m m2; M2="$(git rev-parse HEAD)"
git checkout -q -b side "$M1"; echo side > g; git add -A; git commit -q -m s1; S1="$(git rev-parse HEAD)"
git checkout -q main
git update-ref refs/remotes/origin/main "$M2"

# plan <KEY=VALUE>...: the script's stdout and status, in a clean environment
plan() { env -i PATH="$PATH" HOME="$T" GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null "$@" sh "$SCRIPT" 2> "$T/err"; }
field() { echo "$1" | grep "^$2=" | cut -d= -f2-; }
check() { # check <label> <got> <want>
  if [ "$2" = "$3" ]; then pass=$((pass + 1)); else failn=$((failn + 1)); echo "BAD  $1: got '$2', wanted '$3'"; fi
}

REPO=aRustyDev/thrust

# ---- workflow_run: every combination ---------------------------------------------------------------------------------------------
n=0
for conclusion in success failure cancelled skipped ""; do
  for event in push pull_request workflow_dispatch schedule; do
    for branch in main feat/x ""; do
      for head_repo in "$REPO" "someone/thrust" ""; do
        for enabled in true false "" yes; do
          n=$((n + 1))
          out="$(plan EVENT_NAME=workflow_run RUN_CONCLUSION="$conclusion" RUN_EVENT="$event" RUN_BRANCH="$branch" RUN_SHA="$M2" \
            RUN_HEAD_REPO="$head_repo" REPO="$REPO" DEPLOY_ENABLED="$enabled")"; rc=$?
          # the rule, stated here on its own: a successful push to our own main, with the switch on
          want=skip
          if [ "$conclusion" = success ] && [ "$event" = push ] && [ "$branch" = main ] && [ "$head_repo" = "$REPO" ] && [ "$enabled" = true ]; then want=deploy; fi
          check "workflow_run($conclusion,$event,$branch,$head_repo,$enabled) exit" "$rc" 0
          check "workflow_run($conclusion,$event,$branch,$head_repo,$enabled) mode" "$(field "$out" mode)" "$want"
          if [ "$want" = deploy ]; then
            check "workflow_run deploy: environment" "$(field "$out" environment)" staging
            check "workflow_run deploy: sha" "$(field "$out" sha)" "$M2"
            check "workflow_run deploy: no injected failure" "$(field "$out" inject)" false
          else
            check "workflow_run skip($conclusion,$event,$branch,$head_repo,$enabled): no environment" "$(field "$out" environment)" ""
          fi
        done
      done
    done
  done
done
echo "ok   $n workflow_run combinations"

# a workflow_run whose sha is not a sha is a usage error, not a skip and not a deploy
out="$(plan EVENT_NAME=workflow_run RUN_CONCLUSION=success RUN_EVENT=push RUN_BRANCH=main RUN_SHA=notasha RUN_HEAD_REPO="$REPO" REPO="$REPO" DEPLOY_ENABLED=true)"; rc=$?
check "a workflow_run with a bad sha" "$rc" 2

# ---- workflow_dispatch: every combination ---------------------------------------------------------------------------------------------
n=0
for run_ref in refs/heads/main refs/heads/feat/x refs/pull/1/merge ""; do
 for env_name in staging production dev "" Staging; do
  for inject in true false "" yes; do
    for skip_smoke in true false "" yes; do
      for ref in main "$M1" "$M2" side nonsense-ref ""; do
        n=$((n + 1))
        out="$(plan EVENT_NAME=workflow_dispatch RUN_REF="$run_ref" DISPATCH_ENV="$env_name" DISPATCH_INJECT="$inject" DISPATCH_SKIP_SMOKE="$skip_smoke" DISPATCH_REF="$ref")"; rc=$?
        # the rules, stated here on their own
        want_rc=0
        [ "$run_ref" = refs/heads/main ] || want_rc=1
        case "$env_name" in staging|production) ;; *) want_rc=1 ;; esac
        case "$inject" in true|false|"") ;; *) want_rc=1 ;; esac
        case "$skip_smoke" in true|false|"") ;; *) want_rc=1 ;; esac
        [ "$inject" = true ] && [ "$env_name" = production ] && want_rc=1
        [ "$inject" = true ] && [ "$skip_smoke" = true ] && want_rc=1
        case "$ref" in main|"$M1"|"$M2"|"") ;; *) want_rc=1 ;; esac
        label="dispatch(run_ref=$run_ref,env=$env_name,inject=$inject,skip=$skip_smoke,ref=$ref)"
        check "$label exit" "$rc" "$want_rc"
        if [ "$want_rc" = 0 ]; then
          want_sha="$M2"; [ "$ref" = "$M1" ] && want_sha="$M1"
          check "$label mode" "$(field "$out" mode)" deploy
          check "$label environment" "$(field "$out" environment)" "$env_name"
          check "$label sha" "$(field "$out" sha)" "$want_sha"
          check "$label inject" "$(field "$out" inject)" "$([ "$inject" = true ] && echo true || echo false)"
          check "$label skip_smoke" "$(field "$out" skip_smoke)" "$([ "$skip_smoke" = true ] && echo true || echo false)"
        else
          check "$label prints nothing on stdout when refused" "$out" ""
        fi
      done
    done
  done
 done
done
echo "ok   $n workflow_dispatch combinations"

# a manual run needs main to be there to compare with
out="$(plan EVENT_NAME=workflow_dispatch RUN_REF=refs/heads/main DISPATCH_ENV=staging DISPATCH_REF=main MAIN_REF=origin/not-a-branch)"; rc=$?
check "a missing main ref is a usage error, not a pass" "$rc" 2

# ---- the other events ---------------------------------------------------------------------------------------------------------------
out="$(plan EVENT_NAME=pull_request GITHUB_SHA="$M2")"; rc=$?
check "pull_request exit" "$rc" 0
check "pull_request is a dry run" "$(field "$out" mode)" dry-run
check "a dry run has no environment" "$(field "$out" environment)" ""
check "a dry run carries the sha" "$(field "$out" sha)" "$M2"
out="$(plan EVENT_NAME=pull_request GITHUB_SHA=nope)"; rc=$?
check "pull_request with a bad sha" "$rc" 2
for ev in push schedule release issue_comment; do
  out="$(plan EVENT_NAME=$ev GITHUB_SHA="$M2")"; rc=$?
  check "$ev exit" "$rc" 0
  check "$ev does not deploy" "$(field "$out" mode)" skip
done
out="$(plan)"; rc=$?
check "no event is a usage error" "$rc" 2

echo "passed=$pass bad=$failn"
[ "$failn" = 0 ]
