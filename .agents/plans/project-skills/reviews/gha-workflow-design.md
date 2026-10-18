# Review: gha-workflow-design (round 1)

Reviewer: skill-reviewer. Read all 21 files (~81 KB; SKILL.md 2,844 B); executed nothing from the candidate (only read-only `git ls-remote`, `curl`/`gh api` GETs, WebFetch, YAML syntax parse).

**Verdict: PASS (Section A clean). Grade: B.** Reviewer recommends fixing findings 1-7 before adoption.

## Section A
- `allowed-tools`: Read, Grep, Glob, `Edit(.github/**)`, `Write(.github/**)` (inert), exact `Bash(actionlint)` (static linter; exact match so invocations with arguments prompt). SKILL.md:18 states it authors files under `.github/` only.
- Executable content: all 6 third-party `uses:` SHAs real and matching their tag comments (checkout v7.0.1, download-artifact v8.0.2, upload-artifact v7.0.2, cache v6.1.0, rust-cache v2.9.2 peeled commit 6323deb1, rust-toolchain v1); pinned installs exist (wrangler 4.123.0, worker-build 0.8.7); no `pull_request_target`; no `${{ github.event.* }}` interpolated into `run:` (all via `env:`); `permissions: {}` at workflow level, `contents: read` per job; `persist-credentials: false`; no secrets; POSIX sh with `set -eu`, quoted, guarded globs; `assert-required-contexts.sh` uses the caller's authenticated `gh` for repo-scoped GETs, reads no credential file.
- No remote-fetch-and-execute; no injection text.

## Findings
1. MEDIUM: delete inert `Write(.github/**)` (SKILL.md:9); `Edit` covers Write.
2. MEDIUM: SKILL.md:18 vs :31, :8: prose says `.github/` only but step 1 says put logic in `scripts/` or `.github/scripts/` (`scripts/` is outside the grant); `.github/**` also pre-approves silent edits to CODEOWNERS and workflows; consider `Edit(.github/workflows/**)`, `.github/actions/**`, `.github/scripts/**`, `.github/dependabot.yml`.
3. MEDIUM: the "deliberately broken copy" control cannot work with the declared tools (a broken file left in `.github/workflows/` is live; bare `Bash(actionlint)` does not match `actionlint <file>`): state where the control file goes (e.g. `.github/lint-controls/`), "delete it before finishing", and that `actionlint <file>` prompts; do not widen to a wildcard.
4. MEDIUM: wall-clock.md:28-29 quotes rust-cache as "unsuitable for caching `cargo install` binaries"; not in the README at v2.9.2 or master (README says installed binaries are cached; `cache-bin` defaults to true). Remove the quote; justify the separate worker-build cache on its own merits or drop it.
5. MEDIUM: invariant "PR cells restore, none saves; the example saves only from `refs/heads/main`" is false for the `worker-build` entry (full `actions/cache` saves on any ref when the exact key missed; not a poisoning path since PR-scoped caches are not readable from main). Use `actions/cache/restore` + `actions/cache/save` guarded by `inputs.save-cache == 'true'`, or drop it; hoist `0.8.7`, duplicated in the key and the install line (setup-rust/action.yml:36-42, wall-clock.md:31-33, security-hardening.md:62-63, SKILL.md:36).
6. MEDIUM: `assert-required-contexts.sh:30-32` treats ANY failure of the classic-protection call (403/5xx/network) as "no classic protection", so classic contexts can be dropped and a PR certified on a partial set: the failure the script exists to prevent (doc says it "stops (set -e) instead of treating unknown as none" at required-checks.md:99-101, 110-111; true only for the rulesets call). Capture the HTTP status (`gh api -i`), treat only 404 as "none", fail otherwise; add a 403 test case; correct the doc. (From reading, not execution.)
7. MEDIUM: flake-hunt.yml:55 `cache-key: flake-...` is restore-only and always cold; key by what shares build output (`contract-native` / `contract-wasm`).
8. LOW-MED: ci.yml does not run the two test scripts (the logic behind the required check is not regression-tested); classifier cases and the real `contract-all.sh` plan case only `echo` and cannot fail (tests/test-ci-scripts.sh:17-19, 28-33), undercutting the "Verified" claim (triggers-and-concurrency.md:36).
9. LOW: "rely on nextest's default" is version-dependent (default became `fail`/exit 4 at 0.9.85): pass `--no-tests=fail`; the empty-shard case is untested (wall-clock.md:78-81).
10. LOW: flake-hunt.yml:18-21 `pass` input should be `type: choice`.
11. LOW: assert-required-contexts.sh:11, 28-43 writes temp files into the cwd; use `mktemp -d` and a trap.
12. LOW: required-checks.md:73-74, 104: "same workflow name and job name" (only the check name forms the context); the certifier counts SKIPPED as certified while the thesis is that a skipped required job passes by accident: document or fail SKIPPED for `gate`.
13. LOW: SOURCES.md authoring-session ledger (:31-46; "installed locally" wall-clock.md:64; "zizmor not installed" security-hardening.md:16, 92); state licences (rust-cache LGPL-3.0, sccache-action Apache-2.0); give the current troubleshooting URL.
14. LOW: merge-order dependency: worked example, evals 3 and 4, SOURCES rows 8-14 depend on scripts that exist only on the spike branch; add a "Preconditions" line to contract-matrix-example.md; adopt only after the merge.
15. LOW: add a security eval (e.g. `github.event.pull_request.title` interpolated into `run:`); thin trigger set (6/6).

## Spot-checks: 14 checked; 12 hold (matrix cells and 16 serial cells; flake-hunt exits 0; contract.sh vacuity guard; release profile; path-filtered required checks stay Pending; merge_group semantics; concurrency limits; schedule/dispatch limits; reusable-workflow limits; rust-cache behaviours; nextest partitions; actionlint local-action resolution; zizmor ids). 2 fail: rust-cache "unsuitable" quote (F4); "saves only from main" (F5).

## Collisions: none installed beyond the word "Workflows" in the Cloudflare `wrangler` skill; add "the mdBook Pages deploy workflow (see mdbook)" and "what the contract tests assert (see rust-testing)" to the NOT clause.

## Load: router 2.8 KB; references ~34 KB; examples ~31 KB read on demand; staleness cost is pinned SHAs/versions (marked as a snapshot; correct mitigation).
