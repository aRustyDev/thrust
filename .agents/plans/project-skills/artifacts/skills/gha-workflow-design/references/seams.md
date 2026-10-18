# Seams: reusable workflow, composite action, or job

Tags: "documented" = GitHub docs (retrieved 2026-10-18); `[derived]` = inferred, not executed;
`[unverified: not run]` = not executed. Example files: `examples/github/`.

## The decision

| Unit | Choose it when | In the example |
|---|---|---|
| **Job** (in one workflow) | one-off work with no second caller | `deps`, `mutants`, `plan`, `changes`, `ci-scripts` in `ci.yml` |
| **Composite action** (`.github/actions/<name>/action.yml`) | the same *steps* repeat inside several jobs | `setup-rust`: toolchain, targets, cache, pinned Worker tools |
| **Reusable workflow** (`on: workflow_call`) | the same *whole job* (own runner, timeout, permissions, artifacts) is called from several workflows or in a matrix | `contract-cell.yml`: one contract cell per runner |

Rule of thumb: if the thing needs `runs-on`, `timeout-minutes` or `permissions`, it is a job, so reuse it as a
reusable workflow. If it is a list of steps, reuse it as a composite action.

## Reusable workflow facts

- Declared with `on: workflow_call:` plus `inputs:` (typed), `secrets:`, `outputs:`; called with
  `uses: ./.github/workflows/x.yml` (same repo) or `owner/repo/.github/workflows/x.yml@<ref>` (documented).
  The `uses:` value cannot contain expressions (documented).
- A job that calls one may use `strategy.matrix` and pass `${{ matrix.* }}` through `with:` (documented; the
  example's `contract` job does this and lints clean).
- Permissions "can only be maintained or reduced, not elevated, throughout the chain" (documented). The failure
  mode of a callee asking for more than the caller grants is not described on the page; it is commonly
  reported as a run that fails before any job starts `[derived]`.
- Nesting is limited to ten levels (documented). Keep it at one.
- Inputs arrive as typed values; pass untrusted ones through `env:` in the callee, never into script text
  (`security-hardening.md`).
- `secrets: inherit` hands every secret to the callee (documented; zizmor flags it as `secrets-inherit`
  `[derived]`). Pass only the named secrets.
- The caller's check names nest: the example's cells appear as `<caller job> (<matrix values>) / <callee job
  name>` `[derived]`. Such names change whenever the matrix does, so required contexts must not be the matrix
  jobs (`required-checks.md`).

## Composite action facts

- Lives at `.github/actions/<name>/action.yml`; called as `uses: ./.github/actions/<name>`. The caller must
  have run `actions/checkout` first, because the action files come from the checkout.
- Every `run:` step needs `shell:` (for example `bash`). Composite actions have no `permissions:` and no
  `runs-on`; they inherit the calling job's.
- Inputs are strings. Compare booleans as `inputs.save-cache == 'true'`; pass input values to scripts through
  `env:`, as the example does for the tool versions.
- Nested `uses:` of third-party actions inside a composite are fine and must be SHA-pinned like any other.
- **Validation caveat (actionlint 1.7.12):** actionlint checks the inputs a workflow passes to a *local*
  composite action or reusable workflow only when it can find the repository root (a `.git` directory).
  Outside a git checkout the same broken `with:` block linted clean with exit 0. Run it in a checkout and keep
  a must-fail control (`security-hardening.md`).

## Where the shared logic lives

Prefer scripts in the repo over YAML: `scripts/contract.sh`, `scripts/mutants.sh`, `scripts/check-deps.sh` and
the helpers in `.github/scripts/` run identically on a laptop and in CI, are linted by shellcheck, and can be
tested without GitHub (`examples/github/scripts/tests/`, run by the `ci-scripts` job). The workflow then only
supplies a runner, a cache and an environment.

## Anti-patterns

- A reusable workflow that is called once and never will be again: inline it.
- A composite action that wraps a single `run:` line: inline it.
- A reusable workflow per step: the per-call setup (runner start, checkout, cache restore) is paid every time.
- Passing a whole matrix through `inputs:` as JSON to a callee that fans out internally while the caller also
  fans out: pick one place to fan out.
