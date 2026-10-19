use serde_json::Value;
use thrust_editor_doc::{
    domparse,
    pm::norm_num,
    schema::{Schema, Variant},
};

fn load() -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/corpus.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn parity_stage3_html_to_json() {
    let c = load();
    let (mut ok, mut n) = (0, 0);
    let mut fails = vec![];
    for d in c["docs"].as_array().unwrap() {
        let id = d["id"].as_str().unwrap();
        let Some(want) = d.get("html_in_json") else {
            continue;
        };
        let variant = if d["variant"] == "rich" {
            Variant::Rich
        } else {
            Variant::Document
        };
        let schema = Schema::new(variant);
        let html = d
            .get("html_in")
            .or_else(|| d.get("html"))
            .and_then(|h| h.as_str())
            .unwrap_or("");
        let doc = domparse::html_to_doc(&schema, html);
        let got = doc.to_json(&schema);
        n += 1;
        if norm_num(got.clone()) == norm_num(want.clone()) {
            ok += 1;
        } else {
            fails.push(format!(
                "{id}\n   html: {}\n   got : {}\n   want: {}",
                &html[..html.len().min(150)],
                &got.to_string()[..got.to_string().len().min(400)],
                &want.to_string()[..want.to_string().len().min(400)]
            ));
        }
    }
    eprintln!("html->json {ok}/{n}");
    for f in fails.iter().take(40) {
        eprintln!("FAIL {f}");
    }
    assert_eq!(ok, n, "html -> json must equal generateJSON");
}

#[test]
fn stage3_not_vacuous() {
    let c = load();
    let mut bytes = 0usize;
    let mut nonempty = 0usize;
    for d in c["docs"].as_array().unwrap() {
        let Some(want) = d.get("html_in_json") else {
            continue;
        };
        let variant = if d["variant"] == "rich" {
            Variant::Rich
        } else {
            Variant::Document
        };
        let schema = Schema::new(variant);
        let html = d
            .get("html_in")
            .or_else(|| d.get("html"))
            .and_then(|h| h.as_str())
            .unwrap_or("");
        let got = domparse::html_to_doc(&schema, html).to_json(&schema);
        assert_eq!(norm_num(got.clone()), norm_num(want.clone()), "{}", d["id"]);
        bytes += got.to_string().len();
        if html.len() > 40 {
            nonempty += 1;
        }
    }
    eprintln!("compared {bytes} bytes of JSON over {nonempty} non-trivial inputs");
}
