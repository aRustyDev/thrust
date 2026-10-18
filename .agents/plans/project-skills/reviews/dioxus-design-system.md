# Review: dioxus-design-system (round 1)

Reviewer: skill-reviewer. Draft read in full (SKILL.md, 4 references + SOURCES.md, 2 eval files). Nothing executed beyond `dx --version`/`--help`.

**Verdict: REJECT. Grade: UNGRADEABLE (Section A failed).**

## Section A (blocking)
- A2.1: `Bash(dx components list:*)`, `Bash(dx components add:*)`, `Bash(dx components remove:*)` (SKILL.md:11-13). The `--rev` pin exists only as prose; `:*` permits `--git <any URL>`, `--path`, `--all`, `--force`, `--module-path`. The registry can be chosen silently via `Dioxus.toml [components.registry]`. A manifest's `cargoDependencies` drive `cargo add`; a pre-approved `cargo check` then runs build scripts of what was added: a pre-approved chain from fetched manifest to executed code. `remove` does `remove_dir_all` on a raw CLI path; `add --force` on a registry-supplied name (CLI source v0.7.10).
- A2.1/A2.4: `Bash(cargo check:*)` (SKILL.md:14): `--config build.rustc-wrapper=...` / `--manifest-path` trampoline.
- Considered, not failed: the pinned git dependency on the official registry (reviewable, declared in Cargo.toml).
- Passed: key/format, provenance, trigger specificity, credentials, no remote-exec constructs, no shipped scripts.
- No safe `:*` form exists for `dx components add`; a pre-approval would be the owner's decision. Minimal fix: delete the three `dx components` grants (keep `dx --version`), make `cargo check` exact or delete; run `dx components add` with the full command including a 40-hex `--rev`, shown for approval.

## Section B/C
1. Pin claim incomplete: built-in component dependencies resolve from the default registry regardless of `--rev` (upstream issue DioxusLabs/dioxus#5310; affects color_picker, date_picker, item, sidebar); require a full 40-hex SHA; add the dependency-first workaround (untested).
2. "never use crates.io names" overbroad (registry manifests legitimately use dioxus-icons, time, palette); retitle to dioxus-primitives/dioxus-components, dated.
3. theme.md:60-63 override example contradicts the hashed-class note.
4. No procedure for choosing a SHA.
5. Preflight: give the reason (Deno shim fetches/runs npm packages), remediation, and a check that `[components.registry]` is unset or official.
6. "Do the printed steps" unsafe: finishing steps print only when the components module is newly created.
7. "exact" in mapping.md means name-match only: rename to same-name/nearest/none; caveat in SKILL.md; reword eval 4.
8. Collisions: dioxus-07 hands all "theme or CSS-token work" here; narrow; Tailwind detail duplicated between dioxus-07 cheatsheet.md and theme.md; add negative trigger for generic token audits.
9. SOURCES.md/workflow.md/mapping.md process narrative and tooling leaks.
10. Evals: no assertion of the Deno-shim preflight; none for `--git`/registry override or dependency-component pinning; eval 7 grades an unverified rumour; add near-miss negatives.
11. "dx components broken in 0.8 alpha" has no source: replace with a plain precaution.
12. `css_module!` is wrong notation (attribute macro `#[css_module("...")]`).
13. Point-in-time data: component count; "last push" is pushed_at of any branch.
14. Load cost 4,095 B vs benchmark; move theme and 0.8 paragraphs to references (~3 KB).
15. Unscoped Edit/Write; path-scope.
16. No worked output example.

## Spot-checks: 15 groups; confirmed except as noted above (crates.io primitives 0.0.0 placeholder; unrelated `dioxus-components` crate; registry CSS modules not Tailwind; theme variables; CLI subcommands; Deno shim detected by preflight; Dialog/Tabs source behaviour; `merge_attributes`/`use_controlled`).

## Collisions: none with installed Dioxus skills (none exist); weak overlap with generic `design:design-system`.
