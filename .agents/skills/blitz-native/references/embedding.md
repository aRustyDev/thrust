# Using and embedding the native renderer (Dioxus 0.7.10 / Blitz 0.2)

Sources (retrieved 2026-10-18): `dioxus-native` `src/lib.rs`, `Cargo.toml`, `dioxus-native-dom`
`src/dioxus_document.rs` and `mutation_writer.rs`, and `dioxus` `Cargo.toml` / `src/lib.rs` at tag `v0.7.10`;
Blitz `README.md` and `HOWTO_WASM.md` (`main`); `dx serve --help` from CLI 0.7.10. No native window was
built or run (it needs a GPU stack and a full compile): the code blocks are readings of source and doc
comments, tagged where they are an assumption.

## Choose the entry point

| Goal | Use |
|---|---|
| An app that targets several renderers | the `dioxus` crate with the `native` feature; app code stays `dioxus::launch(App)` (see `dioxus-07`) |
| A native-only app, minimal dependencies | `dioxus-native` directly with its `prelude` feature |
| Track unreleased Blitz fixes | git version of `dioxus-native` (below) |
| Run a VirtualDom with no window (tests, custom shell) | `dioxus-native-dom` |
| Render a static HTML or markdown string | the `blitz` crate (no interactivity) |
| Draw your own wgpu content inside the UI | `use_wgpu` / `CustomPaintSource` |

## 1. Through the `dioxus` crate

```toml
[dependencies]
dioxus = { version = "0.7", features = ["native"] }
```

`native = ["dep:dioxus-native", "dioxus-config-macro/native"]` in `dioxus` 0.7.10, re-exported as
`dioxus::native`. Upstream leaves a TODO in that feature table: "decompose the desktop crate such that
`webview` is the default and native is opt-in". CLI: `--renderer native` is a listed value of `dx serve` /
`dx build` / `dx check` (`webview`, `native`, `server`, `liveview`, `web`), "usually inferred from the
platform". `[unverified: the exact flag combination for a native desktop build, e.g. --desktop with
--renderer native; not built here]`.

## 2. `dioxus-native` directly

```toml
[dependencies]
dioxus-native = { version = "0.7", features = ["prelude"] }
```

```rust
use dioxus_native::prelude::*;        // hooks, signals, stores, macros, manganis (feature "prelude")

fn main() {
    dioxus_native::launch(App);
}

#[component]
fn App() -> Element { rsx! { h1 { "Hello, native" } } }
```

Public API at 0.7.10 (from `lib.rs`):
- `launch(app: fn() -> Element)`
- `launch_cfg(app, contexts: Vec<Box<dyn Fn() -> Box<dyn Any> + Send + Sync>>, cfg: Vec<Box<dyn Any>>)`
- `launch_cfg_with_props(app, props, contexts, configs)`
- `cfg` entries are downcast by type: `Features`, `Limits` (wgpu device settings), `WindowAttributes`
  (winit window), `Config` (carries `window_attributes`). Re-exports include `LogicalSize`, `PhysicalSize`.
- `DioxusNativeApplication`, `DioxusNativeEvent`, `DioxusNativeWindowRenderer` for custom shells.

What `launch` does: builds the event loop; with the `net` feature starts a multi-thread tokio runtime;
in debug builds with `hot-reload` (not on Android or iOS) connects to the `dx` dev server; creates the
`VirtualDom`, inserts root contexts, builds a `DioxusDocument` with a net provider, an HTML parser
provider (feature `html`) and a navigation provider; creates the window renderer (Vello on wgpu; a CPU
renderer on the iOS simulator); runs the winit loop.

Features (defaults): `accessibility`, `hot-reload`, `net`, `html`, `svg`, `system-fonts`, `clipboard`,
`file_dialog`. Opt-in: `incremental`, `autofocus`, `prelude`, `tracing`. Disable defaults you do not
need to shrink the dependency tree `[unverified: effect on binary size not measured]`.

## 3. Git version of `dioxus-native` (from the Blitz README)

Development of `dioxus-native` happens in the Blitz repository. To use it, remove the `dioxus` dependency,
depend on `dioxus-native = { git = "https://github.com/DioxusLabs/blitz", rev = "<sha>", features = ["prelude"] }`,
and change `use dioxus::prelude::*` to `use dioxus_native::prelude::*`. Per the README the git version still
depends on stable Dioxus `0.7.x` from crates.io, so `dioxus-sdk`, `dioxus-components` and similar libraries
keep working. **Always pin a `rev`.** The README's example sha is only an example; choose your own.
Note this pulls the Blitz `main` line (0.3 beta), not 0.2.

## 4. Headless: `dioxus-native-dom`

"Core headless native renderer for Dioxus based on blitz." It owns the VirtualDom-to-blitz-dom bridge with no
window, painting or networking. Its public API in 0.7.10 is `DioxusDocument` (a `VirtualDom` paired with a
blitz-dom `BaseDocument`; it derefs to `BaseDocument`) and `DocumentConfig`. The `MutationWriter` that
implements `WriteMutations` is **crate-private**, so a custom renderer cannot reuse it from outside. The
crate's own doc example (0.7.10):

```rust
use blitz_traits::shell::{Viewport, ColorScheme};
use dioxus_native_dom::{DioxusDocument, DocumentConfig};
use dioxus::prelude::*;

fn app() -> Element { rsx! { div { "Hello, world!" } } }

fn main() {
    let vdom = VirtualDom::new(app);
    let mut doc = DioxusDocument::new(vdom, DocumentConfig {
        viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Light)),
        ..Default::default()
    });
    doc.initial_build();
}
```

Events and flushing are not inherent methods: `handle_ui_event` and `poll` are methods of the
**`blitz_dom::Document` trait**, which `DioxusDocument` implements, so the caller needs a `blitz-dom`
dependency (the version `dioxus-native-dom` resolves to) and `use blitz_dom::Document;`:

```rust
use blitz_dom::Document;
// fn handle_ui_event(&mut self, event: blitz_traits::events::UiEvent)
// fn poll(&mut self, cx: Option<std::task::Context>) -> bool   // true when the VirtualDom had work and
//                                                              // its mutations were applied (render_immediate)
```

The document wraps your app in `<html><head></head><body><main id="main"></main></body></html>`
(custom `index.html` templates are a TODO in the source) and installs a default user-agent stylesheet. Use
it to drive a Blitz DOM from your own event loop, to inspect layout in tests, or to embed Dioxus UI in
another shell; painting is separate (`blitz-paint` into an AnyRender backend) and its wiring is
`[unverified]` here. Defaults: `accessibility`, `svg`, `system-fonts`. For the renderer-author view of
mutations, see the `dioxus-07` skill's `references/renderer.md`.

## 5. Static HTML: the `blitz` crate

"High-level APIs for rendering HTML with Blitz": an HTML/markdown viewer built from `blitz-dom`,
`blitz-html`, `blitz-shell` and a renderer; it currently lacks interactivity. The Blitz repo examples
(`cargo run -rp browser`, `-rp rdme ./README.md`, `-rp todomvc`, `-rp wgpu_texture`) are the runnable
references; build them from a Blitz checkout (`main`, 0.3 line).

## 6. Custom GPU drawing

`dioxus-native` re-exports `CustomPaintCtx`, `CustomPaintSource`, `DeviceHandle`, `TextureHandle` from
`anyrender_vello` and (except on the iOS simulator) the `use_wgpu` hook. The roadmap lists "WGPU texture
rendering (`<canvas>` support)" under 0.1. The Blitz repo's `wgpu_texture` example shows the pattern
`[unverified: API not exercised here]`.

## 7. Running in a browser

`HOWTO_WASM.md` (Blitz `main`): Blitz can run in the browser on `wasm32-unknown-unknown` via `trunk`, using
`anyrender_vello_hybrid` with the `webgl` feature; there are no system fonts in the browser, so examples
bundle DejaVu Sans. Examples: `wasm_hello`, `seven_guis`, `todomvc`. This is a 0.3-line capability
`[unverified for 0.7.x]`. Do not confuse it with the normal Dioxus web renderer (DOM via web-sys), which
does not use Blitz.

## Differences from the webview renderer

- **No JavaScript.** `document::eval` never runs the script on native, so JS-dependent pages, libraries and
  components cannot work. Verified detail and the dioxus-components consequences: `limitations.md`.
- No `wry` APIs (`use_window` desktop config, webview devtools) `[inferred]`.
- Styling and layout come from Stylo and Taffy, not a browser engine: verify each CSS feature
  (`css-support.md`).
- `asset!` resolves through `dioxus-asset-resolver` built with its `native` feature.
- Accessibility goes through AccessKit rather than the browser's accessibility tree.
