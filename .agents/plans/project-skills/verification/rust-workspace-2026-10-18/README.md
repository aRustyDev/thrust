# Verification pack: the workspace configs (2026-10-18)

Evidence behind the `[re-run]` tags in `.cargo/mutants.toml` and the claims in `AGENTS.md`. Everything ran in a scratch
workspace outside the repo, using the exact config files from this change (copied unchanged) plus a throwaway probe
crate. Environment: macOS (Apple silicon), rustc/cargo 1.98.1, cargo-nextest 0.9.140, cargo-mutants 27.1.0. Paths are
scrubbed (`$SCRATCH`, `$WORKTREE`, `$REPO`, `$HOME`); `Compiling` lines were dropped. Each script writes every command's
exit status into its own log (`[label] exit=N`); the script's own exit code means nothing.

| Script and log | What it shows |
|---|---|
| `verify-ws` | W1: the workspace with `members = []` and no crates: `cargo metadata` exits 0, but `cargo build` and `cargo test --doc --workspace` exit 101 and `cargo nextest run` exits 102, all with "The manifest is virtual, and the workspace has no members". This is why the root manifest is not part of this change. W3 to W5: profile selection (default and ci skip `oracle_`, `oracle` runs only `oracle_`, `deep` runs `prop_`, `model_` and `oracle_`, `mutants` inherits the default filter, `e2e` is absent: exit 96) and retry behaviour (a `prop_` probe fails under `--retries 2`, passes under `--retries 2 --flaky-result pass`, a plain probe is FLAKY and passes). W6's `cargo mutants` run exited 4 because the probe's flaky test file failed the baseline: a probe mistake, redone in `verify-ws2` |
| `verify-ws2` | W6 redone without the flaky file: the `exclude_globs` control (with the config no `fuzz_harness.rs` mutants; without it five), and 7 of 7 mutants caught through nextest with `--cargo-profile=mutants --profile mutants` |
| `verify-ws3` | The trap behind the commented-out `profile` line: with `profile = "mutants"` set and no `[profile.mutants]` in the root manifest, the baseline fails with "profile `mutants` is not defined" and exit 4; with the line commented out cargo-mutants runs (7 of 7 caught) |
| `verify-ws4` | The final files as committed, in a workspace with a probe crate and no `[profile.mutants]`: default, ci, deep, oracle and a `prop_` category filter run what they should; an empty filter exits 4; doctests pass; 7 of 7 mutants caught; no "recommends nextest" notice |

Not run: the `required = "0.9.131"` floor (the archive's claim; only 0.9.140 was used), `--profile ci` junit output in this
workspace, and anything with a real crate from the spike branches.
