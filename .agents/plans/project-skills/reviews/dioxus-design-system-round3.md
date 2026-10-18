# Review: dioxus-design-system (round 3, focused)

Reviewer: skill-reviewer (fresh). Read-only; fetched dx v0.7.10 source and GitHub/crates.io metadata; ran no `dx` commands.

**Verdict: PASS (Section A clean). Grade: B.** B reflects no usage evidence, SKILL.md at 4,060 B, and one MEDIUM correctness gap (finding 1).

## Section A
- `allowed-tools`: Read, Grep, Glob, exact `Bash(dx --version)`; no Write/Edit/wildcard/`dx components`/cargo/network grant. The skill only proposes `dx components add` behind a pinned reviewed SHA, a mandatory diff and explicit user approval.
- No credential-adjacent reads, remote execution by the skill, injection text, home paths, usernames, first person or round narrative.

## Focus 1 re-verification (dx v0.7.10 `component.rs`)
(a) `--rev` is dropped without `--git` (424-430; `is_default()` false when `rev` is set, so Dioxus.toml `[components.registry]` is skipped): CONFIRMED. (b) with `--git` + `--rev` the registry is cloned and checked out at the rev; cache keyed by URL+rev and reused if present (356): CONFIRMED. (c) finishing steps print only for a newly created components module (201); the stylesheet line also needs `registry.is_default()` (208): CONFIRMED. (d) built-in dependencies resolve from the default registry (150-151); #5310 open; PARTLY: described incompletely (finding 1).
Command sweep: no old `--rev`-only form presented as working; no `dx components` command delegated to the agent.

## Round-2 findings: all resolved except 5 (Tailwind ordering: still stated as fact in evals.json:75) and 12 (load cost, acceptable).

## Remaining findings
1. MEDIUM (correctness): at 0.7.10 the #5310 gap is not a silent mix but an ABORT: `registry_root` is the pinned cache and is passed to every component's `add_component`; built-in dependencies' files are copied from the default-registry cache, and `copy_global_assets` (794-813) bails with `Cannot copy global asset '...' for component '...' because it is outside of the component registry '...'` (every dependency of color_picker, date_picker, item, sidebar declares `globalAssets: ["../../../assets/dx-components-theme.css"]`; checked popover, input, calendar, separator, sheet, skeleton, tooltip, button). Draft PR #5309 is open. Consequence: a first-time `add color_picker|date_picker|item|sidebar --git ... --rev ...` should fail after `cargo add` already ran, possibly after partial copies. Fix: say the add aborts with that error (source-derived and issue-reported, not run); make dependency-first the primary proposal for those four; add a triage row for the error with cleanup guidance (`Destination directory ... already exists` on re-run); add the abort to eval 9; keep `[unverified]` until run. (SKILL.md:26; workflow.md:129-135, 176; evals.json:111; SOURCES.md:28.)
2. MED-LOW: the mandatory post-add diff has no method and incomplete scope: say the user runs a clone/checkout of the SHA and the agent compares with Read/Grep; list expected differences (manifest `exclude` entries `variants`, `docs.md`, `component.json` are not copied; source dir `preview/src/components/<name>/`); include `assets/dx-components-theme.css` and the `Cargo.toml` changes (dependencies from default-registry manifests still get `cargo add`ed even with the dependency-first workaround).
3. LOW: workflow.md:98-99 "add --force and a re-add overwrite local edits": a re-add without `--force` fails with `Destination directory ... already exists`; only `--force` overwrites, `remove` deletes.
4. LOW: SKILL.md:25 states "`--rev` is ignored without `--git`" flatly while workflow.md:63-64 tags it `[unverified]`; scope with "(dx 0.7.10 source)" or unify; the `dx components list --rev deadbeef` probe needs "(user-run, throwaway crate)".
5. LOW: workflow.md:27 `update` row: with no flags `update` uses `config.components.registry.remote`, so a Dioxus.toml git+rev is reset to that rev.
6. LOW: evals.json:75 states the Tailwind ordering as fact (tagged `[unverified]` in theme.md).
7. LOW optional: eval 3 should assert the element-qualified selector / specificity point.
8. LOW: SKILL.md 4,060 B; SKILL.md:29-30 duplicates workflow.md:137-145; the inline add procedure is safety-critical and fine to keep.
9. LOW: the SKILL.md:27 parenthetical is cryptic (omits `--path` and the Dioxus.toml case; does not say nothing prints after the first add).

## Spot-checks: registry `main` still 9a75825; 45 component directories match the mapping; dependency lists of the four affected components as stated; `DioxusLabs/components` 301; crates.io facts; #5310 open, PR #5309 open draft; `.dx-button` specificities hold; `cargo add` argument construction matches.

## Collisions: none installed; sibling NOT clauses symmetric; native-dialog fact has a single owner (blitz-native).
