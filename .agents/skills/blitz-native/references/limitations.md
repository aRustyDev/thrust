# Known limitations of Blitz and dioxus-native

Evidence tags: `README` = Blitz README; `roadmap` = Blitz issue #119; `source` = read in the `v0.7.10`
`DioxusLabs/dioxus` tag or the dioxus-components repository; `issue` = a GitHub issue that was open when
read (2026-10-18; it may be fixed in a newer release, and may have been filed against 0.7.0-era builds);
`inferred` = follows from the architecture; `[unverified]` = could not be checked.

## Status

- Blitz describes itself as **beta**: it "can already render many popular no-JS websites (Wikipedia,
  (Old) Reddit, etc)", is "usable for making apps if you are an early adopter and willing to live on the
  bleeding edge", and has "many bugs and missing features" (`README`).
- `dioxus-native` is the Dioxus renderer built on it. The Dioxus desktop docs page describes the default
  desktop renderer as webview (wry) and Blitz as the planned direction (Dioxus 0.7 docs, desktop page).
  Treat native as an opt-in renderer, not the default.

## No JavaScript (by design and in source)

- Blitz does **not** provide webrtc, websockets, bluetooth or localstorage; "in a native app, much of this
  functionality can be fulfilled using regular Rust crates" (`README`). JavaScript support is in the
  roadmap **backlog** with no schedule (`roadmap`).
- **`document::eval` never runs the script on native** (`source`). `DioxusNativeDocument::eval`
  (`packages/native/src/contexts.rs`, 0.7.10) delegates to `NoOpDocument::eval`
  (`packages/document/src/document.rs`), whose evaluator returns `Err(EvalError::Unsupported)` from `send`,
  `poll_recv` and `poll_join`.
- **The dioxus-components Dialog survives only because it ignores those results** (`source`,
  `primitives/src/dialog.rs` and `primitives/src/lib.rs` in `DioxusLabs/dioxus-components`, `main` at commit
  `9a75825`). Its focus trap (`window.createFocusTrap`), escape-to-close (`use_global_escape_listener`),
  outside-dismiss (`use_outside_dismiss`) and exit animation (`use_animated_open`) are all built on
  `document::eval`, so on native they do nothing: focus is not trapped or restored, Escape and outside clicks
  do not close it, and the exit animation is skipped. Alert Dialog and Popover share these helpers; other
  JS-assisted primitives were not checked `[unverified]`.
- Consequences (`inferred` from the above): no JavaScript-dependent pages, libraries or components; browser
  storage and network APIs must be replaced with Rust equivalents.

## Rendering and layout gaps (roadmap; see `css-support.md` for the data)

Open for 1.0: `position: sticky`; native table layout (currently emulated with grid); `contain`;
`line-clamp`; `text-wrap-style`; reliable font fallback, `font-display`, `unicode-range`;
`background-clip: text`; text shadow; `mix-blend-mode`; Shadow DOM and custom elements; `<picture>` /
`srcset`; `<select>` dropdown, range input, colour picker; devtools. Backlog: 3D transforms, subgrid,
anchor positioning, multi-column, `writing-mode` and vertical text, ruby, first-party stylable SVG.
`text-overflow` is an unchecked item inside the 0.3 milestone.
Several items marked done are on the 0.3 line only (filters, masks, `clip-path`, floats, `position: fixed`,
`<iframe>`, text selection): the absence of these from a 0.7.x app (blitz 0.2) is inferred from milestone
placement, not tested.

## Input, events and accessibility

Verified in the 0.7.10 source (`packages/native-dom/src/dioxus_document.rs`): the handler that feeds Blitz
events into the VirtualDom converts only mouse move, mouse down, mouse up, click, key down, key up, key
press and input events; **IME events are dropped** (`DomEventData::Ime(_) => None`, with
`// TODO: Implement IME handling`). The match has no other arms, so other event kinds (focus, scroll,
wheel, pointer, touch, double-click and so on) are not forwarded to Dioxus handlers in that release
`[inferred from the exhaustive match; not run]`.

Roadmap (`roadmap`): unchecked inside the 0.3 milestone are `<body>` and `<html>` events, drag and drop, and
the Android app-icon item in the CLI. Open for 1.0: animation events and `beforeinput`; richer accessibility
tree, text-input accessibility and accessibility events. Clipboard events and composition / IME carry a
question mark upstream (not committed). Accessibility is wired through AccessKit (the `accessibility`
feature, on by default).

## Platform

- Drawing is Vello on wgpu (GPU). On 0.7.10 a CPU renderer is used only for the iOS simulator target
  (`cfg`). `[unverified: behaviour on machines without a usable GPU or driver]`.
- Windowing is winit 0.30 on 0.7.x, so winit-level platform behaviour applies.

## Issues seen upstream (open when read)

Examples of the kind of defect to expect, not a bug list:
- `DioxusLabs/dioxus` #5282: native renderer not updating a signal subscriber on a checkbox `checked`
  property (2026-02-04).
- #4951: `input` / `textarea` on macOS native cannot use custom fonts while buttons and labels can
  (2025-11-11).
- #4901: minimal Dioxus Native crashes immediately on Windows (2025-11-03).
- Recently closed, showing the churn: #5617 and #5620 (transforms applied wrongly), #5821 (`disabled: false`
  written as a present attribute), all 2026.

## Performance

Upstream states the 0.2 line added "incremental box tree construction (and other performance improvements)"
(`roadmap`), and `dioxus-native` / `blitz-dom` expose an `incremental` feature that is off by default. The
sources used contain **no benchmark, frame-time or memory data** `[unverified]`. Do not claim the native
renderer is faster or slower than the webview renderer; measure the app's real screens.

## Security notes (recommendations, not upstream statements)

- `blitz-net` can fetch resources over http, from the filesystem and from data URIs. If the app renders
  untrusted HTML or CSS, disable the `net` feature unless it is needed and review which URLs can load
  `[unverified: effect of disabling on all resource types]`.
- Pin a git `rev` if using the Blitz git version of `dioxus-native`.
- The transitive licence tree is not audited here.

## Answering "can we ship this on native?"

Say: it is beta; list the concrete CSS and element features the app uses; check each against
`css-support.md` at the pinned version; run a repro on every target OS and GPU class; keep the webview
renderer as the fallback until those checks pass.
