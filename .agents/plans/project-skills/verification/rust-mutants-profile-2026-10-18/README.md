# Verification pack: the mutants profile and the hermetic default filter (2026-10-18)

Evidence for the change that adds `[profile.mutants]` to the root `Cargo.toml`, enables `profile = "mutants"` in
`.cargo/mutants.toml`, excludes the contract-suite crate from mutation, and excludes the two non-hermetic suites from
the default nextest profile. Run against the real workspace that landed on `main` with the Rust-on-Workers spike
(11 crates), in a worktree. Environment: macOS (Apple silicon), rustc/cargo 1.98.1, cargo-nextest 0.9.140,
cargo-mutants 27.1.0, Docker running (OrbStack). Paths are scrubbed; `Compiling` lines were dropped. Each script
writes every command's exit status into its own log (`[label] exit=N`); the script's own exit code means nothing.

| Log | What it shows |
|---|---|
| `logs/before-default-run.log` | **Before the change**, `cargo nextest run --workspace --no-fail-fast` (the command `AGENTS.md` called the hermetic inner loop): 33 tests, 6 passed, 27 failed. The 27 are `thrust-contract-tests` panicking on `THRUST_BASE_URL must be set`; the 6 that passed are the `pg_adapter` tests, which started a Postgres container through Docker. Run directly, not by a script |
| `logs/verify-mp.log` (`scripts/verify-mp.sh`) | **After the change.** P1: the manifest parses and holds `[profile.mutants]`. P2: the default profile and `ci` select no tests (5 binaries skipped by the default filter) and exit 4 (`no tests to run`); an `-E` filter for the excluded suite intersects to nothing, exit 4. P3: `--ignore-default-filter -E 'binary(pg_adapter)'` lists the 6 adapter tests. P4: `cargo mutants --list` drops from 611 to 561 mutants, none in `thrust-contract-tests`. P5: `cargo mutants -f crates/thrust-contracts/src/lib.rs` builds under `--cargo-profile=mutants` (the profile works), then the baseline fails with exit 4 because the crate has no tests (`no tests to run`; cargo-mutants prints "cargo test failed in an unmutated tree") |
| `logs/optin-pg-adapter-run.log` | The opt-in command written into `AGENTS.md`, run directly: `cargo nextest run --workspace --ignore-default-filter -E 'binary(pg_adapter)'`: 6 run, 6 passed (Docker up) |

Not run: the contract suite against a target (it needs a running server or wrangler), any nextest or cargo-mutants run
on the 0.9.148 recommended version, `cargo mutants` on a crate that has its own tests (none exists in this workspace),
and the wasm targets.
