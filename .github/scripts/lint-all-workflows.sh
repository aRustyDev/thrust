#!/bin/sh
# Run lint-workflow.sh over EVERY workflow and composite action of the repository, each with the credentials it is allowed.
# A file is found by its directory, not listed by name, so a new workflow is linted the day it is added, and it gets the
# strictest policy (no `secrets.`, no `environment:`, no `id-token: write`) until a line below gives it more.
#
#   .github/scripts/lint-all-workflows.sh [repository root]   (default: this one)
#
# Exit status: 0 every file clean; 1 a rule is broken (lint-workflow.sh says where); 2 no workflow found or a call gone wrong.
set -u
ROOT="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
LINT="$(cd "$(dirname "$0")" && pwd)/lint-workflow.sh"

# `-` is a job name that cannot exist (a job id starts with a letter or `_`), so it means "no job". lint-workflow.sh reads an
# empty value as its own default, which is the deploy's, so empty must never be passed.
NONE=-
# policy <workflow file name>: "<jobs that may use secrets and an environment>|<jobs that may have id-token: write>"
policy() {
  case "$1" in
    deploy.yml) echo "deploy deploy-do deploy-target deploy-target-migrate deploy-edge|build" ;;  # the environments' secrets; the attestation
    release.yml) echo "$NONE|build" ;;                 # the release's attestations; its token is the workflow's own
    release-plz.yml) echo "release release-pr|$NONE" ;; # the release App's key, in the `release` environment
    flagship.yml) echo "sync check|$NONE" ;;           # the Flagship app's tokens, in the `flagship` environment
    cloudflare-infrastructure.yml) echo "tofu|$NONE" ;;
    edge-infrastructure.yml) echo "tofu|$NONE" ;;           # the edge root's state and Cloudflare token, in `edge-infrastructure`
    deploy-preview-provisioner.yml) echo "deploy|$NONE" ;;
    enable-preview-provisioner.yml) echo "enable|$NONE" ;;
    preview-reconcile.yml) echo "reconcile|$NONE" ;;
    *) echo "$NONE|$NONE" ;;
  esac
}

status=0
found=0
for f in "$ROOT"/.github/workflows/*.yml "$ROOT"/.github/workflows/*.yaml; do
  [ -f "$f" ] || continue
  found=$((found + 1))
  p="$(policy "$(basename "$f")")"
  SECRET_JOBS="${p%%|*}" ID_TOKEN_JOBS="${p#*|}" "$LINT" "$f" || status=1
done
[ "$found" -ge 1 ] || { echo "lint-all-workflows: no workflow under $ROOT/.github/workflows" >&2; exit 2; }
actions=0
for f in "$ROOT"/.github/actions/*/action.yml "$ROOT"/.github/actions/*/action.yaml; do
  [ -f "$f" ] || continue
  actions=$((actions + 1))
  "$LINT" "$f" || status=1
done
echo "lint-all-workflows: $found workflows and $actions actions"
exit "$status"
