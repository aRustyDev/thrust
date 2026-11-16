# crates-layout: plan

Four pull requests. PR 1 is independent and can land at any time. PR 2 changes no layout: it makes the code correct at
any directory depth, so PR 3 is only renames and path edits. PR 4 adds the guard that keeps ADR-0009's rules.
Counts and line numbers below are from `main` at `ce84d85` (2026-11-15); recount them when each PR starts.

## The mapping

| From | To | Package (unchanged) |
|---|---|---|
| `crates/thrust-adapters-cloudflare` | `crates/adapters/cloudflare` | `thrust-adapters-cloudflare` |
| `crates/thrust-adapters-memory` | `crates/adapters/memory` | `thrust-adapters-memory` |
| `crates/thrust-adapters-oidc` | `crates/adapters/oidc` | `thrust-adapters-oidc` |
| `crates/thrust-adapters-postgres` | `crates/adapters/postgres` | `thrust-adapters-postgres` |
| `crates/thrust-adapters-selfhosted` | `crates/adapters/selfhosted` | `thrust-adapters-selfhosted` |
| `crates/thrust-api` | `crates/api/core` | `thrust-api` |
| `crates/thrust-api-v1` | `crates/api/v1` | `thrust-api-v1` |
| `crates/thrust-compat-plane` | `crates/api/compat-plane` | `thrust-compat-plane` |
| `crates/thrust-plane-types` | `crates/api/plane-types` | `thrust-plane-types` |
| `crates/thrust-ports` | `crates/core/ports` | `thrust-ports` |
| `crates/thrust-contracts` | `crates/core/types` | `thrust-contracts` |
| `crates/thrust-core` | `crates/core/usecases` | `thrust-core` |
| `crates/thrust-editor-doc` | `crates/editor/doc` | `thrust-editor-doc` |
| `crates/thrust-editor-worker` | `crates/spikes/editor-worker` | `thrust-editor-worker` |
| `crates/thrust-contract-tests` | `crates/testing/suites/api` | `thrust-contract-tests` |
| `crates/thrust-oracle-fixture` | `crates/testing/fixtures/oracle` | `thrust-oracle-fixture` |
| `crates/thrust-server` | `crates/server` | `thrust-server` |
| `crates/thrust-worker` | `crates/worker` | `thrust-worker` |

## PR 1: the `regression_` test-name prefix

Owner decision (2026-11-15): a pinned regression's test name must contain `regression_`, and the one-line comment that
says which bug it pins stays.

- **AGENTS.md.** Rule 4 adds `regression_` to the prefix list. Rule 9's case (a) requires the prefix as well as the
  comment. `docs/src/testing/rules.md` includes AGENTS.md (`{{#include ../../../AGENTS.md:testing-rules}}`), so there is
  no copy to edit; check `docs/src/testing/glossary.md` and `techniques/example-and-parameterized.md` for wording that
  describes the old rule.
- **The filterset is `test(/regression_/)`, without `\b`.** An rstest case `#[case::regression_issue_39]` generates a test
  named `case_<n>_regression_issue_39`, and `\b` does not match between `_` and `r`. The `\b(prop|model)_` style of
  `.config/nextest.toml` therefore cannot be copied here.
- **Rename the existing pinned regressions.** Found by comment on `ce84d85`:
  `adapters-postgres/src/lib.rs:3495`; `adapters-selfhosted/tests/counting/mod.rs:313,347,370,391`;
  `adapters-selfhosted/tests/pg_adapter.rs:176,377,755,975,1438,2211,2747` (upgrade-path regressions: read each to
  confirm); `contract-tests/tests/contract.rs:80`; `contract-tests/tests/model/mod.rs:1343` (Rule 9a). The grep was for
  "pinned regression" and `case::…regress…`, so it can miss one described in other words; say so in the PR.
- **Renaming a contract-suite test.** `scripts/contract.sh` selects passes by substring (`sweeper_`, `csrf_off_`,
  `panic_`, `blast_`, `worker_room_`, `worker_bindings_`, `worker_cron_`) and checks an expected passed count per pass. A
  new name must not contain one of those substrings, and every pass's count must be unchanged.
- **`.config/nextest.toml`.** Add `regression_` to the header's list of prefixes. It runs in the default profile, so no
  filter changes.
- **Verify.** `cargo nextest list -E 'test(/regression_/)'` lists exactly the renamed tests: zero before the PR, all of
  them after. That before-and-after difference is the control that the filter is not vacuous.

## PR 2: paths that work at any depth (before the move)

Behaviour-preserving on today's flat layout. CI goes green with nothing moved.

1. **A workspace-root variable instead of `../..`.** Add `.cargo/config.toml` with
   `[env] THRUST_ROOT = { value = "", relative = true, force = true }` (measured in PR 2: `""` gives the root with a
   trailing slash, `"."` gives `<root>/.`, and without `force` a THRUST_ROOT already in the environment wins; Cargo reads
   the file from the directory it runs in, not from `--manifest-path`), and rewrite the 12 Rust sites that climb out of
   a crate to `concat!(env!("THRUST_ROOT"), "/…")`:
   - `thrust-adapters-postgres/tests/tls.rs:507` (`include_bytes!`, file-relative)
   - `thrust-api-v1/tests/openapi.rs:49`, `tests/oracle_seed.rs:37`
   - `thrust-editor-doc/examples/fidelity.rs:114`, `tests/controls.rs:218`, `tests/decode_panics.rs:87`,
     `tests/entities_fuzz.rs:9`, `tests/sanitize.rs:117`, `tests/stage3_fuzz.rs:21`
   - `thrust-oracle-fixture/tests/parity.rs:21,45`
   - `thrust-plane-types/tests/parity.rs:44`

   These are not climbs and stay as they are: test data strings (`"../../etc/passwd"` in `thrust-core`,
   `"../../sneaky/…"` in `thrust-compat-plane`) and `thrust-core/src/auth/key_hash_tests.rs`'s `#[path]`, which stays
   inside the crate. The five `wrangler*.toml` of `thrust-worker` keep `../../scripts/rs`, because `crates/worker` is at
   the same depth; only `thrust-editor-worker/wrangler.toml` changes, in PR 3.

   Verify: the `oracle` profile (several of these tests are `oracle_` and do not run by default; they need
   `tools/spike-0006/gen-all.sh`'s output), the Postgres TLS test, and one cargo-mutants run on a crate with a rewritten
   site. cargo-mutants builds in a copy of the tree, and `THRUST_ROOT` must point into that copy.
2. **Every internal dependency through `[workspace.dependencies]`.** Five sibling paths remain in member manifests:
   `thrust-editor-doc` in `thrust-api-v1`, `thrust-compat-plane` and `thrust-editor-worker`, and `thrust-oracle-fixture`
   in `thrust-adapters-selfhosted` and `thrust-api-v1`. Add both crates to the root's `[workspace.dependencies]` and
   switch the members to `{ workspace = true, … }`. Cargo ignores a member's `default-features = false` unless the
   workspace entry has it, and all three users of `thrust-editor-doc` turn its defaults off, so its workspace entry
   does too. Verify: `cargo tree -e features` for each of the four dependents is identical before and after. After this,
   only the root `Cargo.toml` names crate paths. Update `docs/src/contributing/crates.md` step 3, which says the editor
   crates are not declared that way.
3. **`plan-ci.py` maps a file to the member whose directory is its longest prefix**, not to its first two path segments
   (`top = "/".join(path.split("/")[:2])`). The rule `^crates/[^/]+/` becomes `^crates/`. A group `README.md` that no
   member owns needs no jobs; anything else under `crates/` that no member owns stays FULL.
   `.github/scripts/tests/test_plan_ci.py` gains nested graphs (members two to four levels deep, and two members whose
   directories share a prefix, such as `crates/api/v1` and `crates/api/v1x`). Its check "every crate directory is
   `crates/<name>`" (line 218) becomes "every crate directory is a member's directory from `cargo metadata`".
4. **`check-deps.sh`'s ID-minting guard exempts each member's own `tests/`**, read from `cargo metadata`, instead of
   `^crates/[^/]+/tests/` (line 122). A looser regex such as `^crates/.*/tests/` would also exempt a `src/tests/` module
   and would stop checking it without saying so. Its self-test, if it has one, gains a nested case.

## PR 3: the move

Start it when #194 (the release PR) and #201 (the migration ledger) have merged, and no other open PR touches
`crates/`. Any branch alive across the move rebases onto it: git's rename detection carries edits to moved files, but a
file a branch *adds* under an old directory lands at the old path.

1. **Commit 1: renames only** (`git mv` per the mapping), so review and `git log --follow` see 100 % renames.
2. **Commit 2: path edits.**
   - Root `Cargo.toml`: `members` and `[workspace.dependencies]`.
   - `crates/spikes/editor-worker/wrangler.toml`: `../../../scripts/rs`.
   - The 45 files outside the docs that name a crate path (list below). The ones that fail *silently* if missed:
     `.github/workflows/deploy.yml` (a `paths:` filter naming `crates/thrust-worker/wrangler.deploy.toml` would never
     fire again), `.cargo/mutants.toml` (a stale exclude glob would start mutating the contract suite) and `.gitignore`
     (build output would show as untracked). `scripts/check-topology.sh` and `scripts/check-placement.sh` default to
     `crates/thrust-worker`; check-topology already fails on "nothing to check", but confirm check-placement does too.
   - The 27 documentation files (list below), and `AGENTS.md` (it names
     `thrust-adapters-selfhosted/tests/pg_adapter.rs`). Rewrite `docs/src/contributing/crates.md`'s "Adding a crate"
     (pick a group, or the top level for a program; update the group's README) and "Not part of the product graph"
     (the editor Worker is in `spikes/`; `thrust-editor-doc` is product code, used by `thrust-api-v1` and
     `thrust-compat-plane`).
   - ARCHITECTURE §2a: redraw the tree from ADR-0009.
3. **Commit 3: READMEs.** `crates/README.md` (the groups, ADR-0009's rule 1, the reserved program names),
   `adapters/`, `api/`, `core/`, `editor/`, `spikes/` (when a spike goes here and when it goes in `probes/`) and
   `testing/` (ADR-0009's rule 7: what goes here, what stays in the crate, what is not a crate).
4. **Separate commit:** `thrust-editor-doc`'s description still begins "Spike 0006"; say what it is now.

Verify:
- `cargo metadata --format-version 1 --no-deps` before and after: the same 18 package names and the same dependency
  edges (name, kind, features, target), differing only in `manifest_path`. Write the comparison as a small script and
  show it failing on a planted difference first (rule 8).
- `git grep -n 'crates/thrust-' -- . ':!.agents/plans' ':!CHANGELOG.md'` finds nothing.
- CI runs everything (the root `Cargo.toml` changed).
- Locally: `cargo nextest run --profile oracle`; `scripts/rs worker-build` in `crates/worker` and
  `crates/spikes/editor-worker`; `scripts/check-deps.sh`, `check-topology.sh`, `check-placement.sh`; the scripts in
  `.github/scripts/tests/`; one `scripts/contract.sh` cell; `scripts/mutants.sh`.
- Every literal path under `on.*.paths` in `.github/workflows/deploy.yml` exists (`git ls-files --error-unmatch`).

After merge: re-index GitNexus from the canonical checkout of `main` (AGENTS.md: index `main`, not a worktree), and
prune this plan's worktrees.

### Files outside the docs that name a crate path (45)

`.cargo/mutants.toml`, `.gitignore`, `Cargo.toml`;
`.github/actions/build-bundle/action.yml`, `.github/actions/deploy-worker/action.yml`, `.github/workflows/deploy.yml`,
`.github/scripts/classify-deploy.sh`, `.github/scripts/tests/{test_plan_ci.py, test-classify-deploy.sh,
test-deploy-template.sh, test-deploy-worker.sh}`;
`crates/thrust-adapters-selfhosted/tests/counting/mod.rs`, `crates/thrust-api/Cargo.toml`;
`probes/ammonia-400/{run.sh, src/main.rs}`;
`scripts/{check-acceptance-list.sh, check-deps.sh, check-placement.sh, check-plane-500.sh, check-topology.sh,
contract-all.sh, deploy-worker.sh, measure-argon2.sh, migrate-ci.sh, mutants.sh, oracle-v1-reset.sh, render-wrangler.sh,
smoke-deploy.sh, spike-0002-pg.sh, startup-check.sh, worker-dev.sh}`;
`tools/compat-record/{gen-acceptance.py, gen-reserved-slugs.py, gen-slugify.py, gen-timezones.py, normalise.py,
record.py}`; `tools/measure-zxcvbn/cloudflare/measure-cpu.sh`,
`tools/measure-zxcvbn/patches/{variant-called-nodefault.patch, variant-called.patch, variant-uncalled.patch}`;
`tools/spike-0006/{bench/deep-probe.sh, bench/run-bench.sh, gen-all.sh, sanitize/run-all.sh}`.

The `measure-zxcvbn` patches apply to the old paths, so they are regenerated rather than edited by hand. The `plan-ci.py`
and `check-deps.sh` patterns that assume one level are PR 2's.

### Documentation files that name a crate path (27)

`.github/deploy/README.md`, `tools/measure-zxcvbn/README.md`;
`docs/src/architecture/{cicd.md, deploy.md, infra.md}`; `docs/src/contributing/{crates.md, features.md, patterns.md,
standards.md}`; `docs/src/flows/app.md`;
`docs/src/testing/{by-change.md, evidence-and-controls.md, glossary.md, running.md, status.md, strategy.md}`;
`docs/src/testing/techniques/{architecture-gates.md, contract-suite.md, differential.md,
example-and-parameterized.md, fuzz.md, golden-fixtures.md, model-based.md, mutation.md, performance.md, property.md,
real-backends.md}`.

Not edited: `.agents/plans/**` (dated records, including accepted ADRs such as ADR-0042 that cite line numbers) and
`CHANGELOG.md`.

## PR 4: the layout guard

`scripts/check-layout.sh`, run by CI's `deps` job, with a `--self-test` that plants each violation and must fail:
- every directory under `crates/` is a workspace member or a group with a `README.md`, and no member contains another;
- every workspace member is under `crates/`;
- no crate has a normal or build dependency on a crate under `crates/testing/` or `crates/spikes/` (from
  `cargo metadata`; dev-dependencies are allowed);
- nothing in `crates/core/` depends on a workspace crate outside it.

Optional, in the same PR or later: every literal path in a workflow's `paths:` filter exists, so a renamed file cannot
silently stop a workflow from triggering.

## Left for later (not this plan)

- **The contract suite onto nextest.** It, `pg_adapter` and `smtp_sink` run on `cargo test` through scripts that parse
  its `test result:` line. Moving it means rebuilding ADR-0121's passed-count guard on nextest's output (JUnit, or
  libtest JSON). nextest's setup scripts could start the target, but they are experimental
  (`experimental = ["setup-scripts"]`). Doctests stay on `cargo test --doc`; nextest cannot run them.
- **`testing/suites/load`.** It takes the `bench` and `bench_scale` binaries out of `thrust-contract-tests`, which is a
  code change.
- **`testing/profiling/`.** It is created with its first cross-crate workload. Its pieces:
  - nextest's wrapper scripts (experimental since 0.9.98, `experimental = ["wrapper-scripts"]`) run selected tests under
    a profiler, one wrapper per tool;
  - `cargo nextest bench` (experimental since 0.9.117) runs Criterion benches;
  - `--tracer` (stable since 0.9.114) gives strace or dtruss for a single test.

  A wrapper that never runs the test binary makes the test pass without running it, so each wrapper needs a must-fail
  control. A wrapper profiles the test process, so profiling a running server means starting the *server* under the
  profiler instead. Native profilers cannot see the Worker in workerd; perf and eBPF are Linux-only; nothing in Thrust
  runs on a GPU today.
- **New programs.** `edge/` (#195 stage C), `collab/` (M5), and `cli/`, `tui/`, `desktop/`, `webui/`, `mobile/`, each
  created with its first real code.
