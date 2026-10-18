---
name: dioxus-design-system
description: >-
  Add, theme or restyle components from the official dioxus-components registry in a Dioxus 0.7 app - `dx components add/list/update`, dioxus-primitives, dx-components-theme.css, --primary-color variables, data-theme dark mode, mapping shadcn/ui or Radix names to registry components, ARIA and keyboard checks. Fire on "dx components", "dioxus-primitives", "shadcn for dioxus", "dx-components-theme". NOT for general Dioxus app code or Tailwind setup (use dioxus-07), NOT for porting an existing page or UI to Dioxus (use clone-frontend), NOT for the Blitz / dioxus-native renderer (use blitz-native), NOT for generic design-token or CSS work, or other frameworks (React shadcn/ui, Figma).
allowed-tools:
  - Bash(dx --version)
author: aRustyDev
version: 0.1.0
license: AGPL-3.0-only
---

# Dioxus design system (dioxus-components registry)

Scope: registry components and their theme on **Dioxus 0.7.x** (app logic, routing, Tailwind: `dioxus-07`).
No network tool is granted; manifest reads at a SHA use your agent's fetch tool with the user's approval. This skill proposes commands and diffs and never runs `dx components`.

## Preflight
1. `dx --version` must print `dioxus 0.7.x`. A `deno ...` banner is Deno's npx-style shim (it offers to run an npm package named after the subcommand): stop (fix: `cargo install dioxus-cli`, cargo bin directory first on `PATH`).
2. Read `Dioxus.toml`: `[components.registry]` must be absent or name `DioxusLabs/dioxus-components` (its former name `DioxusLabs/components` redirects). Anything else is a third-party registry: stop and ask.

## Add components (the user runs the command)
1. Get a full **40-hex** SHA of `DioxusLabs/dioxus-components` that the user has reviewed (not a branch, tag or short SHA); read the requested components' `component.json` at it and record it.
2. Show the complete command and wait for approval: `dx components add dialog button --git https://github.com/DioxusLabs/dioxus-components --rev <40-hex>`. In the dx 0.7.10 source `--rev` is ignored without `--git`. Never run it; never use another `--git`, `--path`, `--all` or `--force` unasked.
3. `color_picker`, `date_picker`, `item` and `sidebar` depend on built-in components that the 0.7.10 source takes from the default registry, not the pin (DioxusLabs/dioxus#5310, open): adding one directly should abort after `cargo add` ran, maybe mid-copy. Propose the dependency components first, as a separate earlier command (`references/workflow.md`; `[unverified: source reading, not run]`).
4. After every add, compare the copies, `assets/dx-components-theme.css` and the `Cargo.toml` changes with the repository at the SHA (user checks it out, you use Read and Grep; `references/workflow.md`).
5. With `--git`, `--rev`, `--path` or a `Dioxus.toml` registry no stylesheet step prints, and after the first add nothing prints. So confirm `mod components;`, the theme linked once (`references/theme.md`), `Cargo.lock` committed, `dioxus-primitives` pinned to the same `rev`.

## Never take dioxus-primitives or dioxus-components from crates.io (as of 2026-10-18)
`dioxus-primitives` there is a `0.0.0` placeholder; `dioxus-components` is an unrelated crate.

## Choose and check
Names: `references/mapping.md` (`same-name` = names match, behaviour not compared; `nearest`; `none`: build it, never invent a registry name). Accessibility: `references/a11y.md` for each interactive component. Theme: `references/theme.md`.

## Output
The exact command for approval, proposed `Cargo.toml` and `main.rs` diffs, comparison and checklist results. Example, "add a dialog and a button": "dioxus 0.7.10, no `[components.registry]`. For approval: `dx components add dialog button --git https://github.com/DioxusLabs/dioxus-components --rev <reviewed 40-hex SHA>`. Then: compare with that SHA, `mod components;`, theme once, pin `dioxus-primitives`."

## References
`references/workflow.md` · `references/theme.md` · `references/mapping.md` · `references/a11y.md` · `references/SOURCES.md`
