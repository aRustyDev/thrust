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

/// The editor-generated HTML documents of the corpus (not the `foreign` ones, which are deliberately hostile or odd and are
/// measured by the XSS corpus), with the variant of the schema each is read with.
fn editor_html() -> Vec<(String, Variant, String)> {
    let c = load();
    c["docs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| {
            let h = d.get("html_in").and_then(|x| x.as_str()).or_else(|| d.get("html").and_then(|x| x.as_str()))?;
            if h.is_empty() || d["kind"] == "foreign" {
                return None;
            }
            let variant = if d["variant"] == "rich" { Variant::Rich } else { Variant::Document };
            Some((d["id"].as_str().unwrap().to_string(), variant, h.to_string()))
        })
        .collect()
}

/// What a sanitiser does to the editor's own HTML: parse(html) against parse(clean(html)) for every document of `editor_html`.
struct Preservation {
    total: usize,
    changed: Vec<String>,
    /// the first differing construct of each changed document (`diff_kind`), counted
    causes: std::collections::BTreeMap<String, usize>,
}

fn preservation(clean: impl Fn(&str) -> String) -> Preservation {
    let mut p = Preservation { total: 0, changed: vec![], causes: Default::default() };
    for (id, variant, h) in editor_html() {
        let schema = Schema::new(variant);
        let before = domparse::html_to_doc(&schema, &h).to_json(&schema);
        let after = domparse::html_to_doc(&schema, &clean(&h)).to_json(&schema);
        p.total += 1;
        if norm_num(before.clone()) != norm_num(after.clone()) {
            p.changed.push(id);
            *p.causes.entry(diff_kind(&before, &after)).or_default() += 1;
        }
    }
    p
}

/// How many editor-generated HTML documents the corpus holds. Issue #48 and `research/0008` §7 say 195 and 129 unchanged; the corpus
/// (byte-identical to the one the spike measured) has 196 and 130 unchanged. The 66 that Plane's list changes, and why (5 and 61),
/// agree; only the spike's total is one short. A corpus that grows or shrinks fails here, so the other counts are read afresh.
const EDITOR_DOCUMENTS: usize = 196;

/// Does the default allow-list change what the editor would load? It must not, for any of the documents the editor generated
/// (issue #48, D-017 and D-018: with `issue-embed-component` and `rel` allowed, nothing Plane's list strips is left to strip).
#[test]
fn allow_list_preservation_of_editor_html() {
    let s = sanitize::Sanitizer::default();
    let p = preservation(|h| s.clean(h));
    assert_eq!(p.total, EDITOR_DOCUMENTS, "the corpus has {} editor-generated HTML documents", p.total);
    assert!(
        p.changed.is_empty(),
        "the default allow-list changes {} of {} editor documents: {:?}; first difference per document: {:?}",
        p.changed.len(),
        p.total,
        p.changed,
        p.causes
    );
}

/// The control of the test above: Plane's own allow-list changes 66 of the same documents, for two reasons only, 5 that lose their
/// `issue-embed-component` and 61 whose `rel` loses `nofollow` (`research/0008` §7, D-017, D-018). If this stopped being true, the
/// test above would be passing on a corpus that Plane's list does not hurt.
#[test]
fn plane_allow_list_changes_66_of_the_editor_documents_for_two_reasons() {
    let plane = sanitize::Sanitizer::new(sanitize::plane_builder());
    let p = preservation(|h| plane.clean(h));
    assert_eq!(p.total, EDITOR_DOCUMENTS);
    assert_eq!(p.total - p.changed.len(), 130, "changed: {:?}; causes: {:?}", p.changed, p.causes);
    let cause = |pat: &dyn Fn(&str) -> bool| p.causes.iter().filter(|(k, _)| pat(k)).map(|(_, n)| n).sum::<usize>();
    let embeds = cause(&|k| k.contains("issue-embed-component"));
    let rels = cause(&|k| k.starts_with("link.rel"));
    assert_eq!((embeds, rels, embeds + rels), (5, 61, 66), "causes: {:?}", p.causes);
}

/// D-016's other half: the value filters take nothing from the editor's own HTML. The hardened builder's output is byte-identical to
/// Plane's on every one of the documents.
#[test]
fn hardened_builder_changes_none_of_the_editor_html() {
    let (hardened, plane) = (sanitize::hardened_builder(), sanitize::plane_builder());
    let docs = editor_html();
    assert_eq!(docs.len(), EDITOR_DOCUMENTS);
    for (id, _, h) in docs {
        assert_eq!(hardened.clean(&h).to_string(), plane.clean(&h).to_string(), "{id}");
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
                        (None, Some(q)) => return Some(format!("{ctx}: child {} added", q["type"])),
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
/// `oracle_`: the input is generated by the Python and TypeScript oracles and git-ignored, so this runs only under the
/// `oracle` and `deep` nextest profiles, and fails when the input is missing rather than passing with nothing compared.
#[test]
fn oracle_v1_write_with_sanitiser_matches_python_plus_js() {
    use thrust_editor_doc::{html, ydoc};
    let p = concat!(env!("THRUST_ROOT"), "/target/spike-0006/sanitised-converted.json");
    let s = std::fs::read_to_string(p).unwrap_or_else(|e| {
        panic!("{p}: {e} (generate it with tools/spike-0006/sanitize/run-all.sh, then the convert-sanitised oracle)")
    });
    let oracle: Value = serde_json::from_str(&s).unwrap();
    let xss: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/xss-corpus.json")).unwrap(),
    )
    .unwrap();
    let corpus = load();
    let mut inputs: Vec<(String, String)> = xss
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["id"].as_str().unwrap().into(), p["html"].as_str().unwrap().into()))
        .collect();
    for d in corpus["docs"].as_array().unwrap() {
        if let Some(h) = d.get("html_in").and_then(|x| x.as_str()).or_else(|| d.get("html").and_then(|x| x.as_str())) {
            if !h.is_empty() {
                inputs.push((format!("doc:{}", d["id"].as_str().unwrap()), h.into()));
            }
        }
    }
    // Plane's own builder, not the default: nh3 is the oracle here, and the default keeps `rel` and `issue-embed-component` and
    // filters values, which nh3 does not (D-016 to D-018). Left on the default this test would fail on the 66 documents those change.
    let san = sanitize::Sanitizer::new(sanitize::plane_builder());
    let (mut ok, mut n) = (0, 0);
    let mut fails = vec![];
    for (k, h) in inputs {
        let Some(want) = oracle.get(&k) else { continue };
        if want.get("error").is_some() {
            continue;
        }
        let variant = if want["variant"] == "rich" { Variant::Rich } else { Variant::Document };
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
    eprintln!(
        "v1 write path with sanitiser, Rust vs (nh3 + real JS conversion): {ok}/{n} identical; differing: {fails:?}"
    );
    assert!(n > 0, "{p} matched no inputs: nothing was compared");
    // the only differences are svg/math payloads that ammonia 4.2.x removes entirely and 4.0.0 (inside nh3) leaves behind
    for f in &fails {
        assert!(["owasp-tag-080", "owasp-tag-083", "mxss-118"].contains(&f.as_str()), "unexpected difference: {f}");
    }
}

// `validate_html_content_semantics` and `unsanitised_payload_is_caught` moved to `crates/core/html/tests/sanitize.rs` with the
// sanitiser (issue #48); what stays here needs the editor's schema or the nh3 oracle.
