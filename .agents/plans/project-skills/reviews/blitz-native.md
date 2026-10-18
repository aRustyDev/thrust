# Review: blitz-native (round 1)

Reviewer: skill-reviewer. Draft read in full (SKILL.md, 5 references, 2 eval files).

**Verdict: REJECT. Grade: UNGRADEABLE (Section A failed).**

## Section A (blocking)
- A2.1/A1 overly broad tool permissions (SKILL.md:4-12): unscoped `Edit` + `Write` with `Bash(cargo check:*)`. `cargo check` runs build.rs/proc-macros and honours `--config`/`.cargo/config.toml`, so with unscoped Write the agent can author and run code without a prompt. Nothing in the skill tells the agent to write files or run `cargo check`; `Bash(dx --version)` is unused.
  Fix: `Read`, `Grep`, `Glob`, exact `Bash(cargo tree -p dioxus-native)`, `Bash(cargo tree -i blitz-dom)`.
- Passed: credentials, injection text, provenance, remote content (the jq pointer in css-support.md is a read-only filter over a user-downloaded file; "this skill performs no network access" is stated).

## Section B/C
2. Trigger precision: bare "blitz" / "native renderer" fire on Blitz.js, chess, React Native; anchor on blitz-dom / dioxus-native / `--renderer native`; add negatives.
3. "the brief's 0.3.0-beta.1" points at an authoring input; the claim is true of dioxus-native 0.8.0-alpha.1 only (alpha.2 requires =0.3.0-beta.3). Add rows, reword.
4. SOURCES.md process/tooling leaks (spend limit, Context7 auth, "the brief", first person, "deliberately not used" third-party skills): source, date, licence only.
5. 48-row WPT table is a dated snapshot: move to a dated file or cut to ~10 rows; keep run-395 table primary.
6. jq prints upstream commit messages into context: project to `{date, product_revision}`.
7. Put the no-network caveat in SKILL.md, not only in css-support.md.
8. "pins" column mislabeled (caret requirements; wgpu/vello are workspace-root).
9. Issue #119 groupings: drag-and-drop, text-overflow, body/html events are under 0.3 Beta; IME/clipboard are upstream "?".
10. Drop the "103 open issues" count.
11. Evals: promote focus-trap claim to verified (dioxus-primitives dialog.rs calls `createFocusTrap`); repo moved to DioxusLabs/dioxus-components; eval 1 expectation 4 is not skill-derived; add `dx serve --renderer native` boundary case.
12. Load cost 3,719 B vs benchmark; make crate list pointer-only, target <= 2.8 KB; add a worked example answer.

## Spot-checks: 12 groups, all CONFIRMED (dioxus-native 0.7.x requires blitz-dom ^0.2.4; 0.8.0-alpha series pins =0.3.0-*; README quotes; Stylo/Taffy/Parley/Vello roles; blitz-wpt-results figures digit-for-digit; roadmap issue #119; dioxus-native features and API).
Not verified: blitz.is status page contents, AGENTS.md renderer prose, `dx serve --help` text, transitive licences, runtime behaviour.

## Collisions: none with installed skills. Residual overlap with dioxus-07 on `dx serve --renderer native` (finding 11).
Side note from reviewer: dioxus-07 and dioxus-design-system declare the same Edit/Write/`cargo check:*` pattern; reviewed separately.
