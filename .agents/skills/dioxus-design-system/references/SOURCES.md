# Sources for `dioxus-design-system`

Verified against dx 0.7.10 (57d6794). Registry facts are for `DioxusLabs/dioxus-components` `main`, commit
`9a75825` (2026-09-08), as retrieved on **2026-10-18**; CLI facts are for the `v0.7.10` source tag. Short
code samples in the skill are adapted from the sources below.

## Registry repository: `https://github.com/DioxusLabs/dioxus-components`

Licence: README states dual MIT or Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`); GitHub metadata reports
Apache-2.0. `https://github.com/DioxusLabs/components` returns HTTP 301 to the URL above.

| File / endpoint | Supports | Retrieved |
|---|---|---|
| `https://api.github.com/repos/DioxusLabs/dioxus-components` and `.../git/trees/main?recursive=1` | repository identity, layout, the list of 45 components, asset paths | 2026-10-18 |
| `README.md` | install flow (`dx components add button`), project structure, WAI-ARIA statement, licence text | 2026-10-18 |
| `component.json`, `preview/src/components/button/component.json` | registry index shape; Button manifest (git dependency, global asset) | 2026-10-18 |
| `preview/src/components/*/component.json` (all 45) | which components declare `componentDependencies` (`color_picker`, `date_picker`, `item`, `sidebar`) | 2026-10-18 |
| `Cargo.toml`, `primitives/Cargo.toml` | workspace `dioxus = "0.7.8"`; primitives crate is `0.0.1` in-repo | 2026-10-18 |
| `preview/src/components/button/component.rs`, `style.css` | CSS-module styling, `data-style` / `data-size`, `merge_attributes`, `:focus-visible` rule | 2026-10-18 |
| `preview/assets/dx-components-theme.css` | theme variables, dark and light switching | 2026-10-18 |
| `primitives/src/lib.rs`, `dialog.rs`, `alert_dialog.rs`, `tabs.rs`, `toast.rs`, `popover.rs`, `js/focus-trap.js` | exported modules, `use_controlled`, `merge_attributes`, roles, `aria_*`, keyboard handling, prop names, `DialogTitle` / `DialogDescription` id wiring (`dialog.rs` and `lib.rs` were re-checked as identical at `9a75825`) | 2026-10-18 |
| `complaints.md` | upstream's list of Dioxus gaps hit while building the library (context only) | 2026-10-18 |

## Upstream issue and pull request (facts only)

| Source | Supports | Retrieved |
|---|---|---|
| `https://github.com/DioxusLabs/dioxus/issues/5310` ("dx components add fails to handle a rev for the registry properly", open, created 2026-02-13) | `--rev` does not apply to built-in component dependencies | 2026-10-18 |
| `https://github.com/DioxusLabs/dioxus/pull/5309` ("fix(cli): use correct registry root for dependency asset copying", open draft) | a fix is proposed; its description states that asset copying for a dependency resolved from a different registry fails because one registry root is used for all components | 2026-10-18 |

## crates.io metadata (`https://crates.io/api/v1/crates/<name>`; facts only, no licence applies)

| Crate | Finding | Retrieved |
|---|---|---|
| `dioxus-primitives` | only version `0.0.0`, created 2025-04-13, no repository link | 2026-10-18 |
| `dioxus-components` | version `0.1.2`, repository `LeTri234/dioxus_component` | 2026-10-18 |

## Dioxus CLI (`https://github.com/DioxusLabs/dioxus`, tag `v0.7.10`), MIT OR Apache-2.0

| File | Supports | Retrieved |
|---|---|---|
| `packages/cli/src/cli/component.rs` | behaviour of `list` / `add` / `update` / `remove` / `clean`; default registry URL; `--rev` dropped without `--git` (lines 424-430); `update` with no flags (215-218); component dependency resolution (141-176); `cargo add` before any copy (185); per-component copy order, early return for existing dependency directories and `pub mod` append (575-625, 651-657); global-asset root check and its error text (789-813); finishing-steps conditions (200-211); cache reuse; `remove_dir_all` on a raw name (273) | 2026-10-18 |
| `packages/cli/src/config/component.rs` | `[components]` configuration keys | 2026-10-18 |
| `packages/component-manifest/src/lib.rs` | manifest types; `CargoDependency::add_command` (the `cargo add` arguments) | 2026-10-18 |

## Official documentation, no licence declared

| Page | Supports | Retrieved |
|---|---|---|
| `https://dioxuslabs.com/learn/0.7/essentials/ui/styling` | Tailwind detection claim (see `dioxus-07`) | 2026-10-18 |
| `https://dioxuslabs.com/learn/0.7/essentials/ui/assets` | `asset!` rules used in the theme link snippet | 2026-10-18 |

## Verification performed (dx 0.7.10 (57d6794), minimal crate)

`dx components --help` (and `add`, `list`, `update`); `dx components schema`; `dx config schema -o <file>`;
`dx components list --offline` outside a crate (fails: no `Cargo.toml`) and inside a crate (fails:
`Cannot download component registry 'https://github.com/dioxuslabs/components' while offline`). Not run:
online `list` and `add`; the `--rev`, finishing-step and #5310 abort behaviours above come from the source and
the issue report, not from a run.
