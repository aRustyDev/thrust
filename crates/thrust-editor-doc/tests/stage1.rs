use base64::Engine;
use serde_json::Value;
use thrust_editor_doc::{pm::Node, schema::{Schema, Variant}, ydoc};

fn load() -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/corpus.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}
fn b64(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD.decode(s).unwrap()
}
fn eq(a: &Value, b: &Value) -> bool {
    thrust_editor_doc::pm::norm_num(a.clone()) == thrust_editor_doc::pm::norm_num(b.clone())
}

#[test]
fn stage1_probe() {
    let c = load();
    let docs = c["docs"].as_array().unwrap();
    let (mut dec_ok, mut dec_n, mut enc_ok, mut enc_n, mut st_ok) = (0, 0, 0, 0, 0);
    let mut fails = vec![];
    for d in docs {
        let id = d["id"].as_str().unwrap();
        if d.get("binary").is_none() || d["kind"] == "foreign" && d.get("json").is_none() {
            continue;
        }
        let variant = if d["variant"] == "rich" { Variant::Rich } else { Variant::Document };
        let schema = Schema::new(variant);
        let bin = b64(d["binary"].as_str().unwrap());
        dec_n += 1;
        match ydoc::decode(&schema, &bin, "default") {
            Ok(n) => {
                let j = n.to_json(&schema);
                if eq(&j, &d["json"]) {
                    dec_ok += 1;
                } else {
                    fails.push(format!("decode {id}: json differs"));
                }
            }
            Err(e) => fails.push(format!("decode {id}: {e}")),
        }
        // encode from the oracle json, then compare structure
        enc_n += 1;
        let src = d.get("authored").unwrap_or(&d["json"]);
        let node = Node::from_json(&schema, src).unwrap();
        let out = ydoc::encode(&schema, &node, None, Some(4242));
        match ydoc::decode(&schema, &out, "default") {
            Ok(n2) if eq(&n2.to_json(&schema), &d["json"]) => enc_ok += 1,
            Ok(_) => fails.push(format!("roundtrip {id}: json differs")),
            Err(e) => fails.push(format!("roundtrip {id}: {e}")),
        }
        if d["kind"] != "history" {
            if let (Ok(s1), Some(s2)) = (ydoc::struct_dump(&out, "default"), d.get("ystruct")) {
                if eq(&s1, s2) {
                    st_ok += 1;
                } else if fails.len() < 40 {
                    fails.push(format!("ystruct {id}: differs"));
                }
            }
        }
    }
    eprintln!("decode {dec_ok}/{dec_n}  json->bin->json {enc_ok}/{enc_n}  ystruct-equal {st_ok}");
    for f in fails.iter().take(40) {
        eprintln!("FAIL {f}");
    }
}
