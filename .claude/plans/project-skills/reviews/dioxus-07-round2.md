# Review: dioxus-07 (round 2)

Reviewer: skill-reviewer (fresh). Read every file, the round-1 review, PLAN.md and the three sibling SKILL.md files; edited nothing; ran real `dx` read-only (`--help`, `--version`).

**Verdict: PASS (Section A clean, round 2). Grade: B.** Must-fix before adoption (not Section A): finding 1.

## Section A
- `allowed-tools`: Read, Grep, Glob, `Edit(**/src/**)`, exact `Bash(dx --version)`, exact `Bash(cargo check)`. `Edit` rules cover all file-editing tools (`Write(path)` is never consulted); `**/src/**` is the documented any-depth spelling; a skill's grant lasts one turn. `cargo check` as an exact literal has no `--config`/`--manifest-path` injection; the spike workspace has no `build.rs`, proc-macro crate or `build =` key, so the write-then-execute chain does not exist today.
- Pipe-to-shell sweep of all 10 files: no installer reproduced in any form (refusals only: SOURCES.md:67-68, tooling.md:18-19). One descriptive mention of `npx @tailwindcss/cli` (cheatsheet.md:187), not an instruction.
- No credential paths, home paths, usernames, internal names, injection text or first person.

## Round-1 findings 1-11: resolved except 7 (partly: `dx translate` overlap) and 8 (3,508 B, under the plan's 4 KB target).

## Findings
1. HIGH (before adoption): `Edit(**/src/**)` + pre-approved `Bash(cargo check)` is a write-then-build pairing; the permissions doc says rules are matched after stripping wrappers including bare `xargs`, so exact `Bash(cargo check)` may also match `echo ... | xargs cargo check` [unverified: matcher not tested]; SKILL.md:19 "performs no network access" is contradicted (cargo can fetch crates and rewrite Cargo.lock outside the Edit scope); classing is inconsistent (`dx build`/`dx serve` are user-run because they can fetch tools). Fix: delete `- Bash(cargo check)` (SKILL.md:11); the output contract already allows "`cargo check` result or 'not run'".
2. MEDIUM: `dx translate` collision with clone-frontend: dioxus-07 owns "one HTML snippet to rsx" in body/evals but not in its description; clone-frontend names `dx translate` and "HTML to RSX" outright; the same behaviour is documented twice (tooling.md:78 vs clone-frontend/references/translate.md:40). Pick one owner.
3. LOW: tooling.md:55-58 says `dx check --cargo-args/--rustc-args` mean "treat like a build"; in 0.7.10 `dx check` runs an rsx lint over `.rs` files and never compiles; the flags are accepted but unused.
4. LOW: grep gates drifted (SKILL.md:29 scans only `src` and omits `cx\.render`, `ImageAssetOptions`, `"dioxus/axum"`, `register_server_fns`; `use_future\(\(` misses `use_future(cx, (id,), ...)`): use one gate in one place; pattern `use_future\((cx, )?\(`; include `Cargo.toml`.
5. LOW: SKILL.md:52 "read the `to_08` migration page" does not exist (404): "read the 0.8 migration notes when published".
6. LOW: Tailwind wording: a `tailwindcss` on PATH is ignored unless `NO_DOWNLOADS` is set (any value, even `0`) or `no_downloads` is configured; the binary is downloaded and made executable with no checksum check in the code.
7. LOW: SKILL.md:23 user-run list omits `dx init` and `dx run`.
8. LOW: legacy.md:26 misattribution ("to_06 calls it removed"); only the HTML-global `prevent_default` is `#[deprecated]` and does nothing in 0.7.10; the SVG one is not.
9. LOW: cheatsheet.md:63 `document::eval` lacks the native `EvalError::Unsupported` pointer to blitz-native; :97 `use_loader` exported only under the `fullstack` feature.
10. LOW: fullstack.md:68-74 `#[server(protocol = Http<GetUrl, Json>)]` is accepted but ignored in 0.7.10 (`_input_encoding`/`_output_encoding` unused): replace the `[unverified]` tag and delete the example.
11. INFO: "would download and execute" overstates slightly (`deno x` prompts by default; `-y` assumes confirmation).
12. LOW: cheatsheet.md:187 `npx @tailwindcss/cli` descriptive mention: reword "an npm-based CLI flow (not used here)".
13. INFO: PLAN.md:30 still lists `to_owned!` as banned (orchestrator to fix).
14. INFO: "Commands run" in SOURCES.md is acceptable verification evidence (consider "Verification record"); samples "adapted" from unlicensed docs into an AGPL repo is disclosed; the decision is the maintainers'.

## Spot-checks: 28 passed (register_server_functions; `to_owned!` exists; `--all-code` prettyplease; `--check` rewrites; native eval; `prevent_default`; Tailwind; `--hotpatch` control; `dx new` branch/template facts; renderer API match; prelude/features; releases; org facts; llms files; `ReadOnlySignal`; to_06/to_07 rows; `use_action`; server API; `dx serve`/`dx translate` flags). Failed: `dx check` described as build-like; `to_08`; gate pattern coverage.

## Collisions: none installed; clone-frontend collides on single-snippet `dx translate`; design-system and blitz-native clean.
