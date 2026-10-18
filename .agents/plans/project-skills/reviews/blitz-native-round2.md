# Review: blitz-native (round 2)

Reviewer: skill-reviewer (fresh). Read all files incl. the dated WPT snapshot; executed nothing from the candidate (the three jq filters were run against live public data and work).

**Verdict: PASS. Grade: B.** Section A clean: `allowed-tools` = Read, Grep, Glob, exact `Bash(cargo tree -p dioxus-native)`, exact `Bash(cargo tree -i blitz-dom)`, exact `Bash(dx --version)`; no wildcard, no Edit/Write; `cargo tree` cannot run build.rs/proc-macros.

Round-1 findings: 1-3, 5-7, 9-11 resolved; 4 and 12 partly (SOURCES "Commands run" log and one search-API phrase remain; load cost 3,688 B); 8 mostly (a loose "inherits" sentence); 13 mostly.

## Optional follow-ups (none changes the verdict)
1. MEDIUM: `MutationWriter` is crate-private in dioxus-native-dom 0.7.10 (public headless entry points: `DioxusDocument`, `DocumentConfig`); say so (embedding.md:82, crates.md:19).
2. MED-LOW: `handle_ui_event`/`poll` are `blitz_dom::Document` trait methods (need `use blitz_dom::Document;` and a `blitz-dom` dependency); add signatures (embedding.md:102-103).
3. MED-LOW: `document::eval` "is a no-op" understates: `NoOpDocument::eval` returns an `Eval` whose `send`/`poll_recv`/`poll_join` return `Err(EvalError::Unsupported)`; the Dialog survives only because it ignores the `send` result. Fix in blitz-native (SKILL.md:40, limitations.md:22, embedding.md:135) AND dioxus-design-system a11y.md:58.
4. LOW: "which dioxus-native inherits" inaccurate for wgpu/vello/wgpu_context (they arrive via `anyrender_vello 0.6.2`); drop "inherits" (crates.md:72-75).
5. LOW: "performs no network access" vs `cargo tree` registry access and possible `Cargo.lock` update: reword, or tighten to `cargo tree --locked --offline ...`.
6. LOW: delete SOURCES.md:55 "found through the GitHub issue search API" and the "Commands run" section (keep the "No native window was built or run" scope line under "Not verified").
7. LOW: SOURCES.md:62 overstates the escape page ("renders to the GPU using Vello"); use the page's words.
8. LOW: SOURCES.md:26 says 48 files; the directory has 89 area files, 48 tabulated: state the criterion.
9. LOW-MED: denominator-change cause asserted without evidence (tests x1.43, subtests x5.9, WPT revision also changed 42a9dfcc6 -> cd1d8aa5b); state numbers, record run 395's wpt_revision, drop the causal claim.
10. LOW: 90-day regeneration rule addressed to a role the skill lacks and a hard-coded snapshot filename; say "if after 2027-01-15, report the snapshot as stale and ask for current summary files"; move regeneration to a maintainer note; use a stable filename with the date inside.
11. LOW: roadmap absence stated as fact but tagged inferred; `<details>` also appears under "Archived Roadmap Items"; partial filter-effects/css-masking passes may be parse-only.
12. LOW: source labels: AccessKit comes from architecture.mmd, not the README; pipeline text is in `notes/architecture/06-RENDERERS.md`, not AGENTS.md.
13. LOW: record the dioxus-components revision read for the Dialog claim (main 9a75825).
14. LOW-MED: load cost; duplicate version table in SKILL.md vs crates.md; target ~2.5 KB.
15. INFO: add a negative for "dx serve --desktop opens a blank window".

## Spot-checks: 17 groups confirmed (version table digit for digit, workspace pins, WPT figures digit for digit, roadmap #119, README quotes, native features/API, `dx` Renderer enum, headless recipe with caveats 1-2). Not verified: blitz.is page contents, runtime behaviour, transitive licences.

## Collisions: none installed; boundaries with dioxus-07 and dioxus-design-system clean and mirrored.
