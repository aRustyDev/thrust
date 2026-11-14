#!/bin/sh
# What should this run of deploy.yml do, and to which commit? (issue #49, ADR-0110 round 29)
#
#   .github/scripts/plan-deploy.sh        (everything comes from the environment)
#
# Prints `key=value` lines, ready for $GITHUB_OUTPUT:
#   mode          deploy    build the commit and deploy it to `environment`
#                 dry-run   build it and check the deploy configuration; touch no environment, use no secret (a pull request)
#                 skip      do nothing (a workflow_run that is not a successful push to main, or automatic deploys switched off)
#   environment   staging | production (empty unless mode is deploy)
#   sha           the commit (40 hex digits)
#   inject        true when the smoke is to fail on purpose, to prove the rollback (staging only)
#   skip_smoke    true when the run is asked to deploy without the smoke (the first deploy of an environment only: the composite action
#                 refuses it for any other)
#   reason        one line
#
# The rules, which are the reason this is a script with a test and not an `if:` in YAML:
#   * a push to main deploys STAGING, after `ci` succeeded for that push, and only when the repository variable
#     THRUST_DEPLOY_ENABLED is `true` (the switch that keeps the workflow quiet until an environment exists). A run of `ci` for a pull
#     request, a fork, another branch or a failure deploys nothing.
#   * a manual run names the environment and a ref, and must itself be run FROM main (RUN_REF is refs/heads/main), so the workflow's own
#     code is main's. The ref it deploys must be a commit of main (an ancestor of it, or main itself): a branch's code never reaches an
#     environment's secrets. `inject_smoke_failure` is refused for production, and together with `skip_smoke` (which makes no sense)
#     for any environment.
#   * a pull request is a dry run.
#
# Environment: EVENT_NAME; for workflow_run RUN_CONCLUSION RUN_EVENT RUN_BRANCH RUN_SHA RUN_HEAD_REPO REPO DEPLOY_ENABLED; for
# workflow_dispatch RUN_REF DISPATCH_ENV DISPATCH_REF DISPATCH_INJECT DISPATCH_SKIP_SMOKE; GITHUB_SHA; MAIN_REF (default origin/main).
# Exit status: 0 it planned (possibly `skip`); 1 the request is refused; 2 it was called wrongly.
set -u
LC_ALL=C
export LC_ALL

MAIN_REF="${MAIN_REF:-origin/main}"

refuse() { echo "plan-deploy: refused: $*" >&2; exit 1; }
misuse() { echo "plan-deploy: $*" >&2; exit 2; }
emit() { # emit <mode> <environment> <sha> <inject> <skip_smoke> <reason>
  printf 'mode=%s\nenvironment=%s\nsha=%s\ninject=%s\nskip_smoke=%s\nreason=%s\n' "$1" "$2" "$3" "$4" "$5" "$6"
  exit 0
}
is_sha() { case "$1" in *[!0-9a-f]*|'') return 1 ;; esac; [ "${#1}" -eq 40 ]; }

[ -n "${EVENT_NAME:-}" ] || misuse "EVENT_NAME is not set"

case "$EVENT_NAME" in
  workflow_run)
    [ "${RUN_CONCLUSION:-}" = success ] || emit skip "" "" false false "ci did not succeed (conclusion '${RUN_CONCLUSION:-}'): nothing to deploy"
    [ "${RUN_EVENT:-}" = push ] || emit skip "" "" false false "ci ran for '${RUN_EVENT:-}', not a push: only a push to main deploys"
    [ "${RUN_BRANCH:-}" = main ] || emit skip "" "" false false "ci ran for the branch '${RUN_BRANCH:-}', not main"
    [ -n "${RUN_HEAD_REPO:-}" ] && [ "${RUN_HEAD_REPO:-}" = "${REPO:-}" ] \
      || emit skip "" "" false false "ci ran for '${RUN_HEAD_REPO:-}', not for this repository's own main"
    is_sha "${RUN_SHA:-}" || misuse "RUN_SHA '${RUN_SHA:-}' is not a 40-digit commit sha"
    [ "${DEPLOY_ENABLED:-}" = true ] \
      || emit skip "" "$RUN_SHA" false false "automatic deploys are off: set the repository variable THRUST_DEPLOY_ENABLED to true when staging exists"
    emit deploy staging "$RUN_SHA" false false "a push to main that ci passed: deploy it to staging"
    ;;
  workflow_dispatch)
    [ "${RUN_REF:-}" = refs/heads/main ] || refuse "a manual deploy is run from main, not from '${RUN_REF:-}': the workflow's own code must be main's"
    env_name="${DISPATCH_ENV:-}"
    case "$env_name" in
      staging|production) ;;
      *) refuse "the environment must be 'staging' or 'production', not '$env_name'" ;;
    esac
    inject="${DISPATCH_INJECT:-false}"
    skip_smoke="${DISPATCH_SKIP_SMOKE:-false}"
    case "$inject" in true|false) ;; *) refuse "inject_smoke_failure must be true or false, not '$inject'" ;; esac
    case "$skip_smoke" in true|false) ;; *) refuse "first_deploy_skip_smoke must be true or false, not '$skip_smoke'" ;; esac
    [ "$inject" = true ] && [ "$env_name" = production ] && refuse "inject_smoke_failure is for staging: it makes a deploy fail on purpose, and production is not the place to prove the rollback"
    [ "$inject" = true ] && [ "$skip_smoke" = true ] && refuse "inject_smoke_failure and first_deploy_skip_smoke together: a smoke that is skipped cannot be made to fail"
    ref="${DISPATCH_REF:-main}"
    sha="$(git rev-parse --verify --quiet "$ref^{commit}")" || refuse "'$ref' is not a commit in this checkout"
    is_sha "$sha" || refuse "'$ref' did not resolve to a commit sha"
    git rev-parse --verify --quiet "$MAIN_REF^{commit}" > /dev/null || misuse "$MAIN_REF does not exist in this checkout (fetch the history)"
    git merge-base --is-ancestor "$sha" "$MAIN_REF" || refuse "$sha is not on main: only commits of main reach an environment"
    emit deploy "$env_name" "$sha" "$inject" "$skip_smoke" "a manual run: $ref ($sha) to $env_name"
    ;;
  pull_request)
    is_sha "${GITHUB_SHA:-}" || misuse "GITHUB_SHA '${GITHUB_SHA:-}' is not a 40-digit commit sha"
    emit dry-run "" "$GITHUB_SHA" false false "a pull request: build and check the deploy configuration, deploy nothing"
    ;;
  *)
    emit skip "" "" false false "'$EVENT_NAME' does not deploy"
    ;;
esac
