# Review: dioxus-07 (round 1)

Reviewer: skill-reviewer. Draft read in full (SKILL.md, 5 references + SOURCES.md, 2 eval files). Local `dx` used for `--help`/`--version` only.

**Verdict: REJECT. Grade: UNGRADEABLE (Section A failed).**

## Section A (blocking)
- A2.1: `Bash(cargo check:*)`, `Bash(cargo clippy:*)` (SKILL.md:11-12): `--config` can set `build.rustc-wrapper`/`build.rustc`/linker keys; with unscoped `Edit`/`Write` the agent can author a build script and run it with no prompt.
- A2.1: `Bash(dx fmt:*)`, `Bash(dx check:*)`, `Bash(dx translate:*)` (SKILL.md:14-16): on the authoring machine bare `dx` is Deno's npm runner (`deno x`), and the skill itself warns about it; the grants pre-approve exactly that shadowed case while the safe path (full path) matches no rule. Even with the real CLI: `dx check --cargo-args/--rustc-args` (same trampoline), `dx translate --output <path>`, `dx fmt --file <path>` write anywhere.
- Passed: key/format, provenance, credentials, no injection text, trigger scope, and **no pipe-to-shell installer reproduced in any form** (14 files swept; mentions are refusals only).
- Minimal fix: `allowed-tools` = Read, Grep, Glob, Edit, Write, `Bash(dx --version)` (optionally argument-free exact `cargo check` / `cargo clippy --all-targets -- -D warnings`); everything else prompts.

## Section B/C
1. HIGH factual: `register_server_fn_on_router` does not exist in 0.7.10 (fullstack.md:106-107); registration is `DioxusRouterExt::register_server_functions()`.
2. HIGH factual: `to_owned!` is banned but exists in 0.7.10 (dioxus_hooks docs); drop from banned list, grep gate, eval 1.
3. MEDIUM: `#[server(protocol = Http<GetUrl, Json>)]` presented as working; macro parses it but the value flows to unused fields; "server_fn 0.7" stale. Tag unverified or delete.
4. MEDIUM: `--all-code` is prettyplease, not rustfmt, and likely deletes `//` comments tree-wide; `dx fmt` only on clean git status; single files via `dx fmt --file -`.
5. MEDIUM: run-time network wording vs plan policy; Context7 library id never queried (likely needs a leading slash).
6. MEDIUM: SOURCES.md/tooling.md/legacy.md process narrative and first person; docsite repo declares no licence.
7. MEDIUM: collision with `clone-frontend` on `dx translate`/"turn this html into rsx" (no NOT clause); Tailwind documented in two skills; description lacks custom-renderer words.
8. Load cost 4,086 B: trim to ~3 KB.
9. Evals: depend on false `to_owned!` ban; vacuous ReadOnlySignal/guard-across-await evals; 0.7.10-only `--check` bug pinned; add router/Stores/asset!/translate coverage and harder negatives.
10. `prevent_default` is deprecated/no-op, not removed.
11. Disclose that `dx` downloads a tailwind binary; `dx self-update`/`dx new` user-run only.

## Spot-checks: 21 groups; PASS except #4 (rustfmt), #13 (to_owned!), #19 (register_server_fn_on_router), #20 (protocol), #21 (Context7 id unverifiable).

## Collisions: none with installed skills; clone-frontend (sibling) collides; design-system and blitz-native boundaries clean.
