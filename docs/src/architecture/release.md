# Releasing

How a change becomes a release and reaches production, how to check a release, and what to do when a step fails. How the
release-candidate pull request is made is in [CI/CD](cicd.md#release-candidates); how a deploy works is on [Deploying the
Worker](deploy.md). The decision is ADR-0110's update of 2026-11-15.

## From a merge to production

1. **A release candidate.** After every push to `main`, release-plz opens or updates the pull request `chore(release): vX.Y.Z`
   from an `rc/<timestamp>` branch (the version and `CHANGELOG.md`). `ci` runs on it in full, and when it is green and the
   repository variable `THRUST_DEPLOY_ENABLED` is `true`, `deploy.yml` deploys that commit to **staging**.
2. **The release.** Merging that pull request (squash, the only method) makes the commit release-plz tags `vX.Y.Z`, and it
   creates the GitHub release as a **draft**. A draft starts no workflow, so the same job starts `release.yml` on the tag.
3. **Its assets** (`release.yml`, run on the tag): the deploy bundle is built at the tag, its build provenance and an SPDX
   SBOM of the source are attested, and `deploy-bundle.tar.gz`, `sbom.spdx.json` and `SHA256SUMS` are attached. Then the release
   is published. Published releases are immutable: the tag cannot move and the assets cannot change, and GitHub attests the
   release and its assets.
4. **Production.** When `ci` passes on the tagged commit (its push to `main`), `deploy.yml` dispatches a production deploy on the
   tag, which waits for you in the `production` environment. Approve it. It deploys the release's own bundle and builds nothing.

Steps 1 and 4 need `THRUST_DEPLOY_ENABLED`; without it, run **deploy** by hand: staging from the `rc/*` branch, production from
the `v*` tag ("Use workflow from").

## Checking a release

```sh
gh release verify vX.Y.Z                                   # GitHub's attestation of the immutable release
gh release download vX.Y.Z --dir /tmp/rel && cd /tmp/rel
gh release verify-asset vX.Y.Z deploy-bundle.tar.gz        # this file is that release's asset
gh attestation verify deploy-bundle.tar.gz --repo aRustyDev/thrust \
  --signer-workflow aRustyDev/thrust/.github/workflows/release.yml --source-ref refs/tags/vX.Y.Z   # built by release.yml at the tag
gh attestation verify deploy-bundle.tar.gz --repo aRustyDev/thrust \
  --predicate-type https://spdx.dev/Document/v2.3                                                 # its SBOM
sha256sum -c SHA256SUMS
```

The bundle names the commit it was built from in `BUILD_SHA`; the Worker reports the deployed commit as `x-thrust-version`.

## When a step fails

| What failed | State | What to do |
|---|---|---|
| `release.yml` never started (no run on the tag) | The tag exists; the release is a draft with no assets | Start it on the tag: `gh workflow run release.yml --ref vX.Y.Z`. The tag's commit must hold a `release.yml` with `workflow_dispatch`, which `v0.1.0`'s does not |
| `release.yml` before `publish` (the build, the SBOM, an attestation) | The tag exists; the release is still a draft with no assets or some. Production refuses a draft | Re-run the failed jobs of that `release.yml` run. `publish` replaces any asset already attached (`--clobber`) |
| `publish` says the release is no longer a draft | Someone published it by hand, or a second run did | If it was published without its assets it stays that way (immutable): release the next version. Production refuses a release without exactly one bundle |
| `publish` says the release is not immutable | Immutable releases were turned off | Turn them on (repository settings); the release that was published meanwhile is not trusted by production |
| The production deploy says the release is a draft | `release.yml` has not finished, or failed | Wait for it, or fix it as above, then run **deploy** by hand from the tag |
| A bad version is released | It is permanent: a `v*` tag cannot be deleted or moved, by design | Fix forward: merge the fix, and release the next version. To get production off the bad one now, [roll back by hand](deploy.md#rolling-back-by-hand) |
| The release candidate is stale when merged | release-plz updates it on every push to `main`, so this is a race | The tag is on the merge commit whatever it holds; `ci` on that commit gates production |

## A change outside the crates

release-plz releases what changes a crate's files. A change only under `web/`, `scripts/` or `e2e/` is released by hand: the
steps are in [CI/CD](cicd.md#release-candidates) ("What does not").
