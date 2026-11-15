# Plan

| Step | Status (2026-11-14) |
|---|---|
| PR 0: this review and the doc corrections | open |
| Settings (owner) | not started |
| PR 1: git-cliff and the rc bot | not started |
| PR 2: staging from `rc/*` | not started, needs the settings |
| PR 3: release on an rc merge, production from the tag | not started, needs the settings and PR 2 |
| PR 4: run only the related checks | not started, independent |

Order: PR 0, then the settings alongside PR 1, then PR 2, then PR 3. PR 4 can go any time after PR 0.

## Settings (the owner applies them)

Repository settings are outward-facing, so they are proposed as `gh api` calls for the owner to read and run, not run
by an agent.

- **`main` ruleset** (25418178): add `pull_request` (no approvals required, one owner), `non_fast_forward` and
  `deletion`; keep `gate`. A merge queue is optional (`ci.yml` already runs on `merge_group`).
- **`rc/*` ruleset**: restrict creation, update and deletion, and require `gate`. Bypass: the release App and the
  admin role.
- **`v*` tag ruleset**: restrict creation, update and deletion to the release App. Turn on immutable releases.
- **GitHub App `thrust-release`**: contents and pull requests, write. Its id and key in a `release` environment
  limited to `main`.
- **Environment branch policies**: `staging` and `staging-do-lifecycle` take only `rc/*`; `production` and
  `production-do-lifecycle` take only `v*` tags and get required reviewers.
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

## PR 2: staging from `rc/*` (`feat/deploy-from-rc`)

- `ci.yml` also runs on a push to `rc/**`, always in full.
- `deploy.yml`: `workflow_run` filtered to `rc/**`; a manual run takes staging with an `rc/*` ref, or production with a
  `v*` tag.
- `plan-deploy.sh`: staging needs `refs/heads/rc/v*`, the branch tip, and `gate` green on it; decide whether
  `THRUST_DEPLOY_ENABLED` stays as an off switch. `test-plan-deploy.sh` over every combination.
- `verify-bundle.sh`: the source ref `refs/heads/rc/*` for staging and `refs/tags/v*` for production; a must-fail case
  for a feature branch.
- `cicd.md` and `deploy.md` rewritten for the new trigger.

## PR 3: the release (`feat/release-on-rc-merge`)

- `.github/workflows/release.yml` on a merged pull request into `main` whose head is `rc/v*`:
  1. read the version from the branch and check `Cargo.toml` on the merge commit agrees;
  2. require the merge commit's tree to equal the rc tip's (true when the rc branch was current), else fail and let the
     bot regenerate;
  3. tag `vX.Y.Z` (annotated) with the App token;
  4. build once, promote: fetch the staging bundle of the rc commit, verify it, attest it again for the tag, and make
     and attest an SBOM, so production runs the bytes staging tested (the bundle's `BUILD_SHA` stays the rc commit,
     and the release records it);
  5. `gh release create` with that version's changelog, the bundle, the SBOM and `SHA256SUMS`;
  6. dispatch `deploy.yml` for production with the tag; reviewers approve it.
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
