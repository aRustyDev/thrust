# Findings

Read on `origin/main` at `43a7070` (2026-11-14). Counts marked `[measured]` come from a command run on that commit; the
rest come from reading the files.

## 1. Which tests run against which crates

| Layer | CI job | What it exercises |
|---|---|---|
| Hermetic tests, nextest `ci` profile | `hermetic` | **3922 tests** `[measured: cargo nextest list --profile ci --workspace]`: thrust-core 1582, thrust-compat-plane 894, thrust-adapters-memory 562, thrust-api-v1 545, thrust-adapters-cloudflare 130, thrust-adapters-postgres 92 (TLS, in memory), thrust-api 61, thrust-editor-doc 31, thrust-oracle-fixture 16, thrust-plane-types 9 |
| Doctests | `hermetic` | `cargo test --doc --workspace` |
| Contract suite, 19 cells | `contract` (one `contract-cell.yml` job per cell) | The HTTP surface on four targets (native-memory, native-pg, worker-memory, worker-cf), in passes main, sweeper, panic, csrfoff, cron, worker, bindings. Through it: api, api-v1, compat-plane, every adapter, worker, server |
| v1 oracle, 3 cells | `contract` | api-v1 against the pinned Plane's corpus: Schemathesis, corpus comparison, controls C0 to C4 |
| Browser journeys, 1 cell | `contract` (`worker-cf e2e`) and the daily `e2e-schedule.yml` | The three journeys in `e2e/journeys/` |
| Postgres adapter | `pg-adapter` | thrust-adapters-selfhosted's `pg_adapter` binary (testcontainers) |
| Planted mutants, about 34 | `mutants` (`scripts/mutants.sh`) | Contract-level kills on native-memory and native-pg only |
| Static gates | `deps`, `lint`, `deny` | Dependency, topology, Plane-500, acceptance-list and placement guards; rustfmt; clippy native and wasm32; cargo-deny |

**Crates with no tests of their own:** thrust-contracts, thrust-ports, thrust-adapters-oidc, thrust-worker (G-6),
thrust-server, thrust-editor-worker. thrust-adapters-selfhosted has only its Docker suite. They are tested only through
the contract suite, so `cargo mutants` on any of them stops at the baseline.

**Never run in CI:** the `oracle` and `deep` nextest profiles, the blast, bench, engine, scale and budget passes (engine
and budget only inside `mutants.sh`), stage 2, the probes, the book build, the 24 upstream vitest files under `web/`,
fuzzing (there is no `fuzz/` and no `fuzz_replay_` test), coverage, and `cargo mutants` itself.

`docs/src/testing/status.md`'s inventory table (measured 2026-10-19, "12 tests in the default profile") is stale and
should be re-measured in its own change; it is a large page with many dated claims.

## 2. When and how checks are triggered

| Workflow | Starts on |
|---|---|
| `ci.yml` | every pull request, `merge_group`, push to `main`, dispatch |
| `contract-cell.yml` | `workflow_call` from `ci.yml` and `e2e-schedule.yml` |
| `e2e-schedule.yml` | daily 04:41 UTC, dispatch, a pull request that edits it |
| `flake-hunt.yml` | Mondays 03:17 UTC (`native-memory main` x5), dispatch |
| `deploy.yml` | `workflow_run` of `ci` on `main`, dispatch, a pull request that edits a deploy file (dry run) |

`ci.yml`'s `changes` job runs `classify-changes.sh`, which answers two booleans, `rust` and `web`:

| The pull request changes | Jobs that run |
|---|---|
| only docs, `.agents/`, most of `tools/` | 2 (`changes`, `gate`) |
| only `web/` | 3 (adds `web`) |
| anything Rust-relevant | 32 (10 jobs and 22 cells) |
| a file under `.github/workflows`, `actions` or `scripts` | 33 (adds `web`) |

What is too wide and too narrow is now in [CI/CD](../../../docs/src/architecture/cicd.md#where-the-triggers-are-too-wide-or-too-narrow).
The fix is [PLAN.md](PLAN.md) PR 4.

## 3. Which E2E flows to test

Covered today by `e2e/journeys/`, each with must-fail controls and the must-pass `slow-page`:

- M1: sign up, onboarding, workspace, project with cover, work item, the list after a reload (S01 to S48);
- M2 sign-in: the page offers the password and nothing else;
- M2 change-password: X09, and the other session ends.

To add, in this order, each with a must-fail control and a pinned-Plane leg where the pin serves the flow:

1. **Sign out** (X06). Nothing walks it (G-30); cheap.
2. **Operator temporary password, first sign-in, forced change** (G-34). Walks the locked-out recovery door end to end.
3. **Forgot and reset password by email** (X10, X11; G-37). Needs a Mailpit service in the cell; waits for the SMTP
   adapter (PR #176) and a wired mail service.
4. **OIDC sign-in from the button to the callback** (G-35). Needs `mock_idp` reachable from the browser.
5. **Work-item detail**: description, state, priority, labels, comments, an attachment (X01 to X03). The most valuable
   product flow; blocked by the `description-versions` 404.
6. **Work-item list** (group-by, filters, pagination; S47), then project settings (S32, S34 to S46) and workspace
   members (S15, S18).
7. **M1 against staging after each `rc/*` deploy**, in `deploy.yml` after the smoke. The only browser check of real
   Cloudflare bindings; not per pull request.

Not as E2E: API keys (the contract suite and the v1 oracle cover them) and realtime `/live` (not served).

E2E infrastructure gaps: a `web/`-only pull request does not run the e2e cell (G-30; PR 4 fixes it); the pinned-Plane
leg is local only because the fork is not pushed (G-31); React hydration error #418 is not asserted.

## 4. Which integration points to test

Ranked by what is most likely to break first in a real environment and is least covered.

| Boundary | Covered today | Recommended |
|---|---|---|
| Cloudflare API and wrangler | fakes (`deploy_fakes.py`), the pull-request dry run | The first real staging run failed at `assets-upload-session` (run 43676097831). The `rc/*` staging deploy and the rollback drill become the test (G-36) |
| Access tunnel, `migrate-ci.sh`, verify-ca | a fake `cloudflared`, TLS in memory | A connect-only step on every staging deploy, so the route is proven each time and not only when a migration changes |
| Migrations | run on start; only the 0003 upgrade path has a test (G-9, G-13) | In `pg-adapter`: every migration on an empty database, re-applied (idempotence), and an upgrade from the previous release's schema once releases exist |
| Hyperdrive | wrangler's local emulation, which does not pool | `scripts/stage2-contract.sh` against staging after each `rc/*` deploy, not blocking at first (ran once, by hand, 2026-10-24) |
| Durable Objects, cron, Queues and DLQ, R2, rate limits, Secrets Store | worker-cf cells under `wrangler dev` | Keep. Add a dead-letter queue test (none) and an HTTP test of the sign-in limiter (G-12); real bindings come from the post-deploy stage 2 |
| Postgres adapter | `pg-adapter`, native-pg cells, the native-pg oracle cell | Keep; run the model test under `deep` in a nightly job |
| OIDC identity provider | `mock_idp` on four targets, a scripted provider | Fix the mock IdP's port flake (bind `:0`); the browser journey above; a real provider as a documented manual check |
| Email | the dark path and the simulated binding | Mailpit in the contract and e2e cells once SMTP lands (G-37) |
| The Plane oracles | 3 oracle cells; `parity_` replay; UI and `oracle_` local only | The `oracle` nextest profile nightly from committed output (G-5); push the fork so the UI leg can run in CI (G-25, G-31) |
| GitHub attestations | `verify-bundle.sh`'s tests | Proven end to end by the first `rc/*` deploy and by the first release |

Crates to give hermetic tests of their own first: thrust-adapters-oidc (an auth boundary), then thrust-worker (wasm unit
tests, G-6).

## 5. The gap to the target flow

| Target | Today | Gap |
|---|---|---|
| Work reaches `main` only by pull request | The only ruleset on `main` (id 25418178) requires `gate`; no pull-request rule, no non-fast-forward or deletion rule, no bypass list; no classic protection | Add the pull-request, non-fast-forward and deletion rules |
| Only `rc/*` deploys to staging | A push to `main` (`plan-deploy.sh`) and a manual run from `main` deploy staging; `verify-bundle.sh` requires `--source-ref refs/heads/main`; `deploy.yml` filters `workflow_run` to `main`; no environment has a branch policy | Move every check to `rc/*`; branch policies on the staging environments |
| Each merge updates `CHANGELOG.md` on `rc/vX.Y.Z` | No git-cliff, `cliff.toml`, changelog or version (`0.0.0`); commit subjects are Conventional by convention only | A `cliff.toml` and a bot workflow that keeps `rc/vX.Y.Z` as `main` plus one release commit |
| Only bots and admins touch `rc/*` | No `rc/*` ruleset; the workflow token is read-only and cannot start other workflows | An `rc/*` ruleset with a GitHub App and admins as the only bypass actors |
| Merging `rc/*` cuts a release with provenance | No tag or release workflow, no tag ruleset, immutable releases off; the only tag is `spike/0002-worker-do`; provenance exists only for the deploy bundle | A release workflow, a `v*` tag ruleset, immutable releases, the release's own attestation and SBOM |

Other blockers found on the way: the production environments have no reviewers, variables or secrets; staging lacks
`SMOKE_EMAIL`, `SMOKE_WORKSPACE` and `SMOKE_PASSWORD`; `THRUST_DEPLOY_ENABLED` is unset.
