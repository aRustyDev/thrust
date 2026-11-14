#!/bin/sh
# Is this bundle the output of THIS repository's deploy workflow, run on main? (issue #49, ADR-0101)
#
#   .github/scripts/verify-bundle.sh <deploy-bundle.tar.gz> <owner/repo>
#
# The same check for the bundle of the commit being deployed and for the previous release's, whose web files are added to the new
# deploy (merge-assets.sh): an artifact is only a name in this repository's list of artifacts, and a pull request from a fork can upload
# one under any name it likes, so a name is not evidence of anything. The attestation is. It must be signed by this repository's
# `deploy.yml` (`--signer-workflow`), on `refs/heads/main` (`--source-ref`), and not by a self-hosted runner.
#
# `--source-digest` is deliberately not asked for: on a `workflow_run` the workflow's own commit is the tip of main, not the commit `ci`
# ran on. Which commit the bundle is for is unpack-bundle.sh's question, answered from BUILD_SHA inside the attested file.
#
# Needs: gh, authenticated (GH_TOKEN). Exit status: 0 verified; 1 not verified (gh's own words on stderr); 2 called wrongly.
set -u

BUNDLE="${1:-}"; REPO="${2:-}"
[ -n "$BUNDLE" ] && [ -n "$REPO" ] || { echo "usage: verify-bundle.sh <deploy-bundle.tar.gz> <owner/repo>" >&2; exit 2; }
[ -f "$BUNDLE" ] || { echo "verify-bundle: $BUNDLE is not a file" >&2; exit 2; }
case "$REPO" in */*) ;; *) echo "verify-bundle: '$REPO' is not owner/repo" >&2; exit 2 ;; esac
command -v gh > /dev/null 2>&1 || { echo "verify-bundle: gh is not installed" >&2; exit 2; }

gh attestation verify "$BUNDLE" \
  --repo "$REPO" \
  --signer-workflow "$REPO/.github/workflows/deploy.yml" \
  --source-ref refs/heads/main \
  --deny-self-hosted-runners > /dev/null || { echo "verify-bundle: NOT VERIFIED: $BUNDLE has no attestation from $REPO's deploy.yml on main" >&2; exit 1; }
echo "verify-bundle: $BUNDLE is attested by $REPO/.github/workflows/deploy.yml on refs/heads/main"
