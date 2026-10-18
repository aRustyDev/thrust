# Sources: gha-workflow-design

External sources are cited for facts only; no third-party skill text is reproduced. External sources retrieved
2026-10-18. Repository files are cited by path.

## Repository files (assumed contract)

| Path | Supports |
|---|---|
| `scripts/contract.sh`, `scripts/contract-all.sh` | cell list, per-cell invocation, exit-status and vacuity-guard behaviour, serial-run rationale |
| `scripts/flake-hunt.sh` | repeat-a-cell loop; its final `echo` makes the exit status always 0 |
| `scripts/mutants.sh`, `scripts/check-deps.sh` | the `mutants` and `deps` jobs |
| `scripts/rs`, `scripts/worker-dev.sh` | toolchain resolution; Worker cells' tool needs |
| workspace `Cargo.toml` (`[profile.release]`), `crates/thrust-worker/wrangler.toml` | release-profile constraint; the `wrangler` version the repo measured with |

## External sources

| Source | Supports | Licence |
|---|---|---|
| https://docs.github.com/en/actions/how-tos/reuse-automations/reuse-workflows | `workflow_call` syntax, matrix callers, permission narrowing, nesting limit, `secrets: inherit` | CC-BY-4.0 (github/docs) |
| https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax | `concurrency`, `paths`/`paths-ignore`, `permissions` defaults | CC-BY-4.0 |
| https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows | `merge_group`, `workflow_dispatch` limits, `schedule`, `pull_request_target` | CC-BY-4.0 |
| https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/collaborating-on-repositories-with-code-quality-features/troubleshooting-required-status-checks | skipped-by-filter vs skipped-by-`if:` semantics, same-named checks | CC-BY-4.0 |
| https://docs.github.com/en/actions/reference/security/secure-use | SHA pinning, script injection, least privilege | CC-BY-4.0 |
| https://github.com/Swatinem/rust-cache (README) | cache key inputs, `cache-bin`, cleanup, `save-if`, limits | LGPL-3.0 |
| https://github.com/mozilla-actions/sccache-action (README) | usage and required environment | Apache-2.0 |
| https://nexte.st/docs/ci-features/partitioning/ | `hash`/`slice`/`count`, archive reuse, per-test (not per-duration) caveat | Apache-2.0 (nextest-rs/nextest) |
| https://github.com/rhysd/actionlint (`docs/checks.md`) | check categories, shellcheck/pyflakes integration | MIT |
| https://docs.zizmor.sh/audits/ | audit ids named in `security-hardening.md` | MIT (zizmorcore/zizmor) |

## Verification status

| Claim group | Status |
|---|---|
| `actionlint` 1.7.12 over every example workflow in a git checkout, including must-fail controls (bad and missing composite inputs, bad and missing reusable inputs, bad output name, nonexistent `needs:`) and a control kept in `.github/lint-controls/` | executed 2026-10-18, no findings on the examples, all control findings reported |
| `actionlint` outside a git checkout | executed; local-action input errors were not reported |
| `shellcheck` 0.11.0 on `examples/github/scripts/**` | executed, clean |
| `examples/github/scripts/tests/*.sh`, plus mutants of the certifier, gate and classifier | executed; tests pass on the originals and fail on each mutant |
| `cargo-nextest` 0.9.140: `--partition hash`, `slice`, `count`; empty shards and empty filters exit 4; explicit `--no-tests` values; `archive` + `--archive-file` | executed on a minimal 8-test crate |
| `gh api -i` output format and exit status for an HTTP 404; ruleset, rollup, run-list and run-jobs queries | executed read-only against a public repository |
| Action SHAs in the examples | resolved with `git ls-remote` on 2026-10-18; a snapshot |
| `zizmor`; any workflow run on GitHub; `contract.sh` cells; `mutants.sh` | `[unverified: not run]` |
| Runner image contents (`lsof`, `docker`, ...); `startup_failure` causes | `[derived]`; the preflight step guards the former |

## Not reproduced

Floating action refs (`@v4`, `@main`), `secrets: inherit`, `pull_request_target` with a PR-head checkout,
`releases/latest` downloads in CI.
