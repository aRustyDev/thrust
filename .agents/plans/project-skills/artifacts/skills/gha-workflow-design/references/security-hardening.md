# Hardening: permissions, pinning, injection, linting

Tags: "documented" = GitHub or zizmor docs (retrieved 2026-10-18); `[derived]` = inferred, not executed;
`[unverified: not run]` = not executed.

## `permissions:` least privilege

- Set `permissions: {}` at workflow level and grant per job. The docs recommend a read-only default for
  `GITHUB_TOKEN`, raised per job. Once any scope is specified, **every unspecified scope becomes `none`**
  (documented), so a job listing only `contents: read` has no other access.
- Typical grants in the example: `contents: read` for any job that checks out; a Pages deploy needs
  `pages: write` and `id-token: write` only on the deploy job.
- Reusable workflows can only keep or reduce the caller's permissions (documented); set the caller job's
  `permissions:` explicitly (the example's `contract` call does).
- zizmor reports missing or broad grants as `excessive-permissions` and unexplained ones as
  `undocumented-permissions` (documented rule ids) `[unverified: not run]`: comment why each non-read grant
  exists.

## Pin third-party actions to a full commit SHA

Docs: pinning to a full-length commit SHA is the only way to use an action as an immutable release; tags can be
moved. Verify the SHA belongs to the action's own repository, not a fork.

```yaml
- uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
```

- Resolve a tag to a commit (no GitHub token needed): for an annotated tag use the peeled `^{}` line.

```sh
git ls-remote https://github.com/actions/checkout 'refs/tags/v7.0.1*'
```

- The SHAs in `examples/github/` were resolved on 2026-10-18 and are a snapshot. Re-resolve when bumping and
  keep the `# vX.Y.Z` comment: zizmor's `ref-version-mismatch` compares hash and comment `[derived]`.
- `dtolnay/rust-toolchain` is conventionally referenced by a moving ref (`@stable`); pinned by SHA the channel
  must be passed as `with: toolchain:` (the example does).
- Automate bumps: Dependabot `package-ecosystem: "github-actions"` proposes SHA + comment updates `[derived]`.
- Pin tools installed inside steps as well: `cargo install <tool> --locked --version =X`, `npm install --global
  <tool>@X`, never `latest`; zizmor names these `unpinned-tools` / `unpinned-uses` `[derived]`. The example's
  `setup-rust` takes the versions as inputs and passes them through `env:`.

## Script injection

Never interpolate untrusted context values into `run:` text: `${{ github.event.pull_request.title }}` becomes
shell code. Pass them through `env:` and use the shell variable (documented):

```yaml
env:
  TITLE: ${{ github.event.pull_request.title }}
run: echo "$TITLE"
```

The example does this for every input (`CELL_TARGET`, `CELL_PASS`, `TARGET`, `PASS`, `ITER`, the SHAs, the tool
versions). actionlint checks for untrusted inputs in scripts; zizmor's `template-injection` is the stricter
audit.

## Other defaults worth setting

- `actions/checkout` with `persist-credentials: false` (zizmor `artipacked`): the token otherwise stays in
  `.git/config` for every later step and for uploaded artifacts.
- Avoid `pull_request_target` unless essential, and never check out the PR head in it (documented: running
  untrusted code there can leak secrets and enable cache poisoning). zizmor: `dangerous-triggers`.
- Do not let jobs that handle untrusted PRs **save** caches that release jobs later restore (`cache-poisoning`);
  the example saves only from `refs/heads/main`, for the Rust cache and the installed `worker-build` binary alike.
- `timeout-minutes` on every job; `concurrency` limits (zizmor `concurrency-limits`).
- Review uploaded logs before making a public repository's artifacts the debugging path. The contract script
  generates throwaway per-run tokens and passwords in the process environment and states that none is written
  to a file or argv; confirm that stays true before uploading `target/contract-logs/`.
- Treat the scripts the workflow runs as part of the attack surface: review changes to `.github/scripts/` and
  `scripts/` like workflow changes (the example's `changes` job counts them as "relevant").

## Linting

**actionlint 1.7.12.** Run it from a git checkout:

```sh
actionlint                       # lints .github/workflows/*.yml in the repository
actionlint .github/workflows/ci.yml
```

- It parses expressions with types, validates `needs:`, matrix, event names, cron syntax, runner labels, glob
  patterns, action inputs and `workflow_call` inputs, permissions, and runs **shellcheck** on `run:` scripts when
  `shellcheck` is on PATH (and pyflakes for Python when present).
- **Silent-skip trap:** outside a git repository it does not resolve local composite actions or reusable
  workflows, so a misspelled `with:` input passed lint with exit 0. Inside the repository the same input
  errors: `input "cache-kee" is not defined in action "setup-rust"` and `input "targt" is not defined in
  "./.github/workflows/contract-cell.yml" reusable workflow`.
- **Must-fail control.** After the real run is clean, lint a deliberately broken copy (a misspelled step id, a
  nonexistent `needs:` target, a bad `with:` key); it must report all of them. A linter run that cannot fail has
  told you nothing. Rules for the control file:
  - keep it in `.github/lint-controls/`, **never under `.github/workflows/`**: a broken file left there is a
    live workflow and project-mode `actionlint` would fail on it;
  - lint it with `actionlint .github/lint-controls/<file>`; the skill's exact `Bash(actionlint)` grant does not
    cover an argument, so that call prompts, and writing outside the granted `.github/` subtrees prompts too;
  - **delete the control before finishing**.
- It does not check that an action SHA exists or matches its comment.

**zizmor** is a static audit of workflows for the issues above `[unverified: not run]`. Audits relevant here
(documented ids): `unpinned-uses`, `excessive-permissions`, `template-injection`, `artipacked`,
`cache-poisoning`, `dangerous-triggers`, `secrets-inherit`, `concurrency-limits`, `unpinned-tools`,
`undocumented-permissions`, `ref-version-mismatch`, `forbidden-uses` (opt-in allow/deny list). Typical use:
`zizmor .github/workflows`, triage by severity; many audits have auto-fix modes, so review each diff.

**shellcheck** for the repo scripts: `shellcheck -s sh .github/scripts/*.sh`.
