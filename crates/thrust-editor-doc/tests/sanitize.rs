use serde_json::Value;
use thrust_editor_doc::{
    domparse,
    pm::norm_num,
    sanitize,
    schema::{Schema, Variant},
};

fn load() -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/corpus.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// Does Plane's allow-list change what the editor would load? parse(html) vs parse(sanitise(html)) for every corpus HTML.
#[test]
fn allow_list_preservation_of_editor_html() {
    let c = load();
    let s = sanitize::Sanitizer::default();
    let hardened = sanitize::hardened_builder();
    let (mut n, mut same, mut hard_same) = (0, 0, 0);
    let mut changed: Vec<String> = vec![];
    let mut causes: std::collections::BTreeMap<String, usize> = Default::default();
    for d in c["docs"].as_array().unwrap() {
        let Some(h) = d
            .get("html_in")
            .and_then(|x| x.as_str())
            .or_else(|| d.get("html").and_then(|x| x.as_str()))
        else {
            continue;
        };
        if h.is_empty() || d["kind"] == "foreign" {
            continue; // foreign html is deliberately hostile/odd: measured separately by the XSS corpus
        }
        let variant = if d["variant"] == "rich" {
            Variant::Rich
        } else {
            Variant::Document
        };
        let schema = Schema::new(variant);
        let before = domparse::html_to_doc(&schema, h).to_json(&schema);
        let clean = s.clean(h);
        let after = domparse::html_to_doc(&schema, &clean).to_json(&schema);
        n += 1;
        if norm_num(before.clone()) == norm_num(after.clone()) {
            same += 1;
        } else {
            changed.push(d["id"].as_str().unwrap().to_string());
            *causes.entry(diff_kind(&before, &after)).or_default() += 1;
        }
        let hclean = hardened.clean(h).to_string();
        if hclean == clean {
            hard_same += 1;
        }
    }
    eprintln!("sanitiser preserves the parsed document for {same}/{n} editor-generated HTML documents; hardened output byte-identical to plain for {hard_same}/{n}");
    eprintln!(
        "documents whose parsed content changes through Plane's allow-list: {}",
        changed.len()
    );
    eprintln!("first differing construct per changed document (kind -> count):");
    for (k, v) in &causes {
        eprintln!("  {v:3}  {k}");
    }
}

/// classify the first difference: which node/mark type and attribute, and what it became
fn diff_kind(a: &Value, b: &Value) -> String {
    fn walk(a: &Value, b: &Value, ctx: &str) -> Option<String> {
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                let ty = x.get("type").and_then(|t| t.as_str()).unwrap_or(ctx);
                for k in x.keys().chain(y.keys()) {
                    match (x.get(k), y.get(k)) {
                        (Some(p), Some(q)) if norm_num(p.clone()) != norm_num(q.clone()) => {
                            if p.is_object() || p.is_array() {
                                if let Some(r) = walk(p, q, if k == "attrs" { ty } else { k }) {
                                    return Some(r);
                                }
                            } else {
                                return Some(format!(
                                    "{ctx}.{k}: {} -> {}",
                                    p.to_string().chars().take(40).collect::<String>(),
                                    q.to_string().chars().take(40).collect::<String>()
                                ));
                            }
                        }
                        (Some(p), None) => {
                            return Some(format!(
                                "{ty}: `{k}` lost ({})",
                                p.to_string().chars().take(30).collect::<String>()
                            ))
                        }
                        (None, Some(_)) => return Some(format!("{ty}: `{k}` added")),
                        _ => {}
                    }
                }
                None
            }
            (Value::Array(x), Value::Array(y)) => {
                for i in 0..x.len().max(y.len()) {
                    match (x.get(i), y.get(i)) {
                        (Some(p), Some(q)) if norm_num(p.clone()) != norm_num(q.clone()) => {
                            if let Some(r) = walk(p, q, ctx) {
                                return Some(r);
                            }
                        }
                        (Some(p), None) => return Some(format!("{ctx}: child {} lost", p["type"])),
                        (None, Some(q)) => {
                            return Some(format!("{ctx}: child {} added", q["type"]))
                        }
                        _ => {}
                    }
                }
                None
            }
            _ => None,
        }
    }
    walk(a, b, "doc").unwrap_or_else(|| "?".into())
}

/// The API v1 write path end to end: raw html -> sanitise (Rust ammonia, latest 4.x) -> parse -> yjs -> json/html,
/// against Plane's nh3 output run through the real conversion steps (tools/spike-0006/oracle/convert-sanitised.ts).
#[test]
fn v1_write_with_sanitiser_matches_python_plus_js() {
    use thrust_editor_doc::{html, ydoc};
    let base = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/spike-0006");
    let Ok(s) = std::fs::read_to_string(format!("{base}/sanitised-converted.json")) else {
        eprintln!("no sanitised-converted.json (run tools/spike-0006/sanitize/run-all.sh and the convert-sanitised oracle)");
        return;
    };
    let oracle: Value = serde_json::from_str(&s).unwrap();
    let xss: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/corpus/xss-corpus.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let corpus = load();
    let mut inputs: Vec<(String, String)> = xss
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["id"].as_str().unwrap().into(),
                p["html"].as_str().unwrap().into(),
            )
        })
        .collect();
    for d in corpus["docs"].as_array().unwrap() {
        if let Some(h) = d
            .get("html_in")
            .and_then(|x| x.as_str())
            .or_else(|| d.get("html").and_then(|x| x.as_str()))
        {
            if !h.is_empty() {
                inputs.push((format!("doc:{}", d["id"].as_str().unwrap()), h.into()));
            }
        }
    }
    let san = sanitize::Sanitizer::default();
    let (mut ok, mut n) = (0, 0);
    let mut fails = vec![];
    for (k, h) in inputs {
        let Some(want) = oracle.get(&k) else { continue };
        if want.get("error").is_some() {
            continue;
        }
        let variant = if want["variant"] == "rich" {
            Variant::Rich
        } else {
            Variant::Document
        };
        let schema = Schema::new(variant);
        let clean = san.clean(&h);
        let doc = domparse::html_to_doc(&schema, &clean);
        let bin = ydoc::encode(&schema, &doc, None, Some(5));
        let back = ydoc::decode(&schema, &bin, "default").unwrap();
        n += 1;
        if norm_num(back.to_json(&schema)) == norm_num(want["json"].clone())
            && html::to_html(&schema, &back) == want["html"].as_str().unwrap()
        {
            ok += 1;
        } else if fails.len() < 12 {
            fails.push(k);
        }
    }
    eprintln!("v1 write path with sanitiser, Rust vs (nh3 + real JS conversion): {ok}/{n} identical; differing: {fails:?}");
    // the only differences are svg/math payloads that ammonia 4.2.x removes entirely and 4.0.0 (inside nh3) leaves behind
    for f in &fails {
        assert!(
            ["owasp-tag-080", "owasp-tag-083", "mxss-118"].contains(&f.as_str()),
            "unexpected difference: {f}"
        );
    }
}

#[test]
fn validate_html_content_semantics() {
    assert_eq!(sanitize::validate_html_content(""), Ok(None));
    let big = "a".repeat(sanitize::MAX_SIZE + 1);
    assert_eq!(
        sanitize::validate_html_content(&big),
        Err(sanitize::SanitizeError::TooLarge)
    );
    assert!(sanitize::validate_html_content("<p>x</p>")
        .unwrap()
        .unwrap()
        .contains("<p>x</p>"));
    assert!(sanitize::validate_binary_data(b"<script>alert(1)</script> padding padding").is_err());
    assert!(sanitize::validate_binary_data(&[1, 2]).is_err());
    assert!(sanitize::validate_binary_data(&[1, 2, 3, 4, 5]).is_ok());
    assert!(sanitize::validate_binary_data(&[]).is_ok());
}

/// must-fail control for the harness itself: an unsanitised payload is not allowed to survive the check
#[test]
fn unsanitised_payload_is_caught() {
    let s = sanitize::Sanitizer::default();
    for p in [
        "<img src=x onerror=alert(1)>",
        "<script>alert(1)</script>",
        "<a href=\"javascript:alert(1)\">x</a>",
    ] {
        let out = s.clean(p);
        assert!(
            !out.contains("onerror") && !out.contains("<script") && !out.contains("javascript:"),
            "sanitiser let through: {out}"
        );
        assert!(
            p.contains("onerror") || p.contains("<script") || p.contains("javascript:"),
            "control payload must be dangerous as given"
        );
    }
}
