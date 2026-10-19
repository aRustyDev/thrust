//! Spike 0006 Option A in workerd: the conversions as a plain Worker, no bindings.
//!
//! workerd freezes `Date.now()`/`performance.now()` during synchronous work, so CPU is measured from outside:
//! every route repeats the operation `n` times inside ONE request and the client divides the request time by `n`
//! (route `/noop` gives the fixed per-request cost to subtract).
//!
//!   GET  /health
//!   POST /noop?n=K                                body ignored
//!   POST /html-to-all?variant=rich|document&n=K    body = html            -> sanitise, parse, encode, decode, render (the API v1 write path)
//!   POST /binary-to-all?variant=...&n=K            body = base64 binary   -> decode, to json, to html (the collab projection)
//!   POST /json-to-binary?variant=...&n=K           body = ProseMirror JSON -> binary
//!   POST /sanitize?n=K&hardened=0|1                body = html
//!   add `&full=1` to get the last result's bytes back instead of just sizes.

use base64::Engine;
use serde_json::json;
use thrust_editor_doc::{
    domparse, html,
    pm::Node,
    schema::{Schema, Variant},
    ydoc,
};
use worker::*;

// `--no-default-features` builds the conversions without the sanitiser so the size of ammonia/html5ever is measurable
#[cfg(feature = "sanitize")]
mod sanitize {
    pub use thrust_editor_doc::sanitize::*;
}
#[cfg(not(feature = "sanitize"))]
mod sanitize {
    #[derive(Default)]
    pub struct Sanitizer;
    impl Sanitizer {
        pub fn clean(&self, h: &str) -> String {
            h.to_string()
        }
    }
    pub fn strip_tags(s: &str) -> String {
        s.to_string()
    }
}

fn variant(url: &Url) -> Variant {
    match url
        .query_pairs()
        .find(|(k, _)| k == "variant")
        .map(|(_, v)| v.to_string())
        .as_deref()
    {
        Some("document") => Variant::Document,
        _ => Variant::Rich,
    }
}
fn q(url: &Url, k: &str) -> Option<String> {
    url.query_pairs()
        .find(|(kk, _)| kk == k)
        .map(|(_, v)| v.to_string())
}

#[event(fetch)]
pub async fn main(mut req: Request, _env: Env, _ctx: Context) -> Result<Response> {
    console_error_panic_hook::set_once();
    let url = req.url()?;
    let n: usize = q(&url, "n").and_then(|v| v.parse().ok()).unwrap_or(1);
    let full = q(&url, "full").as_deref() == Some("1");
    match (req.method(), url.path()) {
        (Method::Get, "/health") => Response::ok("ok"),
        (Method::Post, "/noop") => Response::ok("noop"),
        (Method::Post, "/html-to-all") => {
            let body = req.text().await?;
            let schema = Schema::new(variant(&url));
            let san = sanitize::Sanitizer::default();
            // `lean=1`: only what convertHTMLDocumentToAllFormats does (no sanitise, no strip_tags), for a like-for-like
            // comparison with the JS Worker
            let lean = q(&url, "lean").as_deref() == Some("1");
            let mut last = (String::new(), String::new(), Vec::new(), String::new());
            for _ in 0..n {
                let clean = if lean { body.clone() } else { san.clean(&body) };
                let doc = domparse::html_to_doc(&schema, &clean);
                let bin = ydoc::encode(&schema, &doc, None, Some(1));
                let back = ydoc::decode(&schema, &bin, "default")
                    .map_err(|e| Error::RustError(e.to_string()))?;
                let h = html::to_html(&schema, &back);
                let stripped = if lean {
                    String::new()
                } else {
                    sanitize::strip_tags(&h)
                };
                last = (back.to_json(&schema).to_string(), h, bin, stripped);
            }
            if full {
                Response::from_json(
                    &json!({"json": last.0, "html": last.1, "binary": base64::engine::general_purpose::STANDARD.encode(&last.2), "stripped": last.3}),
                )
            } else {
                Response::from_json(
                    &json!({"json_bytes": last.0.len(), "html_bytes": last.1.len(), "binary_bytes": last.2.len(), "stripped_bytes": last.3.len(), "n": n}),
                )
            }
        }
        // nesting-depth probe: builds `<shape>`^depth around a paragraph and runs the whole v1 write path
        (Method::Post, "/deep") => {
            let depth: usize = q(&url, "depth").and_then(|v| v.parse().ok()).unwrap_or(10);
            let shape = q(&url, "shape").unwrap_or_else(|| "blockquote".into());
            let body = match shape.as_str() {
                "list" => format!(
                    "{}x{}",
                    "<ul><li>".repeat(depth),
                    "</li></ul>".repeat(depth)
                ),
                "div" => format!(
                    "{}<p>x</p>{}",
                    "<div>".repeat(depth),
                    "</div>".repeat(depth)
                ),
                "inline" => format!(
                    "<p>{}x{}</p>",
                    "<strong><em>".repeat(depth),
                    "</em></strong>".repeat(depth)
                ),
                _ => format!(
                    "{}<p>x</p>{}",
                    "<blockquote>".repeat(depth),
                    "</blockquote>".repeat(depth)
                ),
            };
            let schema = Schema::new(Variant::Rich);
            let clean = sanitize::Sanitizer::default().clean(&body);
            let doc = domparse::html_to_doc(&schema, &clean);
            let bin = ydoc::encode(&schema, &doc, None, Some(1));
            let back = ydoc::decode(&schema, &bin, "default")
                .map_err(|e| Error::RustError(e.to_string()))?;
            let h = html::to_html(&schema, &back);
            let j = back.to_json(&schema);
            Response::from_json(
                &json!({"ok": true, "html_bytes": h.len(), "binary_bytes": bin.len(), "json_depth_probe": j.to_string().len()}),
            )
        }
        (Method::Post, "/binary-to-all") => {
            let body = req.text().await?;
            let bin = base64::engine::general_purpose::STANDARD
                .decode(body.trim())
                .map_err(|e| Error::RustError(e.to_string()))?;
            let schema = Schema::new(variant(&url));
            let mut last = (String::new(), String::new());
            for _ in 0..n {
                let doc = ydoc::decode(&schema, &bin, "default")
                    .map_err(|e| Error::RustError(e.to_string()))?;
                last = (
                    doc.to_json(&schema).to_string(),
                    html::to_html(&schema, &doc),
                );
            }
            if full {
                Response::from_json(&json!({"json": last.0, "html": last.1}))
            } else {
                Response::from_json(
                    &json!({"json_bytes": last.0.len(), "html_bytes": last.1.len(), "n": n}),
                )
            }
        }
        (Method::Post, "/json-to-binary") => {
            let body = req.text().await?;
            let schema = Schema::new(variant(&url));
            let v: serde_json::Value =
                serde_json::from_str(&body).map_err(|e| Error::RustError(e.to_string()))?;
            let mut last = Vec::new();
            for _ in 0..n {
                let node =
                    Node::from_json(&schema, &v).map_err(|e| Error::RustError(e.to_string()))?;
                last = ydoc::encode(&schema, &node, None, Some(1));
            }
            if full {
                Response::ok(base64::engine::general_purpose::STANDARD.encode(&last))
            } else {
                Response::from_json(&json!({"binary_bytes": last.len(), "n": n}))
            }
        }
        #[cfg(feature = "sanitize")]
        (Method::Post, "/sanitize") => {
            let body = req.text().await?;
            let hardened = q(&url, "hardened").as_deref() == Some("1");
            let b = if hardened {
                sanitize::hardened_builder()
            } else {
                sanitize::plane_builder()
            };
            let mut last = String::new();
            for _ in 0..n {
                last = b.clean(&body).to_string();
            }
            if full {
                Response::ok(last)
            } else {
                Response::from_json(&json!({"clean_bytes": last.len(), "n": n}))
            }
        }
        _ => Response::error("not found", 404),
    }
}
