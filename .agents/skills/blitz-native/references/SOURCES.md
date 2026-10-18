# Sources for `blitz-native`

Verified against dx 0.7.10 (57d6794) and the `DioxusLabs/dioxus` `v0.7.10` tag. Blitz facts are for
`DioxusLabs/blitz` `main` (the 0.3 line) unless a row says otherwise. All pages retrieved **2026-10-18**.
Short quotations are attributed to the README or issue they come from.

## Blitz repository: `https://github.com/DioxusLabs/blitz`

Licence: Apache-2.0 and MIT dual (README); `stylo_taffy` additionally MPL-2.0. GitHub metadata reports
Apache-2.0.

| File / endpoint | Supports |
|---|---|
| `README.md` | status ("beta"), goals and non-goals, crate list and roles (Stylo, Taffy, Parley for blitz-dom; winit, AccessKit, muda for blitz-shell), git-version instructions, licences |
| `docs/architecture.mmd` | dependency graph in `crates.md`, including blitz-dom's AccessKit dependency and the AnyRender backends |
| `HOWTO_WASM.md` | browser build notes in `embedding.md` |
| `https://api.github.com/repos/DioxusLabs/blitz/issues/119` ("Roadmap", open, updated 2026-10-14) | milestones, open items, backlog, and the "Archived Roadmap Items" section, in `css-support.md` and `limitations.md` |
| repository trees: top level, `packages/`, `apps/`, `wpt/`, `docs/` | workspace layout |

## WPT results: `https://github.com/DioxusLabs/blitz-wpt-results` (no licence declared)

| File | Supports |
|---|---|
| `summary/README.md` | data format; pass-rate definition `scores[3] / scores[2]` |
| `summary/runs.json` (1,196 runs; run 395 is 2025-10-22, Blitz `5f932bfc68`, WPT `42a9dfcc6`; run 1195 is 2026-10-17, Blitz `fb8d6c15e9`, WPT `cd1d8aa5b`) | which Blitz commit and WPT revision each score index describes |
| `summary/areas/css.json` and 48 of the 89 files under `summary/areas/css/` (hand-picked for layout, box model, text, colour, effects, selectors and animation) | the run-395 table in `css-support.md` and `wpt-snapshot.md` |

## Dioxus source, tag `v0.7.10` (`https://github.com/DioxusLabs/dioxus`), MIT OR Apache-2.0

| File | Supports |
|---|---|
| `packages/native/Cargo.toml`, `src/lib.rs` | features, launch API, crate description |
| `packages/native/src/contexts.rs`, `packages/document/src/document.rs`, `eval.rs` | `DioxusNativeDocument::eval` delegates to `NoOpDocument`, whose evaluator returns `EvalError::Unsupported` |
| `packages/native-dom/Cargo.toml`, `src/lib.rs`, `src/dioxus_document.rs`, `src/mutation_writer.rs` | public API (`DioxusDocument`, `DocumentConfig`), crate-private `MutationWriter`, the `blitz_dom::Document` impl (`poll`, `handle_ui_event`), the doc example, the event kinds forwarded to Dioxus |
| `Cargo.toml` at `v0.7.10` and `main` | workspace dependency table; `main` is `0.8.0-alpha.3` |
| `packages/dioxus/Cargo.toml`, `src/lib.rs` | `native` feature and `dioxus::native` re-export |
| `notes/architecture/06-RENDERERS.md` | the native pipeline description in `crates.md` |

## dioxus-components: `https://github.com/DioxusLabs/dioxus-components` (README: MIT or Apache-2.0)

Read at `main`, commit `9a75825` (2026-09-08).

| File | Supports |
|---|---|
| `primitives/src/lib.rs` (`use_global_escape_listener`, `use_outside_dismiss`, `use_animated_open`), `primitives/src/dialog.rs`, `primitives/src/js/focus-trap.js` | the Dialog focus trap, escape-to-close, outside-dismiss and exit animation are built on `document::eval` and ignore its results |

## crates.io (`https://crates.io/api/v1/crates/<name>`; facts only, no licence applies)

Metadata for `blitz`, `blitz-dom`, `blitz-html`, `blitz-net`, `blitz-paint`, `blitz-shell`, `blitz-traits`,
`dioxus-native`, `dioxus-native-dom`, `anyrender_vello` (descriptions, stable and newest versions, publish
dates), and the dependency lists of `dioxus-native` `0.7.10`, `0.8.0-alpha.0`, `0.8.0-alpha.1`,
`0.8.0-alpha.2`, `0.8.0-alpha.3` (the "requires" table in `crates.md`).

## GitHub issues (facts only)

`DioxusLabs/dioxus` #5282, #4951, #4901 (open when read), #5617, #5620, #5821 (closed), listed in
`limitations.md` as examples.

## Official documentation, no licence declared

| Page | Supports |
|---|---|
| `https://dioxuslabs.com/learn/0.7/guides/platforms/desktop` | webview is the current desktop renderer; Blitz is the planned direction |
| `https://dioxuslabs.com/learn/0.7/essentials/ui/escape` | calls Dioxus Native "nascent", says it paints the widget tree to the screen using Vello-based backends, and documents `eval` for the web and webview renderers only |
| `https://dioxuslabs.com/learn/0.7/essentials/fullstack/native` | native fullstack apps render client side; no Blitz content |

## Not verified

No native window was built or run. The contents of the `blitz.is` status pages and the transitive licence
tree were not checked.
