//! Runs the Rust sanitiser over the XSS corpus and the editor-HTML corpus and writes the outputs for the Python checker.
//!
//!   scripts/rs cargo run --release -p thrust-editor-doc --example sanitize_check -- <xss-corpus.json> <corpus.json> <out.json>
//!
//! This build links the latest ammonia 4.x; probes/ammonia-400 compiles the same source against 4.0.0 (the version
//! inside nh3 0.2.18) so the effect of version drift can be measured.
use serde_json::{json, Map, Value};
use thrust_editor_doc::sanitize;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let xss: Value = serde_json::from_str(&std::fs::read_to_string(&args[1]).unwrap()).unwrap();
    let corpus: Value = serde_json::from_str(&std::fs::read_to_string(&args[2]).unwrap()).unwrap();
    let mut out = Map::new();
    let s = sanitize::Sanitizer::default();
    let mut inputs: Vec<(String, String)> = xss
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["id"].as_str().unwrap().to_string(),
                p["html"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    for d in corpus["docs"].as_array().unwrap() {
        if let Some(h) = d
            .get("html_in")
            .and_then(|h| h.as_str())
            .or_else(|| d.get("html").and_then(|h| h.as_str()))
        {
            if !h.is_empty() {
                inputs.push((format!("doc:{}", d["id"].as_str().unwrap()), h.to_string()));
            }
        }
    }
    let hard = sanitize::hardened_builder();
    let t0 = std::time::Instant::now();
    for (id, html) in &inputs {
        let clean = s.clean(html);
        let stripped = sanitize::strip_tags(&clean);
        out.insert(
            id.clone(),
            json!({"clean": clean, "stripped": stripped, "hardened": hard.clean(html).to_string()}),
        );
    }
    eprintln!(
        "sanitised {} inputs in {:?} (ammonia latest 4.x; the 4.0.0 build is probes/ammonia-400)",
        inputs.len(),
        t0.elapsed()
    );
    std::fs::write(
        &args[3],
        serde_json::to_string(&Value::Object(out)).unwrap(),
    )
    .unwrap();
}
