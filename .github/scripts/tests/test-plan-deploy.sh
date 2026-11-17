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
RC=rc/2026-11-15T07-33-32Z
for conclusion in success failure ""; do
  for event in pull_request push workflow_dispatch ""; do
    for branch in "$RC" rc/manual-1 rc/a/b main feat/x ""; do
      for head_repo in "$REPO" "someone/thrust"; do
        for enabled in true false; do
         for targets in true false ""; do
          for tags in "" v0.1.0 "v0.1.0 other" "v0.1.0 v0.1.1" release-1 v1/x v; do
            n=$((n + 1))
            out="$(plan EVENT_NAME=workflow_run RUN_CONCLUSION="$conclusion" RUN_EVENT="$event" RUN_BRANCH="$branch" RUN_SHA="$M2" \
              RUN_HEAD_REPO="$head_repo" RUN_TAGS="$tags" REPO="$REPO" DEPLOY_ENABLED="$enabled" TARGETS_ENABLED="$targets")"; rc=$?
            # the rules, stated here on their own: ci green on our own code with the switch on, and either a pull request from one of
            # our rc/<name> branches (one segment, as the ruleset matches) -> staging on that branch, or a push to main whose commit
            # has exactly one v<version> tag -> production on that tag. With the targets switch on as well, the candidate goes to the
            # `target` environment instead of staging (ADR-0111); a release is production's either way
            want=skip; want_env=""; want_ref=""
            case "$tags" in v0.1.0|"v0.1.0 other") one_release=v0.1.0 ;; *) one_release="" ;; esac
            if [ "$conclusion" = success ] && [ "$head_repo" = "$REPO" ] && [ "$enabled" = true ]; then
              case "$event:$branch" in
                "pull_request:$RC"|pull_request:rc/manual-1)
                  want=dispatch; want_env=staging; want_ref="$branch"; [ "$targets" = true ] && want_env=target ;;
                push:main) if [ -n "$one_release" ]; then want=dispatch; want_env=production; want_ref="$one_release"; fi ;;
              esac
            fi
            label="workflow_run($conclusion,$event,$branch,$head_repo,$enabled,targets=$targets,tags=$tags)"
            check "$label exit" "$rc" 0
            check "$label mode" "$(field "$out" mode)" "$want"
            check "$label environment" "$(field "$out" environment)" "$want_env"
            check "$label ref" "$(field "$out" ref)" "$want_ref"
            check "$label release (only a deploy names one)" "$(field "$out" release)" ""
            if [ "$want" = dispatch ]; then
              check "$label sha" "$(field "$out" sha)" "$M2"
              check "$label no injected failure" "$(field "$out" inject)" false
            fi
          done
         done
        done
      done
    done
  done
done
echo "ok   $n workflow_run combinations"

# a workflow_run whose sha is not a sha is a usage error, not a skip and not a dispatch
out="$(plan EVENT_NAME=workflow_run RUN_CONCLUSION=success RUN_EVENT=pull_request RUN_BRANCH="$RC" RUN_SHA=notasha RUN_HEAD_REPO="$REPO" REPO="$REPO" DEPLOY_ENABLED=true)"; rc=$?
check "a workflow_run with a bad sha" "$rc" 2
out="$(plan EVENT_NAME=workflow_run RUN_CONCLUSION=success RUN_EVENT=push RUN_BRANCH=main RUN_TAGS=v0.1.0 RUN_SHA=notasha RUN_HEAD_REPO="$REPO" REPO="$REPO" DEPLOY_ENABLED=true)"; rc=$?
check "a tagged push with a bad sha" "$rc" 2
# the reason says which: an untagged push is "not a release", two tags are "more than one" (the job summary is what a person reads)
out="$(plan EVENT_NAME=workflow_run RUN_CONCLUSION=success RUN_EVENT=push RUN_BRANCH=main RUN_TAGS="" RUN_SHA="$M2" RUN_HEAD_REPO="$REPO" REPO="$REPO" DEPLOY_ENABLED=true)"
check "an untagged push: the reason" "$(field "$out" reason | grep -c 'no release tag')" 1
out="$(plan EVENT_NAME=workflow_run RUN_CONCLUSION=success RUN_EVENT=push RUN_BRANCH=main RUN_TAGS="v1 v2" RUN_SHA="$M2" RUN_HEAD_REPO="$REPO" REPO="$REPO" DEPLOY_ENABLED=true)"
check "two release tags: the reason" "$(field "$out" reason | grep -c 'more than one release tag')" 1
# a tag name is split into words and must not be globbed: a '*' in RUN_TAGS is a word, not the names of files
out="$(cd "$T" && touch v9 && plan EVENT_NAME=workflow_run RUN_CONCLUSION=success RUN_EVENT=push RUN_BRANCH=main RUN_TAGS='v*' RUN_SHA="$M2" RUN_HEAD_REPO="$REPO" REPO="$REPO" DEPLOY_ENABLED=true)"; rc=$?
check "RUN_TAGS is not globbed: exit" "$rc" 0
check "RUN_TAGS is not globbed: the tag is the literal word" "$(field "$out" ref)" 'v*' 

# ---- workflow_dispatch: every combination ---------------------------------------------------------------------------------------------
n=0
for run_ref in "refs/heads/$RC" refs/heads/rc/a/b refs/heads/main refs/tags/v0.1.0 refs/tags/v0.1.0/x refs/tags/release-1 refs/pull/1/merge ""; do
 for env_name in staging target production dev ""; do
  for inject in true false yes; do
    for skip_smoke in true false yes; do
      for head in "$M2" "$S1"; do
        for want_sha in "" same other bad; do
          case "$want_sha" in same) dsha="$head" ;; other) dsha="$M1" ;; bad) dsha=nope ;; *) dsha="" ;; esac
          n=$((n + 1))
          out="$(plan EVENT_NAME=workflow_dispatch RUN_REF="$run_ref" GITHUB_SHA="$head" DISPATCH_ENV="$env_name" DISPATCH_SHA="$dsha" \
            DISPATCH_INJECT="$inject" DISPATCH_SKIP_SMOKE="$skip_smoke")"; rc=$?
          # the rules, stated here on their own
          want_rc=0
          case "$env_name" in
            staging|target) [ "$run_ref" = "refs/heads/$RC" ] || want_rc=1 ;;
            production) [ "$run_ref" = refs/tags/v0.1.0 ] || want_rc=1; [ "$head" = "$S1" ] && want_rc=1 ;;
            *) want_rc=1 ;;
          esac
          case "$inject" in true|false) ;; *) want_rc=1 ;; esac
          case "$skip_smoke" in true|false) ;; *) want_rc=1 ;; esac
          [ "$inject" = true ] && [ "$env_name" = production ] && want_rc=1
          [ "$inject" = true ] && [ "$skip_smoke" = true ] && want_rc=1
          [ "$env_name" = target ] && { [ "$inject" = true ] || [ "$skip_smoke" = true ]; } && want_rc=1
          [ "$want_sha" = bad ] && want_rc=1
          label="dispatch(run_ref=$run_ref,env=$env_name,inject=$inject,skip=$skip_smoke,head=$head,sha=$want_sha)"
          check "$label exit" "$rc" "$want_rc"
          if [ "$want_rc" = 0 ] && [ "$want_sha" = other ]; then
            check "$label: the branch moved, so it skips" "$(field "$out" mode)" skip
            check "$label: a skip has no environment" "$(field "$out" environment)" ""
          elif [ "$want_rc" = 0 ]; then
            check "$label mode" "$(field "$out" mode)" deploy
            check "$label environment" "$(field "$out" environment)" "$env_name"
            check "$label sha" "$(field "$out" sha)" "$head"
            check "$label ref" "$(field "$out" ref)" ""
            check "$label release" "$(field "$out" release)" "$([ "$env_name" = production ] && echo v0.1.0)"
            check "$label inject" "$(field "$out" inject)" "$inject"
            check "$label skip_smoke" "$(field "$out" skip_smoke)" "$skip_smoke"
          else
            check "$label prints nothing on stdout when refused" "$out" ""
          fi
        done
      done
    done
  done
 done
done
echo "ok   $n workflow_dispatch combinations"

# an unset inject or skip_smoke is false
out="$(plan EVENT_NAME=workflow_dispatch RUN_REF="refs/heads/$RC" GITHUB_SHA="$M2" DISPATCH_ENV=staging)"; rc=$?
check "unset inputs: exit" "$rc" 0
check "unset inputs: inject is false" "$(field "$out" inject)" false
check "unset inputs: skip_smoke is false" "$(field "$out" skip_smoke)" false
# production needs main to compare with; staging does not look
out="$(plan EVENT_NAME=workflow_dispatch RUN_REF=refs/tags/v0.1.0 GITHUB_SHA="$M2" DISPATCH_ENV=production MAIN_REF=origin/not-a-branch)"; rc=$?
check "production with no main ref is a usage error, not a pass" "$rc" 2
out="$(plan EVENT_NAME=workflow_dispatch RUN_REF="refs/heads/$RC" GITHUB_SHA="$M2" DISPATCH_ENV=staging MAIN_REF=origin/not-a-branch)"; rc=$?
check "staging does not need the main ref" "$rc" 0
# a dispatch whose own commit is not a sha is a usage error
out="$(plan EVENT_NAME=workflow_dispatch RUN_REF="refs/heads/$RC" GITHUB_SHA=nope DISPATCH_ENV=staging)"; rc=$?
check "a dispatch with a bad GITHUB_SHA" "$rc" 2

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
