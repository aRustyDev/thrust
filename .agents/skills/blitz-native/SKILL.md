---
name: blitz-native
description: >-
  Work with Blitz (the DioxusLabs HTML/CSS rendering engine) and dioxus-native, the GPU-rendered no-webview Dioxus renderer - which blitz crate does what, what Stylo, Taffy, Parley and Vello do, whether a CSS feature works in the dioxus native renderer (WPT results), embedding, `--renderer native`, a blank window with dioxus-native. Fire on "blitz-dom", "dioxus-native", "dioxus native renderer", "dx serve --renderer native". NOT for ordinary Dioxus app code or `dx serve --desktop` webview problems (use dioxus-07), dx-components theming (use dioxus-design-system), or unrelated "blitz" (Blitz.js, chess blitz, React Native's renderer).
allowed-tools:
  - Bash(cargo tree --locked --offline -p dioxus-native)
  - Bash(cargo tree --locked --offline -i blitz-dom)
  - Bash(dx --version)
author: aRustyDev
version: 0.1.0
license: AGPL-3.0-only
---

# Blitz and dioxus-native

Blitz calls itself **beta**. Pinned to Dioxus **0.7.x** (verified on 0.7.10).
This skill makes no requests of its own; use whatever docs tool your agent has, or ask the user to download a public data file named here. `cargo tree --locked --offline` reads only the local lockfile and cache; if it fails, ask the user to run it.
If `dx` is involved, `dx --version` must print `dioxus 0.7.x`; a `deno ...` banner is Deno's npx-style shim (it offers to run an npm package named after the subcommand): `cargo install dioxus-cli`, cargo bin directory first on `PATH`.

## Versions
`dioxus-native` 0.7.x requires blitz-dom `^0.2.4`. The 0.3 pre-releases belong to the 0.8 alpha series only (`0.3.0-beta.1` is required by 0.8.0-alpha.1). Table: `references/crates.md`. Blitz `main`, its roadmap and its WPT results describe the 0.3 line: never promise a `main` feature to a 0.7 app.

## Is CSS feature X supported?
Do not guess. Evidence ladder: a repro on the pinned version; the 0.7-era table in `references/css-support.md`; WPT data (`DioxusLabs/blitz-wpt-results`); the roadmap. Tag each claim `repro`, `WPT`, `roadmap`, `inferred` or `[unverified]`. WPT measures conformance, not app usability. After 2027-01-15, report `references/wpt-snapshot.md` as stale and ask for current summary files.

## Facts to state
- On dioxus-native 0.7.10 `document::eval` never runs the script: `send`, `recv` and `join` return `EvalError::Unsupported` (no JavaScript engine). The dioxus-components Dialog survives only because it ignores those results; its focus trap, escape-to-close, outside-dismiss and exit animation all depend on eval. Detail: `references/limitations.md`.
- Embedding: `references/embedding.md`. Stack roles (Stylo, Taffy, Parley, Vello): `references/crates.md`.

## Output
Versions the answer applies to, each support claim tagged by evidence, a check for anything uncertain. Example for "does dioxus-native 0.7.10 support position: sticky and backdrop-filter?": "`cargo tree --locked --offline -i blitz-dom` shows 0.2.4. `position: sticky` is open on the roadmap for 1.0 `[roadmap]`: do not rely on it. `backdrop-filter` is under the 0.3 milestone `[roadmap]`, so its absence from blitz 0.2 is `[inferred]`. Confirm both with a repro."

## References
`references/crates.md` · `references/css-support.md` · `references/wpt-snapshot.md` · `references/embedding.md` · `references/limitations.md` · `references/SOURCES.md`
