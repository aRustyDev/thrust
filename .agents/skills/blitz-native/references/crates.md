# Blitz and dioxus-native crate map

Sources (retrieved 2026-10-18): Blitz `README.md` and `docs/architecture.mmd` (`main`), crates.io metadata
and dependency lists, and the `DioxusLabs/dioxus` `v0.7.10` manifests and sources (`packages/native`,
`packages/native-dom`, root `Cargo.toml`). Blitz `main` is the 0.3 line; 0.7.x apps get the 0.2 line.
Always confirm with `cargo tree -p dioxus-native` and `cargo tree -i blitz-dom`.

## Crates

| Crate | Job | Uses (README; AccessKit for blitz-dom from `architecture.mmd`) | Stable on crates.io | Newest |
|---|---|---|---|---|
| `blitz-traits` | minimal shared types and traits so crates interoperate without depending on each other | none | 0.2.0 | 0.3.0-beta.4 |
| `blitz-dom` | core DOM: style resolution, layout, event handling, accessibility tree. No parsing, rendering or windowing | Stylo, Taffy, Parley, AccessKit | 0.2.4 | 0.3.0-beta.4 |
| `blitz-html` | adds HTML parsing to blitz-dom | html5ever, xml5ever (XHTML) | 0.2.0 | 0.3.0-beta.4 |
| `blitz-net` | fetch resources over http, from the filesystem, or from data URIs | reqwest | 0.2.1 | 0.3.0-beta.4 |
| `blitz-paint` | translate a blitz-dom tree into AnyRender draw commands | anyrender | 0.2.1 | 0.3.0-beta.4 |
| `blitz-shell` | window shell: winit event loop, AccessKit, system menus | winit, accesskit, muda | 0.2.3 | 0.3.0-beta.4 |
| `blitz` | high-level HTML / markdown viewer: render an HTML string; no interactivity | blitz-dom, blitz-html, blitz-shell, a renderer | 0.2.1 | 0.3.0-beta.4 |
| `dioxus-native-dom` | "Core headless native renderer for Dioxus based on blitz". Public API in 0.7.10: `DioxusDocument` and `DocumentConfig`. Internally a `MutationWriter` implements `WriteMutations` onto a blitz-dom `BaseDocument`, but that type is crate-private (`mod mutation_writer;` is not re-exported), so it cannot be used from outside | blitz-dom, blitz-traits, dioxus-core, dioxus-html | 0.7.10 | 0.8.0-alpha.3 |
| `dioxus-native` | "Native renderer for Dioxus based on blitz": the full renderer with window, painting, networking, hot reload | everything above plus anyrender_vello | 0.7.10 | 0.8.0-alpha.3 |

Also in the Blitz repository (not needed to use it): `stylo_taffy` (Stylo-to-Taffy glue; additionally
MPL-2.0 licensed), `accesskit_xplat`, `blitz-test-harness`, `blitz-vibey-script`, `debug_timer`, plus
`apps/browser` (the Browser UI), `apps/readme` (markdown viewer), `examples/`, `wpt/` (WPT runner). The
AnyRender abstraction lives in its own repository (`DioxusLabs/anyrender`).

## How they fit (`docs/architecture.mmd` on `main`)

- `dioxus-native` depends on `blitz-paint`, `blitz-shell`, `dioxus-native-dom`, `blitz-net`, `blitz-html`.
- `blitz-dom` depends on `blitz-traits`, Stylo, Taffy, Parley, AccessKit.
- `blitz-paint` depends on AnyRender and `blitz-dom`; AnyRender backends reach `wgpu`.
- `blitz-shell` and `blitz-html` depend on `blitz-dom`; `dioxus-native-dom` depends on `blitz-dom`.

Pipeline for a Dioxus app (Dioxus `notes/architecture/06-RENDERERS.md`, 0.7.10): VirtualDom, `DioxusNativeDOM`, Blitz DOM
with CSS, layout, Vello, GPU. A winit `RedrawRequested` event calls `render_immediate`, applies mutations to the
Blitz DOM, computes layout and paints the frame. In the webview renderer the VirtualDom instead streams
binary edits to a browser engine.

## What each third-party piece is for

| Piece | Role | Upstream |
|---|---|---|
| Stylo | CSS parsing, selector matching, cascade, computed values (the engine Servo and Firefox use) | `servo/stylo` |
| Taffy | box-level layout: block, flexbox, grid | `DioxusLabs/taffy` |
| Parley | text layout, shaping, line breaking, font selection | `linebender/parley` |
| AnyRender | small 2D-drawing abstraction between blitz-paint and a backend | `DioxusLabs/anyrender` |
| Vello | GPU 2D vector renderer behind AnyRender (`anyrender_vello`), on wgpu | `linebender/vello` |
| html5ever / xml5ever | HTML and XHTML parsing | `servo/html5ever` |
| winit | windowing and input | `rust-windowing/winit` |
| AccessKit | cross-platform accessibility tree | `AccessKit/accesskit` |
| muda | native menus | `tauri-apps/muda` |

AnyRender backends: on 0.7.10 `dioxus-native` uses `anyrender_vello` (GPU) and, only for the iOS simulator
target (`cfg(all(target_os = "ios", target_abi = "sim"))`), `anyrender_vello_cpu`. The architecture diagram
on `main` lists `vello_cpu`, `vello`, `vello_hybrid` and `skia` backends, so backend choice is a
0.3-line feature `[unverified for 0.7.x]`.

## What `dioxus-native` requires (crates.io dependency lists)

The `dioxus-native` crate declares these requirements; Cargo resolves them against what is published.

| `dioxus-native` | blitz-* requirement | anyrender | notes |
|---|---|---|---|
| 0.7.10 | blitz-dom `^0.2.4`; blitz-net, -html, -paint, -traits, -shell `^0.2` | `^0.6.2`; `anyrender_vello ^0.6` | the 0.7.x line |
| 0.8.0-alpha.0 (2026-05-19) | `=0.3.0-alpha.4` | `^0.10`; `anyrender_vello ^0.10.1` | winit `=0.31.0-beta.2` |
| 0.8.0-alpha.1 (2026-07-31) | `=0.3.0-beta.1` | `^0.11.0`; `anyrender_vello ^0.12.0` | winit `=0.31.0-beta.2` |
| 0.8.0-alpha.2 (2026-10-14) | `=0.3.0-beta.3` | `^0.14.0`; `anyrender_vello ^0.15.0` | winit `=0.31.0-beta.3` |
| 0.8.0-alpha.3 (2026-10-16) | `=0.3.0-beta.4` | `^0.14.0`; `anyrender_vello ^0.15.0` | winit `=0.31.0-beta.3` |

`blitz-dom 0.3.0-beta.1` was published 2026-07-10. No 0.7.x release requires a 0.3 pre-release.

The `dioxus` repository's own workspace dependency table at `v0.7.10` lists `wgpu 26.0.1`, `vello 0.6`,
`winit 0.30.11`, `anyrender_vello_cpu 0.8`, `wgpu_context 0.1`, plus `wry 0.53.5` and `tao 0.34.0` for the
webview renderer. These are the workspace's pins, not `dioxus-native` requirements: the 0.7.10
`packages/native/Cargo.toml` lists none of `wgpu`, `vello` or `wgpu_context`, so they reach the build through
`anyrender_vello` `[inferred]`. (The 0.8 alpha manifests do list `wgpu_context` directly.) On `main` the
workspace table moves to `wgpu 30.0.1`, `vello 0.11` and `winit =0.31.0-beta.3`.

## `dioxus-native` Cargo features (0.7.10)

Default: `accessibility`, `hot-reload`, `net`, `html`, `svg`, `system-fonts`, `clipboard`, `file_dialog`.
Opt-in: `incremental` (blitz-dom incremental), `autofocus`, `prelude` (re-exports hooks, signals, stores,
macros and manganis so the `dioxus` crate can be dropped), `tracing`. The crate docs also list a `menu`
feature that is **not** in the 0.7.10 `[features]` table: doc drift; do not rely on it `[unverified]`.

## Licences (facts only)

Blitz: Apache-2.0 and MIT dual (README); `stylo_taffy` additionally MPL-2.0. `dioxus-native` and
`dioxus-native-dom`: MIT OR Apache-2.0. Transitive licences (Stylo, Taffy, Parley, Vello, html5ever, winit,
AccessKit and the rest of the tree) are not audited here `[unverified]`: run a licence tool such as
`cargo deny` before shipping.
