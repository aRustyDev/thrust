# thrust

A fully open source project management platform: Cloudflare-native by default, self-hostable by configuration.
See `README.md`.

Procedures live in skills under `.agents/skills/` (`.claude/skills` is a symlink to it). Load the skill named in the
table below before writing that kind of code or test.

## Testing (Rust)

There is no Cargo workspace on `main` yet. These rules and the configs in `.config/nextest.toml` and
`.cargo/mutants.toml` apply to crates written for them. A suite that parses plain `cargo test` output (the
`test result:` line, exit 101) stays on `cargo test` until it is migrated on purpose: nextest exits 100 when tests fail.

1. Run tests with `cargo nextest run`, and `cargo test --doc` for doctests (nextest cannot run them). The profiles in
   `.config/nextest.toml` decide what runs, the timeouts and the retry policy.
2. Never pass `--retries`, `--flaky-result`, `NEXTEST_RETRIES` or `NEXTEST_FLAKY_RESULT`. `flaky-result = "fail"` in
   the config is only a default: `--flaky-result pass` overrides it and turns a retried randomized failure green.
3. A failing test is a finding. Do not delete it, weaken its assertion, `#[ignore]` it, regenerate its fixture or
   snapshot, or raise its timeout to get green. Fix the code, or say in the commit message why the test was wrong.
4. Name tests by category; nextest routes on the prefix: `prop_` property, `model_` model-based, `parity_` golden
   fixtures, `oracle_` live comparison with TypeScript (opt-in), `fuzz_replay_` fuzz-input replay.
5. Keep a crate's tests in that crate: mutation testing runs only the mutated package's own tests.
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
| Mutation testing, one file | `cargo mutants -f <path>` |

A `-E` filter is intersected with the profile's default filter, so `oracle_` tests only run under the `oracle` and
`deep` profiles; a filter that selects nothing exits 4.

When the first crate lands, add the root `Cargo.toml` with `[profile.mutants]` and enable the `profile` line in
`.cargo/mutants.toml` (the steps are in that file). An empty workspace breaks every cargo command, so the manifest
arrives with a real crate, not before.
