# thrust

A fully open source project management platform: Cloudflare-native by default, self-hostable by configuration.
See `README.md`.

Procedures live in skills under `.agents/skills/` (`.claude/skills` is a symlink to it). Load the skill named in the
table below before writing that kind of code or test.

## Code intelligence (Serena, GitNexus)

Their configuration lives in `.agents/config/` and is reached through symlinks, because neither tool has a per-repo
setting for its location (why, and what was rejected: `.agents/config/README.md`).

- Serena: `.agents/config/serena/project.yml`, symlinked at `.github/config/serena`.
- GitNexus: `.agents/config/gitnexus/gitnexusrc.json`, symlinked at `.gitnexusrc`. It sets `indexOnly`, so a plain
  `gitnexus analyze` never rewrites `AGENTS.md`, `CLAUDE.md` or `.claude/skills/`. The index is generated into `.gitnexus/`
  (ignored). The registry is keyed by path, and from a feature branch `analyze` warns that `main` is not the primary
  index, so index a checkout of `main` that stays, not a worktree you will prune.

## Testing (Rust)

The Cargo workspace root is `Cargo.toml` (members are listed explicitly). These rules and the configs in
`.config/nextest.toml` and `.cargo/mutants.toml` apply to crates written for them.

Today every test in the workspace is a non-hermetic integration suite: the contract suite (`thrust-contract-tests`,
run by `scripts/contract.sh`, needs a running target) and the Postgres adapter tests
(`thrust-adapters-selfhosted/tests/pg_adapter.rs`, need Docker). They stay on `cargo test` through those scripts,
which parse its `test result:` line and exit 101 (nextest exits 100 when tests fail); migrate them on purpose, not
by accident. The default nextest profile excludes both, so `cargo nextest run` selects nothing and exits 4
(`no tests to run`) until a crate has hermetic tests of its own. That exit is not a test failure.

1. Run tests with `cargo nextest run`, and `cargo test --doc` for doctests (nextest cannot run them). The profiles in
   `.config/nextest.toml` decide what runs, the timeouts and the retry policy.
2. Never pass `--retries`, `--flaky-result`, `NEXTEST_RETRIES` or `NEXTEST_FLAKY_RESULT`. `flaky-result = "fail"` in
   the config is only a default: `--flaky-result pass` overrides it and turns a retried randomized failure green.
3. A failing test is a finding. Do not delete it, weaken its assertion, `#[ignore]` it, regenerate its fixture or
   snapshot, or raise its timeout to get green. Fix the code, or say in the commit message why the test was wrong.
4. Name tests by category; nextest routes on the prefix: `prop_` property, `model_` model-based, `parity_` golden
   fixtures, `oracle_` live comparison with TypeScript (opt-in), `fuzz_replay_` fuzz-input replay.
5. Keep a crate's tests in that crate: mutation testing runs only the mutated package's own tests. Today's
   workspace does not (its tests are in `thrust-contract-tests`), so `cargo mutants` on a crate with no tests of its
   own stops at the baseline with exit 4; the contract suite's own mutation controls are `scripts/mutants.sh`.
6. The default profile is hermetic and deterministic: no network except an in-process server on `127.0.0.1:0`, no
   sleeps, no wall clock, no files outside a temp dir; inject time and randomness. Suites that need Docker, a
   database, wrangler or Node are opt-in profiles or scripts, never the default loop.
7. Commit test data: `*.proptest-regressions`, `tests/parity/fixtures/`, `fuzz/seeds/`, `fuzz/regressions/`.
   Never commit `mutants.out*`.
8. A check that cannot fail is not evidence. Show the control that makes it fail (a mutated fixture, a broken codec,
   an empty seeds directory) before trusting a pass.

| When you are | Load |
|---|---|
| Writing example, property, model or parity tests for a unit of logic; a flaky `prop_` test; moving to nextest | `rust-test-logic` |
| Porting a TypeScript module (freezing its behaviour, fixtures, type mapping) | `ts-to-rust` |
| Writing code that parses outside bytes or text | `rust-test-fuzz` |
| Finishing a module, reviewing a PR, or asking whether the tests are strong enough | `rust-test-mutation` |
| Testing an axum router or a Dioxus component | `rust-test-app` |

## Commands

| Purpose | Command |
|---|---|
| Inner loop (hermetic tests only) | `cargo nextest run` |
| One category | `cargo nextest run -E 'test(/\bprop_/)'` |
| Doctests | `cargo test --doc --workspace` |
| CI | `cargo nextest run --profile ci --workspace`, then the doctests |
| Long randomized run (includes `oracle_`) | `PROPTEST_CASES=4096 cargo nextest run --profile deep` |
| Live comparison with TypeScript (needs Node) | `cargo nextest run --profile oracle` |
| Postgres adapter tests through nextest (needs Docker) | `cargo nextest run --ignore-default-filter -E 'binary(pg_adapter)'` |
| Mutation testing, one file (the crate needs tests of its own) | `cargo mutants -f <path>` |

A `-E` filter is intersected with the profile's default filter, so `oracle_` tests only run under the `oracle` and
`deep` profiles, and the two integration suites only with `--ignore-default-filter`; a filter that selects nothing
exits 4.
