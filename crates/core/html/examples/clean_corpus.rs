//! What the default sanitiser makes of every input of the committed corpora: the strings that `strip_tags` is given in production (a
//! description is cleaned before its text is taken), written for tools/strip-tags/record.py to record Django's answer for.
//!
//!   scripts/rs cargo run -p thrust-html --example clean_corpus -- <out.json>
//!
//! The output is a JSON array of `{"from": "xss:<id>" | "doc:<id>", "in": <the cleaned HTML>}`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "an example, not product code"
)]
use serde_json::{json, Value};
use thrust_html::Sanitizer;

fn read(path: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"))).unwrap()
}

fn main() {
    let out = std::env::args().nth(1).expect("usage: clean_corpus <out.json>");
    let xss = read(concat!(env!("THRUST_ROOT"), "/crates/editor/doc/tests/corpus/xss-corpus.json"));
    let docs = read(concat!(env!("THRUST_ROOT"), "/crates/editor/doc/tests/corpus/corpus.json"));
    let sanitiser = Sanitizer::default();
    let mut cases = Vec::new();
    for p in xss.as_array().unwrap() {
        cases.push(json!({"from": format!("xss:{}", p["id"].as_str().unwrap()), "in": sanitiser.clean(p["html"].as_str().unwrap())}));
    }
    for d in docs["docs"].as_array().unwrap() {
        // as the recorder and the corpus tests read a document: `html_in` when it has one, else `html`
        let html = d["html_in"].as_str().filter(|h| !h.is_empty()).or_else(|| d["html"].as_str()).unwrap_or_default();
        if !html.is_empty() {
            cases.push(json!({"from": format!("doc:{}", d["id"].as_str().unwrap()), "in": sanitiser.clean(html)}));
        }
    }
    std::fs::write(&out, serde_json::to_string(&Value::Array(cases)).unwrap()).unwrap();
    eprintln!("wrote {out}");
}
