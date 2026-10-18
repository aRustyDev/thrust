# Required checks: assert the set, not the absence of failures

Tags: "documented" = GitHub docs (retrieved 2026-10-18); `[derived]` = inferred, not executed;
`[unverified: not run]` = not executed. The helper scripts are exercised by `examples/github/scripts/tests/`
with fixtures and a fake `gh`.

## The rule

**A check that reports success because the required contexts never ran is vacuous.** "No failures" is the
wrong predicate. Assert the **set** of required contexts that reported, and that each is green. Two layers:

1. **Inside CI** - `gate` asserts the planned units reported (`assert-gate.sh`).
2. **Outside CI** - anything that says "checks passed" (a script, a bot, an agent polling a PR) compares the
   branch's required contexts against what reported (`assert-required-contexts.sh`).

A summary computed from *reported* checks sees only what exists. If the workflow never started, there is
nothing pending and nothing failing, so the summary reads "all passed" while the PR is blocked on required
contexts that were never created. The same shape as a test runner that exits 0 when its filter selected zero
tests.

## What GitHub does with skipped work (documented)

| Situation | Result |
|---|---|
| Workflow skipped by a **path or branch filter**, or by commit message | its checks stay **Pending** ("Expected - Waiting for status to be reported") and **block merging** |
| A **job** skipped by an `if:` conditional | reports **Success** and does not block |
| A check and a commit status share a name | both must pass when that name is required |

Docs advice: avoid requiring workflows that can be skipped. Corollary: a job skipped by `if:` *passes* a
required check, so skipping must be **proven legitimate**, not assumed.

## `startup_failure`: a run with zero jobs

A workflow can fail before any job exists: the run's conclusion is `startup_failure` and it has no jobs, so it
creates none of the contexts you required. Commonly reported causes `[derived]`: an invalid workflow file after
a merge, an unresolvable `uses:` path, a reusable workflow requesting more permissions than its caller grants, a
policy rejecting an action reference. Do not trust the message; diff the workflow against the base branch and
check the platform status page first, because an outage presents the same way. Detect it:

```sh
gh run list -R <owner>/<repo> --workflow ci.yml --limit 20 --json databaseId,conclusion,event,headSha \
  --jq '.[] | select(.conclusion == "startup_failure")'
gh run view <run-id> -R <owner>/<repo> --json jobs --jq '.jobs | length'     # 0 for a startup_failure
```

(Both `jq` forms are valid against a public repository, which had no `startup_failure` run.)

## Pattern A (preferred): changes + one `gate`

`examples/github/workflows/ci.yml`. The only required context is `gate`.

- `gate` runs `if: always()` with `needs:` on every job. Without `always()` it is *skipped* when a dependency
  fails, and a skipped required job counts as passing.
- `assert-gate.sh` (cases in `test-ci-scripts.sh`):
  - `changes` must have succeeded;
  - nothing relevant changed -> every needed job may be `skipped`, none may be `failure`/`cancelled`;
  - relevant change -> every needed job must be exactly `success` (a `skipped` one is a vacuous pass), **and**
    the set of per-cell markers equals the planned set;
  - an empty planned set, no markers, a marker for an unplanned cell and a cancelled matrix each fail with their
    own message (the tests assert the message, so no case passes for the wrong reason).
- The `ci-scripts` job runs those tests and is a `needs:` of `gate`: the logic behind the required check is
  itself regression-tested, and a mutation of `assert-gate.sh`, the classifier or the certifier is caught.
- Each cell uploads a one-line marker artifact only after `scripts/contract.sh` succeeded; `gate` downloads
  `cell-ok-*` and compares with the planned set using `comm`. This makes the **per-cell** set checkable, because
  `needs.<matrix job>.result` is one aggregate.
- The download step uses `continue-on-error: true` on purpose: if every cell failed there is nothing to
  download, and the assert step then fails on "no cell markers", so tolerating the download error cannot produce
  a green gate.
- Require `gate`, not the matrix jobs: matrix job names change when the matrix changes, which silently turns a
  required name into one that never reports. A stable name that you control does not.
- `startup_failure` of `ci.yml` itself leaves `gate` unreported, so the PR **blocks** rather than passes.

## Pattern B: a stub with the same check name

Use only if a workflow must keep a workflow-level path filter. A required context is the **check name**, which
is the job's name (or its `name:`); the workflow's own name is not part of it. Add a sibling workflow whose job
produces the same check name and that runs on the complement of the paths:

```yaml
name: ci-stub
on:
  pull_request:
    paths-ignore: ["crates/**", "scripts/**", "Cargo.toml", "Cargo.lock", ".github/**"]
jobs:
  gate:
    runs-on: ubuntu-24.04
    steps:
      - run: echo "no relevant paths changed; satisfying the required context"
```

Fragile: the two path lists must be exact complements, a later edit to one silently breaks the other, and
merge-queue events need their own handling. A reusable workflow nests names (`caller / callee`), so a stub for
such a check must reproduce the nested name. The stub passes actionlint 1.7.12; that says nothing about the
pairing being correct. Prefer Pattern A.

## Certifying a PR from outside (`assert-required-contexts.sh`)

```sh
.github/scripts/assert-required-contexts.sh <owner/repo> <pr-number> [base-branch]
STRICT_CONTEXTS="gate" .github/scripts/assert-required-contexts.sh <owner/repo> <pr-number>
```

- **Required contexts** = the branch's active rulesets (`GET repos/<r>/rules/branches/<branch>`, entries of
  type `required_status_checks`) plus classic branch protection
  (`GET .../protection/required_status_checks`, called with `gh api -i` so the HTTP status is visible).
  - A failed ruleset call stops the script.
  - For classic protection **only HTTP 404 means "none configured"** (gh exits 1 and prints the status line
    `HTTP/2.0 404 Not Found`). Any other outcome, 403, 5xx or no HTTP response at all, means the answer is
    unknown, and the script fails instead of certifying on a partial set. Tests cover 404, 403, 502 and a
    network failure.
- **Reported** = `gh pr view <n> --json statusCheckRollup`, normalising check runs (`name`, `conclusion` or
  `status`) and commit statuses (`context`, `state`).
- Exit 0 only if `required \ reported` is empty **and** every required context is SUCCESS, SKIPPED or NEUTRAL.
  No required checks configured is a failure ("nothing to certify against"), not a pass. A re-run leaves several
  rows per name; the script is deliberately conservative about that.
- **SKIPPED semantics:** GitHub lets a skipped check satisfy a required context, so the certifier accepts it
  by default. For an aggregate such as `gate` that is exactly the accidental pass this skill warns about: name it
  in `STRICT_CONTEXTS` (space-separated) and only SUCCESS is accepted for it.
- Intermediate files live in a temporary directory removed on exit.
- Needs an authenticated `gh` and read access to rulesets and branch protection.

## Checklist when a required check "never runs"

1. `gh run list ... --json conclusion` for `startup_failure`; open the run, count jobs.
2. Is the workflow path- or branch-filtered? Does it also list `merge_group`?
3. Does the required context's name still match a check name exactly (matrix renames, job renames, nesting)?
4. Run `assert-required-contexts.sh`; read `required \ reported`, not the colour of the rollup.
