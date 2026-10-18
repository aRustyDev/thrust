# Review: dioxus-design-system (round 2)

Reviewer: skill-reviewer (fresh). Read SKILL.md, references, evals and the round-1 review. Ran only `dx --version`/`--help`.

**Verdict: PASS (Section A). Grade: B.** Held below A by one High correctness finding; recommendation: do not adopt until Finding 1 is fixed.

## Section A
- `allowed-tools` = Read, Grep, Glob, exact `Bash(dx --version)`. No wildcard, no `dx components`, cargo, Edit or Write grant. Safe even where `dx` is the Deno shim. The text categorically says the skill never runs `dx components` (SKILL.md:17, 25; workflow.md:37-38, 100).
- No leaks (home paths, usernames, round-N narrative, first person).

## Round-1 findings 1-16: resolved except 1/4 (partly, see F1), 14 (load cost 3,705 B), 3 (form fixed, specificity tie remains).

## Findings
1. HIGH (correctness; source-derived, not run): `--rev` without `--git` is silently ignored. dx 0.7.10 `component.rs:424-430`: with no `--git` the default registry is cloned at HEAD and `rev` dropped; `is_default()` is false whenever a rev is set, so `Dioxus.toml` is skipped. The skill's pin command `dx components add <names> --rev <40-hex>` therefore does not pin; "reviewed == installed" is false (the round-1 suggested fix shared this flaw). Cheap discriminator: `dx components list --rev deadbeef` in a throwaway crate (a real lookup fails with `Failed to find revision`). Fix: every command becomes `dx components add <names> --git https://github.com/DioxusLabs/dioxus-components --rev <40-hex>`; SKILL.md:25 "never add a --git other than the official URL" and state that `--rev` is ignored without `--git`; make "diff the copied files against the repo at the SHA" mandatory after every add; fix evals 1 and 9; update the `update` row at workflow.md:27.
2. MEDIUM: native-renderer caveat too narrow: dialog/alert_dialog/popover use `document::eval` for escape and outside-dismiss and animation, which are no-ops under the native document (`EvalError::Unsupported`), not only the focus trap (a11y.md:16-17, 57-61; evals.json:64). Extend; keep one owner for the fact (blitz-native repeats it).
3. MEDIUM-LOW: override selector `.settings-panel [data-style="primary"]` ties (0,2,0) with `.dx-button[data-style="primary"]`; use `.settings-panel button[data-style="primary"]` and note hover/focus need their own overrides.
4. LOW-MED: theme.md:42-45 "cannot be wrapped in calc()" contradicted by upstream `color-mix(...)`; delete the clause.
5. LOW-MED: Tailwind ordering advice (theme.md:72-73, evals.json:73-76) is unsourced and v4 layer-dependent; tag `[unverified]` or drop.
6. LOW: finishing-steps explanation omits `registry.is_default()` (component.rs:208): any `--git/--rev/--path` suppresses the stylesheet line.
7. LOW: hand-written dependency line drops `default-features = false` (cargo add uses `--no-default-features`); say keep whatever keys dx wrote, fix only the URL and add `rev`.
8. LOW: "registry HEAD at time of add" not exact; add reuses an existing cache until `dx components update`.
9. LOW: two stray `dx components list` suggestions (mapping.md:6, workflow.md:150) need "(user-run)"; `remove <name>` also calls `remove_dir_all` on a raw path.
10. LOW: preflight flags the former `DioxusLabs/components` URL (301) as third-party (safe false stop).
11. LOW: "lookalikes" overstates the `dioxus-primitives` crates.io placeholder (owned by a project author); retitle "crates.io name collisions".
12. LOW: load cost ~2.5x benchmark; optional trim.
13. LOW: "performs no network access" vs step 1 reading manifests via the agent's fetch tool: reword.
14. LOW: DialogTitle/DialogDescription wire ids from context; do not hand-wire (a11y.md:28-29; evals.json:63).
15. INFO: SOURCES.md "Commands run" is acceptable verification evidence; consider renaming "Verification performed".

## Spot-checks (~18): #5310 open; exactly four affected components (all 45 manifests re-read); `document::eval` no-op on native confirmed; `#[css_module]` attribute macro; mapping rows match primitives; crates.io facts; theme variables; Button/Dialog/Tabs behaviours; CLI behaviours; Deno shim.

## Collisions: none installed; boundaries symmetrical; the native-dialog fact is duplicated with blitz-native (F2).
