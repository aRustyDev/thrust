//! Spike 0006 — Option A: the Plane editor document model in Rust.
//!
//! ProseMirror JSON <-> Yjs update (y-prosemirror XmlFragment encoding, via `yrs`) <-> HTML, for the node and
//! mark set of `@plane/editor`, plus the HTML sanitiser. Everything here builds for native and wasm32.

pub mod domparse;
pub mod html;
pub mod pm;
#[cfg(feature = "sanitize")]
pub mod sanitize;
pub mod schema;
pub mod vdom;
pub mod ydoc;
