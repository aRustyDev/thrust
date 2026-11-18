//! Spike 0006 — Option A: the Plane editor document model in Rust.
//!
//! ProseMirror JSON <-> Yjs update (y-prosemirror XmlFragment encoding, via `yrs`) <-> HTML, for the node and
//! mark set of `@plane/editor`, plus the HTML sanitiser (`sanitize`, a re-export of `thrust-html`). Everything here builds for native and wasm32.

pub mod domparse;
pub mod html;
pub mod pm;
#[cfg(feature = "sanitize")]
pub use thrust_html as sanitize;
pub mod schema;
pub mod vdom;
#[cfg(feature = "yjs")]
pub mod ydoc;
