---
name: gha-workflow-design
description: Use when designing or fixing the GitHub Actions workflows for the Rust workspace - "Expected - Waiting for status to be reported" on a required check, a run with `startup_failure` and zero jobs, "reusable workflow or composite action?", adding `merge_group`, `paths` filters or `concurrency`, CI is slow (`Swatinem/rust-cache`, `cargo nextest --partition`), actionlint or zizmor findings, `permissions:` and SHA pinning. NOT for Cloudflare Workflows, the Dockerfile (see dockerfile), Helm charts (see helm-chart), the mdBook Pages deploy workflow (see mdbook), what the contract tests assert (see rust-testing), or GitLab, Jenkins and release automation.
allowed-tools:
  - Edit(.github/workflows/**)
  - Edit(.github/actions/**)
  - Edit(.github/scripts/**)
  - Edit(.github/dependabot.yml)
  - Bash(actionlint)
author: aRustyDev
version: 0.1.0
---

# GitHub Actions workflow design

Workflows are wiring; logic lives in repo scripts. One stable required check (`gate`) proves the **planned
set** ran, never "nothing failed". The `Edit` rule covers all file writes and pre-approves only the four
`.github/` paths above: edits to `scripts/` or any other file prompt.

**Preconditions (assumed contract):** the `scripts/contract*.sh`, `flake-hunt.sh`, `check-deps.sh` and
`mutants.sh` scripts behave as in `references/contract-matrix-example.md`. `rust-testing` defines the contract
suite; `rust-on-workers` the Worker toolchain.

| Question | Read |
|---|---|
| Reusable workflow, composite action or job? | `references/seams.md` |
| `paths`, `concurrency`, `merge_group`, dispatch, schedule, affected-only | `references/triggers-and-concurrency.md` |
| Slow CI: caching, fan-out, nextest sharding | `references/wall-clock.md` |
| Required checks, stubs, `startup_failure`, certifying a PR | `references/required-checks.md` |
| `permissions:`, SHA pins, injection, actionlint/zizmor | `references/security-hardening.md` |
| Worked example (`references/examples/github/`) | `references/contract-matrix-example.md` |

## Procedure

1. Choose the seam; put logic in scripts, not inline YAML.
2. Include `merge_group`; never path-filter a *required* workflow: use a fail-open `changes` job.
3. One `gate` job (`if: always()`, `needs:` all) asserts each planned unit reported success; require only it.
4. Fan out per independent unit; PR runs restore caches, only the default branch saves.
5. `permissions: {}` at top, least per job, full-SHA pins, `persist-credentials: false`, inputs via `env:`.
6. Lint with `actionlint` in a git checkout, plus a broken copy that must fail (in `.github/lint-controls/`,
   never under `workflows/`; delete it afterwards; `actionlint <file>` prompts).

## Output contract

Files under `.github/`, the `actionlint` result, one negative control, and what was run versus only written.
Tags: `[unverified: not run]` = command not executed; `[derived]` = from documentation, not executed.

## Example

Q: "docs-only PRs hang on the required check." A: a path-filtered workflow never reports. Drop the filter, add
`changes` and a `gate` that accepts `skipped` only when nothing relevant changed; require `gate` only.
