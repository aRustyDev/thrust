# Triggers, concurrency, affected-only runs

Tags: "documented" = GitHub docs (retrieved 2026-10-18); `[derived]` = inferred, not executed;
`[unverified: not run]` = not executed.

## Events the example uses (`examples/github/workflows/ci.yml`)

| Event | Why |
|---|---|
| `pull_request` | PR validation on the merge commit; not run when the PR has a merge conflict (documented) |
| `merge_group` | required when a merge queue is on: "you need to update the workflows to include the `merge_group` event as an additional trigger" (documented). `GITHUB_SHA`/`GITHUB_REF` are the merge group's |
| `push` to `main` | post-merge confidence and cache saving |
| `workflow_dispatch` | manual run; no inputs on `ci.yml` |

Only `branches`/`branches-ignore` filters are documented for `merge_group`; do not rely on `paths:` there.

## `paths` / `paths-ignore` (documented)

- Glob patterns with `*` and `**`; `!` negates, and **order matters** (a later negative pattern excludes
  an earlier positive match).
- `paths` and `paths-ignore` cannot both be used for the same event.
- Not evaluated for tag pushes.
- Diff basis: pull requests use a three-dot diff; pushes use a two-dot diff of the head and base SHAs.
- A workflow skipped by a path filter reports **no check at all**. If that check is required the PR waits
  forever (see `required-checks.md`). Therefore: **path filters only on optional workflows**. For a
  required one, filter inside the workflow with a `changes` job.

## Affected-only via a `changes` job (the example)

`ci.yml` computes `rust=true|false` and downstream jobs use `if: needs.changes.outputs.rust == 'true'`.

- Base SHA per event: `pull_request` -> `github.event.pull_request.base.sha`; `merge_group` ->
  `github.event.merge_group.base_sha`; `push` -> `github.event.before`. Anything else, or an all-zero
  `before` (first push), or a failed `git diff` -> **fail open** (`rust=true`). Failing open costs minutes;
  failing closed costs a missed regression.
- The classifier is a script, `.github/scripts/classify-changes.sh`, reading changed paths on stdin. Its cases
  (`test-ci-scripts.sh`, each asserting its output): docs-only -> `rust=false`; `crates/...`, `Cargo.lock`,
  `.github/workflows/...`, `.github/scripts/...` -> `rust=true`; an empty list -> `rust=true`; a look-alike path
  `crates-notes.md` -> `rust=false`.
- `actions/checkout` needs `fetch-depth: 0` for the diff to find the base commit.
- Include the CI files themselves (`.github/workflows|actions|scripts/`) in the "relevant" set, or a change
  to the pipeline is never tested by the pipeline.
- Third-party path-filter actions exist; a 20-line script avoids another pinned dependency and is testable.

## `concurrency` (documented)

```yaml
concurrency:
  group: ci-${{ github.event.pull_request.number || github.event.merge_group.head_sha || github.ref }}
  cancel-in-progress: ${{ github.event_name == 'pull_request' }}
```

- The expression may use only the `github`, `inputs` and `vars` contexts.
- By default a *pending* run in the same group is replaced by the newer one; the docs page also describes
  `queue: max` (up to 100 pending runs, FIFO) as an alternative `[derived: read, not exercised]`.
- Cancel superseded PR runs; **never** cancel `main` or merge-queue runs. Never cancel a deploy
  (`cancel-in-progress: false` in a dedicated group such as `pages`).
- A cancelled run reports `cancelled`; `assert-gate.sh` treats that as a failure, so a cancelled run can
  never satisfy the gate.

## `workflow_dispatch` inputs (documented; `flake-hunt.yml` uses them)

- Types: `string`, `boolean`, `choice`, `environment`, `number`. Limits: 25 top-level inputs, 65,535 characters
  of payload.
- Every input needs a `default` or the workflow must cope with empty values; **`inputs` is empty on other
  triggers** (e.g. `schedule`), so write `${{ inputs.target || 'native-memory' }}`.
- Pass inputs to scripts through `env:`; validate numbers in the shell (the example rejects non-integers and
  values outside 1-50 before running anything).

## `schedule` (documented)

- POSIX cron, UTC by default (an IANA timezone can be given); runs on the **default branch's** latest commit;
  minimum interval 5 minutes.
- In a public repository, scheduled workflows are disabled automatically after 60 days without repository
  activity. Do not make a scheduled job the only guard for something important.
- Pick an off-peak minute (`17 3 * * 1`), not `0 0 * * *`.

## Quick checks before merging a trigger change

1. Does every required workflow also trigger on `merge_group`?
2. Is any required workflow still path-filtered? (If yes, it is a latent permanent block.)
3. On the default branch, does `concurrency` cancel anything it must not?
4. Does `changes` fail open on every event it can receive?
