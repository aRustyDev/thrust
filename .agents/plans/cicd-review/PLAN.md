# Plan

| Step | Status (2026-11-15) |
|---|---|
| PR 0: this review and the doc corrections | #177, merged |
| Settings (owner) | applied 2026-11-15, the staging environments' branch rule after PR 2 merged |
| PR 1: git-cliff and the rc bot | #178, merged; replaced by PR 1b |
| PR 1b: release-plz in place of the hand-written bot | #183, merged; first release-candidate PR #184 (`v0.1.0`), open |
| PR 2: staging from `rc/*` | #185, merged |
| PR 3: attestation of a release, production from the tag | open |
| PR 4: run only the related checks | not started, independent |

Order: PR 3, then #184 (`v0.1.0`), which waits for PR 3 so that the release gets its assets (immutable releases lock them on
publish). PR 4 can go any time.

**The owner's rule (2026-11-15): use an existing public solution wherever one does the job; hand-write only where none
reliably does.** PR 1 was hand-written around git-cliff, and duplicated what git-cliff already does (`--prepend`,
`[changelog] format = true`). The owner chose to replace it with release-plz, with the branch named `rc/<timestamp>`
(release-plz takes only a prefix) instead of `rc/vX.Y.Z`, and a manual release for changes outside the crates.

## Settings (the owner applies them)

Repository settings are outward-facing, so they are proposed as `gh api` calls for the owner to read and run, not run
by an agent.

- **`main` ruleset** (25418178): add `pull_request` (no approvals required, one owner), `non_fast_forward` and
  `deletion`; keep `gate`. A merge queue is optional (`ci.yml` already runs on `merge_group`).
- **`rc/*` ruleset**: restrict creation, update, deletion and force-push. Bypass: the release App and the admin role.
  (Not `gate`: a required check gates pull requests INTO a branch, and nothing merges into `rc/*`.)
- **`v*` tag ruleset**: restrict creation, update and deletion to the release App. Turn on immutable releases.
- **GitHub App `thrust-release`**: contents and pull requests, write. Its id and key in a `release` environment
  limited to `main`.
- **Environment branch policies**: `staging` and `staging-do-lifecycle` take only `rc/*`; `production` and
  `production-do-lifecycle` take only `v*` tags and get required reviewers. GitHub checks them against the run's own ref,
  and a `workflow_run` always runs on `main`, so the staging rule waits for PR 2, which deploys from a run on the `rc/*`
  branch.
- **Already needed**: the smoke variables and secret on staging; everything on production; the Cloudflare token's
  scope behind the `assets-upload-session` failure. #49 stays open until a real staging deploy and the drill pass.

## PR 1: git-cliff and the rc bot (`feat/release-changelog`)

- `cliff.toml`: Conventional Commits; skip `chore(release)`; in 0.x a feature bumps the minor and a breaking change does
  not bump the major; group by type; leave `Refs:` trailers out of the rendered text.
- `.github/workflows/release-pr.yml` on a push to `main`, with its logic in `.github/scripts/release-pr.sh`:
  1. mint an App token;
  2. `git-cliff --bumped-version`, and stop if nothing is releasable;
  3. on top of `main`'s tip write `CHANGELOG.md`, the workspace version and `Cargo.lock`;
  4. make the commit through the Git Data API, so GitHub signs it;
  5. force `rc/vX.Y.Z` to it, and open or update the pull request `chore(release): vX.Y.Z`;
  6. if the version changed, open the new branch, close the old pull request with a comment and delete its branch.
- Push with the App token: a push made with the workflow's own token starts no workflow, so neither `ci` nor the
  staging deploy would run.
- Skip when the head commit is the squashed `chore(release): v…`, or the run races the release's tag and bumps against
  the old one. The release workflow starts this one again once the tag exists.
- One run at a time (`concurrency`, never cancelled), so two quick merges cannot interleave force pushes.
- Squash merging stays: the rc branch is `main` plus one commit, so its squash is the release commit.
- `test-release-pr.sh` (fake `gh`, a temporary repository; nothing to release, patch, minor, a version change, already
  current), under dash, in `ci-scripts`; `lint-workflow.sh` over the new workflow.

## PR 1b: release-plz (`refactor/release-plz`)

- Remove `cliff.toml`, `release-pr.yml`, `release-pr.sh` and its test.
- `release-plz.toml`: `git_only`, one `version_group` over every crate, one `v{{ version }}` tag and GitHub release (on
  `thrust-core`), one root `CHANGELOG.md` with every crate's commits, `release_always = false` with
  `pr_branch_prefix = "rc/"`, `release_commits` (fixes, features, performance, refactors, reverts, breaking), the minor
  bump for features in 0.x. Each setting checked on a copy of the workspace with release-plz 0.4.3.
- `release-plz.yml`: the `release` and `release-pr` jobs with `release-plz/action`, both with the release App's token.
- `.github/scripts/check-release-plz.py` (the one hand-written piece: release-plz has no workspace-level version group and
  does not check that every crate is listed), tested in `ci-scripts` and run by `deps`.
- The workspace version set to 0.1.0, so the first release is `v0.1.0` (release-plz releases an untagged package at the
  version it holds).

## PR 2: staging from `rc/*` (`feat/deploy-from-rc`)

As built (it differs from the first sketch, which had `ci` run on a push to `rc/**` and `deploy.yml` keep `workflow_run`):

- **Why not `workflow_run` alone.** GitHub checks an environment's branch rule against the run's own ref, and a
  `workflow_run` always runs on `main`, so `staging` restricted to `rc/*` would refuse it. And a push trigger on `rc/**` in
  `ci.yml` would run the full suite twice per release candidate (the push and the pull request), with two `gate` reports
  on one commit that `assert-sha-checks.sh` (every report must be SUCCESS) would trip on.
- `ci.yml`: the pull request of a release candidate (head `rc/*` in this repository) runs in full, whatever its diff.
- `deploy.yml`: `workflow_run` of `ci`, filtered to `rc/**`, plans `dispatch`, and a new `dispatch` job (`actions: write`,
  no secret) runs `gh workflow run deploy.yml --ref <rc branch> -f environment=staging -f sha=<commit>`; the dispatched run
  deploys. A manual run deploys the commit it runs from; the `ref` input is gone, a `sha` input (the commit the branch
  must still be at) replaces it.
- `plan-deploy.sh`: `dispatch` for a green `ci` on this repository's `rc/<name>` pull request while
  `THRUST_DEPLOY_ENABLED` is `true` (kept as the off switch for the automatic path); staging from `refs/heads/rc/<name>`,
  production from `refs/tags/v<version>` whose commit is on `main`; one segment after the prefix, as the rulesets match;
  a moved branch skips. `test-plan-deploy.sh` over every combination.
- `verify-bundle.sh <bundle> <repo> <source-ref>`: exactly that ref, which must be `refs/heads/rc/<name>` or
  `refs/tags/v<version>` (anything else is refused before gh is asked); the action passes the run's own ref.
- `fetch-previous-assets.sh`: artifacts of this repository's own runs (`head_repository_id == repository_id`) on an
  `rc/*` branch or a `v*` tag, verified on the ref their run was on.
- `cicd.md`, `deploy.md` rewritten for the new trigger. The deploy build no longer saves its cache (a cache is readable only
  from its own ref, and every release candidate is a new branch).

## PR 3: attestation of a release, production from the tag (`feat/release-attest-deploy`)

As built. The owner chose on 2026-11-15 to **build once at the tag** rather than promote staging's bundle (the sketch below
had promotion): the tag points at `main`'s squash commit, not at the rc commit staging deployed, and the deploy uses the commit as
the Worker's version and diffs it against the live one in git, so a promoted bundle would have deployed the rc commit, which
nothing on `main` points at once its branch is deleted, and production would have waited on staging working.

- `release-plz.toml`: `git_release_draft = true`. release-plz creates the tag through the API, then the release
  (`create_git_tag_and_release` in its source), so the tag exists when the draft does.
- `.github/actions/build-bundle`: the bundle build, out of `deploy.yml`, so `deploy.yml` and `release.yml` build it the same
  way.
- `release.yml` (new), on `release: created` for a draft of a `v<version>` tag on `main`: `build` (the bundle, an SPDX SBOM of
  the source with `anchore/sbom-action`, both attested with `actions/attest-build-provenance` and `actions/attest`) and
  `publish` (bundle, SBOM and `SHA256SUMS` onto the draft, then published; GitHub's own release attestation checked with
  `gh release verify` and `verify-asset`). A pull request that edits it runs `build` as a dry run.
- `deploy.yml`: `workflow_run` also on `main`; `plan-deploy.sh` plans a production `dispatch` on the tag when `ci` passed on
  a push to `main` whose commit carries exactly one `v*` tag (`RUN_TAGS`, `git tag --points-at`). Production builds
  nothing: the deploy action fetches the release's asset (`fetch-release-bundle.sh`: published, immutable, one bundle,
  `gh release verify-asset`). `verify-bundle.sh` takes `release.yml` as the signer for a `v*` tag, `deploy.yml` for `rc/*`.
- Docs: `docs/src/architecture/release.md` (the path, how to check a release, what to do when a step fails), `cicd.md`,
  `deploy.md`; ADR-0110 gets a dated update (the repository's ADRs are amended by dated updates, not superseded by new ones).

The first sketch, kept for the record:

release-plz (PR 1b) already tags `vX.Y.Z` and publishes the GitHub release when an `rc/*` pull request merges. What is
left, on the new tag (the App's token makes the tag start workflows):

  1. build once, promote: fetch the staging bundle of the rc commit, verify it, attest it again for the tag, and make
     and attest an SBOM (look for a public action first: `actions/attest-build-provenance`, `actions/attest-sbom`,
     `anchore/sbom-action`), so production runs the bytes staging tested;
  2. attach the bundle, the SBOM and `SHA256SUMS` to the release;
  3. dispatch `deploy.yml` for production with the tag; reviewers approve it.
- `plan-deploy.sh` resolves tag, then rc commit, then bundle, with tests.
- `docs/src/architecture/release.md` (the flow and how to recover from a failed release), and an ADR for the branch
  and release model that supersedes ADR-0110's "a push to `main` deploys staging".

## PR 4: run only the related checks (`perf/ci-affected-only`)

- `classify-changes.sh` answers classes instead of two booleans: the changed crates and their reverse dependencies
  (from `cargo metadata`, in a new `affected-crates.sh`), and `deploy`, `ci-self`, `web`, `e2e`, `oracle`, `docs`.
- Each class maps to work: `hermetic` filters with `rdeps(...)`; `plan-cells.sh` keeps the cells whose target or pass
  the change can reach (the `e2e` cell on `web/`, which closes G-30); `mutants.sh` rows are tagged by what they cover;
  `pg-adapter` only for the Postgres crates; deploy scripts run `ci-scripts` and the dry run, not the matrix; `scripts/`
  is mapped file by file.
- Full runs stay for `merge_group`, `rc/**` and a new nightly run of `ci` on `main` (ADR-0100), which also catches a
  new `stable` or a new advisory.
- `assert-gate.sh` gets the expected set from `changes`: expected means `success`, not expected means skipped.
- `test-ci-scripts.sh` gets a generated class-by-job matrix with controls: a path mapped to the wrong class must fail a
  case.
- Smaller fixes: `lint-workflow.sh` over every workflow and `setup-rust`; `--locked` on `hermetic` and `pg-adapter`; a
  `docs` job (book build and orphan check); a `rust-toolchain.toml`; `scripts/verify-all.sh`'s "16-cell" comment.

## Later: the E2E and integration recommendations

One issue per journey in FINDINGS §3 and per gap in §4, drafted to a file and filed only when the owner asks.

## Verification

- Every new or changed script has a test under `.github/scripts/tests/`, run under dash in `ubuntu:24.04`, with a
  must-fail case beside each must-pass case, and is broken by hand once to see its test fail.
- `lint-workflow.sh` and `actionlint` over the workflows; `git-cliff --unreleased --bumped-version` on a scratch clone
  prints `v0.1.0`.
- Once the settings are on: a `docs:` merge makes no rc branch; a `fix:` merge makes `rc/v0.1.0` (signed, with the
  changelog and version), `ci` goes green on it and staging deploys and smokes; a push to it by anyone else is refused;
  merging it makes the tag, an immutable release whose attestations verify, and a production deploy waiting for review;
  deleting the tag is refused.
- PR 4: draft pull requests that each touch one class; the planned set matches the table, and `gate` fails when an
  expected job is forced to skip.
