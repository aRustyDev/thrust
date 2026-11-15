#!/bin/sh
# What should this run of deploy.yml do, and to which commit? (issue #49, ADR-0110 round 29; the rc/* flow, .agents/plans/cicd-review)
#
#   .github/scripts/plan-deploy.sh        (everything comes from the environment)
#
# Prints `key=value` lines, ready for $GITHUB_OUTPUT:
#   mode          deploy    build the commit and deploy it to `environment`
#                 dispatch  start deploy.yml again with workflow_dispatch, ON the rc/* branch `ref`, for commit `sha`: GitHub checks an
#                           environment's branch rule against the run's own ref, and a workflow_run always runs on main
#                 dry-run   build it and check the deploy configuration; touch no environment, use no secret (a pull request)
#                 skip      do nothing
#   environment   staging | production (empty unless mode is deploy or dispatch)
#   sha           the commit (40 hex digits)
#   ref           the branch to dispatch on (mode dispatch only; empty otherwise)
#   inject        true when the smoke is to fail on purpose, to prove the rollback (staging only)
#   skip_smoke    true when the run is asked to deploy without the smoke (the first deploy of an environment only: the composite action
#                 refuses it for any other)
#   reason        one line
#
# The rules, which are the reason this is a script with a test and not an `if:` in YAML:
#   * ci finishing green on a release-candidate pull request (its head an rc/<name> branch of THIS repository, which only the release
#     App and admins can write) dispatches a staging deploy of that commit, only while the repository variable THRUST_DEPLOY_ENABLED
#     is `true`. A run of ci for anything else (a push, main, a feature branch, a fork, a failure) deploys nothing.
#   * a manual run deploys the commit it was run FROM, so the workflow's own code is the code it deploys:
#       staging     from an rc/<name> branch;
#       production  from a v<version> tag whose commit is on main (release-plz tags the merge of an rc pull request).
#     `<name>` has no slash: the rulesets that guard rc/* and v* match one path segment, so rc/a/b is nobody's in particular.
#     `sha`, when given (the dispatch above gives it), must still be the tip: if the branch moved since, the run skips, and the newer
#     commit's own ci run dispatches its own deploy. `inject_smoke_failure` is refused for production, and together with `skip_smoke`
#     (which makes no sense) for any environment.
#   * a pull request is a dry run.
#
# Environment: EVENT_NAME; for workflow_run RUN_CONCLUSION RUN_EVENT RUN_BRANCH RUN_SHA RUN_HEAD_REPO REPO DEPLOY_ENABLED; for
# workflow_dispatch RUN_REF GITHUB_SHA DISPATCH_ENV DISPATCH_SHA DISPATCH_INJECT DISPATCH_SKIP_SMOKE, and MAIN_REF (default
# origin/main) for production; GITHUB_SHA for pull_request.
# Exit status: 0 it planned (possibly `skip`); 1 the request is refused; 2 it was called wrongly.
set -u
LC_ALL=C
export LC_ALL

MAIN_REF="${MAIN_REF:-origin/main}"

refuse() { echo "plan-deploy: refused: $*" >&2; exit 1; }
misuse() { echo "plan-deploy: $*" >&2; exit 2; }
emit() { # emit <mode> <environment> <sha> <ref> <inject> <skip_smoke> <reason>
  printf 'mode=%s\nenvironment=%s\nsha=%s\nref=%s\ninject=%s\nskip_smoke=%s\nreason=%s\n' "$1" "$2" "$3" "$4" "$5" "$6" "$7"
  exit 0
}
skip() { emit skip "" "${2:-}" "" false false "$1"; }
is_sha() { case "$1" in *[!0-9a-f]*|'') return 1 ;; esac; [ "${#1}" -eq 40 ]; }
# one path segment after the prefix, as the rulesets match it
is_rc_branch() { case "$1" in rc/*/*|rc/) return 1 ;; rc/*) return 0 ;; *) return 1 ;; esac; }
is_rc_ref() { case "$1" in refs/heads/*) is_rc_branch "${1#refs/heads/}" ;; *) return 1 ;; esac; }
is_release_tag() { case "$1" in refs/tags/v*/*|refs/tags/v) return 1 ;; refs/tags/v*) return 0 ;; *) return 1 ;; esac; }

[ -n "${EVENT_NAME:-}" ] || misuse "EVENT_NAME is not set"

case "$EVENT_NAME" in
  workflow_run)
    [ "${RUN_CONCLUSION:-}" = success ] || skip "ci did not succeed (conclusion '${RUN_CONCLUSION:-}'): nothing to deploy"
    [ "${RUN_EVENT:-}" = pull_request ] || skip "ci ran for '${RUN_EVENT:-}': only ci's run for a release-candidate pull request deploys"
    is_rc_branch "${RUN_BRANCH:-}" || skip "ci ran for the branch '${RUN_BRANCH:-}', not a release candidate (rc/<name>)"
    [ -n "${RUN_HEAD_REPO:-}" ] && [ "${RUN_HEAD_REPO:-}" = "${REPO:-}" ] \
      || skip "ci ran for '${RUN_HEAD_REPO:-}', not for this repository's own rc/* branch"
    is_sha "${RUN_SHA:-}" || misuse "RUN_SHA '${RUN_SHA:-}' is not a 40-digit commit sha"
    [ "${DEPLOY_ENABLED:-}" = true ] \
      || skip "automatic deploys are off: set the repository variable THRUST_DEPLOY_ENABLED to true when staging works" "$RUN_SHA"
    emit dispatch staging "$RUN_SHA" "$RUN_BRANCH" false false "ci passed on the release candidate $RUN_BRANCH: dispatch its staging deploy on that branch"
    ;;
  workflow_dispatch)
    env_name="${DISPATCH_ENV:-}"
    run_ref="${RUN_REF:-}"
    case "$env_name" in
      staging)
        is_rc_ref "$run_ref" || refuse "staging is deployed from a release-candidate branch (rc/<name>), not from '$run_ref'" ;;
      production)
        is_release_tag "$run_ref" || refuse "production is deployed from a release tag (v<version>), not from '$run_ref'" ;;
      *) refuse "the environment must be 'staging' or 'production', not '$env_name'" ;;
    esac
    inject="${DISPATCH_INJECT:-false}"
    skip_smoke="${DISPATCH_SKIP_SMOKE:-false}"
    case "$inject" in true|false) ;; *) refuse "inject_smoke_failure must be true or false, not '$inject'" ;; esac
    case "$skip_smoke" in true|false) ;; *) refuse "first_deploy_skip_smoke must be true or false, not '$skip_smoke'" ;; esac
    [ "$inject" = true ] && [ "$env_name" = production ] && refuse "inject_smoke_failure is for staging: it makes a deploy fail on purpose, and production is not the place to prove the rollback"
    [ "$inject" = true ] && [ "$skip_smoke" = true ] && refuse "inject_smoke_failure and first_deploy_skip_smoke together: a smoke that is skipped cannot be made to fail"
    sha="${GITHUB_SHA:-}"
    is_sha "$sha" || misuse "GITHUB_SHA '$sha' is not a 40-digit commit sha"
    want="${DISPATCH_SHA:-}"
    [ -z "$want" ] || is_sha "$want" || refuse "sha '$want' is not a 40-digit commit sha"
    if [ "$env_name" = production ]; then
      git rev-parse --verify --quiet "$MAIN_REF^{commit}" > /dev/null || misuse "$MAIN_REF does not exist in this checkout (fetch the history)"
      git merge-base --is-ancestor "$sha" "$MAIN_REF" 2> /dev/null || refuse "$RUN_REF ($sha) is not a commit of main: a release tag marks a merge"
    fi
    [ -z "$want" ] || [ "$want" = "$sha" ] \
      || skip "$RUN_REF has moved on from $want to $sha since this was dispatched: the newer commit's own ci run deploys it" "$sha"
    emit deploy "$env_name" "$sha" "" "$inject" "$skip_smoke" "a manual run from $RUN_REF ($sha) to $env_name"
    ;;
  pull_request)
    is_sha "${GITHUB_SHA:-}" || misuse "GITHUB_SHA '${GITHUB_SHA:-}' is not a 40-digit commit sha"
    emit dry-run "" "$GITHUB_SHA" "" false false "a pull request: build and check the deploy configuration, deploy nothing"
    ;;
  *)
    skip "'$EVENT_NAME' does not deploy"
    ;;
esac
