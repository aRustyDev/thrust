#!/bin/sh
# Is this bundle the output of THIS repository's build workflow for a ref that deploys? (issue #49, ADR-0101; the rc/* flow)
#
#   .github/scripts/verify-bundle.sh <deploy-bundle.tar.gz> <owner/repo> <source-ref>
#
# The same check for the bundle of the commit being deployed and for the previous release's, whose web files are added to the new
# deploy (merge-assets.sh): an artifact is only a name in this repository's list of artifacts, and a pull request from a fork can upload
# one under any name it likes, so a name is not evidence of anything. The attestation is. It must be signed by the workflow of this
# repository that builds that kind of ref (`--signer-workflow`), on exactly `<source-ref>` (`--source-ref`), and not by a self-hosted
# runner.
#
# `<source-ref>` is the ref of the run that built the bundle, and only two kinds deploy (plan-deploy.sh):
#   refs/heads/rc/<name>    a release candidate, for staging: built by deploy.yml, in the run that deploys it;
#   refs/tags/v<version>    a release, for production: built by release.yml at the tag, and attached to the release.
# Any other ref is refused without asking gh: a bundle built anywhere else was never meant for an environment. (main no longer
# deploys: the release flow moved staging to rc/*.)
#
# `--source-digest` is deliberately not asked for: which commit the bundle is for is unpack-bundle.sh's question, answered from
# BUILD_SHA inside the attested file.
#
# Needs: gh, authenticated (GH_TOKEN). Exit status: 0 verified; 1 not verified (gh's own words on stderr), or a ref that does not
# deploy; 2 called wrongly.
set -u

BUNDLE="${1:-}"; REPO="${2:-}"; SOURCE_REF="${3:-}"
[ -n "$BUNDLE" ] && [ -n "$REPO" ] && [ -n "$SOURCE_REF" ] \
  || { echo "usage: verify-bundle.sh <deploy-bundle.tar.gz> <owner/repo> <source-ref>" >&2; exit 2; }
[ -f "$BUNDLE" ] || { echo "verify-bundle: $BUNDLE is not a file" >&2; exit 2; }
case "$REPO" in */*) ;; *) echo "verify-bundle: '$REPO' is not owner/repo" >&2; exit 2 ;; esac
# one path segment after the prefix, as the rulesets that guard rc/* and v* match it
case "$SOURCE_REF" in
  refs/heads/rc/*/*|refs/heads/rc/|refs/tags/v*/*|refs/tags/v) SIGNER="" ;;
  refs/heads/rc/*) SIGNER=deploy.yml ;;
  refs/tags/v*) SIGNER=release.yml ;;
  *) SIGNER="" ;;
esac
[ -n "$SIGNER" ] || { echo "verify-bundle: NOT VERIFIED: '$SOURCE_REF' is not a ref that deploys (refs/heads/rc/<name> or refs/tags/v<version>)" >&2; exit 1; }
command -v gh > /dev/null 2>&1 || { echo "verify-bundle: gh is not installed" >&2; exit 2; }

gh attestation verify "$BUNDLE" \
  --repo "$REPO" \
  --signer-workflow "$REPO/.github/workflows/$SIGNER" \
  --source-ref "$SOURCE_REF" \
  --deny-self-hosted-runners > /dev/null || { echo "verify-bundle: NOT VERIFIED: $BUNDLE has no attestation from $REPO's $SIGNER on $SOURCE_REF" >&2; exit 1; }
echo "verify-bundle: $BUNDLE is attested by $REPO/.github/workflows/$SIGNER on $SOURCE_REF"
