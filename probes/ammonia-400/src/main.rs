//! Same sanitiser source as crates/thrust-editor-doc/src/sanitize.rs, compiled against ammonia =4.0.0.
//!   probes/ammonia-400/run.sh   (writes target/spike-0006/xss-rust-400.json)
use serde_json::{json, Map, Value};

#[path = "../../../crates/thrust-editor-doc/src/sanitize.rs"]
mod sanitize;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let xss: Value = serde_json::from_str(&std::fs::read_to_string(&args[1]).unwrap()).unwrap();
    let corpus: Value = serde_json::from_str(&std::fs::read_to_string(&args[2]).unwrap()).unwrap();
    let mut out = Map::new();
    let s = sanitize::Sanitizer::default();
    let mut inputs: Vec<(String, String)> = xss.as_array().unwrap().iter().map(|p| (p["id"].as_str().unwrap().to_string(), p["html"].as_str().unwrap().to_string())).collect();
    for d in corpus["docs"].as_array().unwrap() {
        if let Some(h) = d.get("html_in").and_then(|h| h.as_str()).or_else(|| d.get("html").and_then(|h| h.as_str())) {
            if !h.is_empty() {
                inputs.push((format!("doc:{}", d["id"].as_str().unwrap()), h.to_string()));
            }
        }
    }
    for (id, html) in &inputs {
        let clean = s.clean(html);
        let stripped = sanitize::strip_tags(&clean);
        out.insert(id.clone(), json!({"clean": clean, "stripped": stripped}));
    }
    std::fs::write(&args[3], serde_json::to_string(&Value::Object(out)).unwrap()).unwrap();
}
