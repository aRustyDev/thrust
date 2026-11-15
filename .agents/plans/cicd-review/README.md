# CI/CD review (2026-11-14)

A review of how thrust is tested, built and deployed, against the git-flow the owner wants:

- work happens in worktrees under `.agents/worktrees/`, on feature branches, and reaches `main` only by pull request;
- every merge to `main` updates a bot-owned `rc/vX.Y.Z` branch with a git-cliff `CHANGELOG.md`;
- only `rc/*` deploys to the `staging` environment, and only bots and admins can create or change `rc/*`;
- merging an `rc/*` branch to `main` cuts a release: an immutable tag, a GitHub release, provenance and attestation.

## Owner decisions (2026-11-14)

| Question | Decision |
|---|---|
| What starts a staging deploy | Every push to `rc/*`. `main` no longer deploys (this replaces #49's "a push to `main` deploys staging") |
| How production is deployed | From the release tag, behind the required-reviewer `production` environment |
| Versioning | `git-cliff --bumped-version`; the rc branch writes `[workspace.package] version` and `CHANGELOG.md`; the first release is `v0.1.0` |
| Where the review lands | Here, with its conclusions in `docs/src/` |

## What is here

| File | Holds |
|---|---|
| [FINDINGS.md](FINDINGS.md) | The answers: which tests run against which crates, when checks trigger, which E2E flows and integration points to test, and the gap between today and the target flow |
| [PLAN.md](PLAN.md) | The staged pull requests that close the gaps, in order, with what each needs from the owner |

Conclusions about the current state are in the docs, not repeated here: [CI/CD](../../../docs/src/architecture/cicd.md)
(including "Where the triggers are too wide or too narrow"), [Deploying the Worker](../../../docs/src/architecture/deploy.md)
and [Testing status](../../../docs/src/testing/status.md).
