use serde_json::Value;
use thrust_editor_doc::{
    html,
    pm::Node,
    schema::{Schema, Variant},
};

fn load() -> Value {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/corpus.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn parity_stage2_json_to_html() {
    let c = load();
    let (mut ok, mut n) = (0, 0);
    let mut fails = vec![];
    for d in c["docs"].as_array().unwrap() {
        let id = d["id"].as_str().unwrap();
        if d.get("json").is_none() {
            continue;
        }
        let variant = if d["variant"] == "rich" {
            Variant::Rich
        } else {
            Variant::Document
        };
        let schema = Schema::new(variant);
        let node = Node::from_json(&schema, &d["json"]).unwrap();
        let got = html::to_html(&schema, &node);
        n += 1;
        let want = d["html"].as_str().unwrap();
        if got == want {
            ok += 1;
        } else {
            let at = got
                .bytes()
                .zip(want.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(got.len().min(want.len()));
            let lo = at.saturating_sub(60);
            fails.push(format!(
                "{id}: first diff at byte {at}\n   got : {}\n   want: {}",
                &got[lo.min(got.len())..(at + 80).min(got.len())],
                &want[lo.min(want.len())..(at + 80).min(want.len())]
            ));
        }
    }
    eprintln!("json->html {ok}/{n}");
    for f in fails.iter().take(25) {
        eprintln!("FAIL {f}");
    }
    assert_eq!(ok, n, "json -> html must be byte-identical to generateHTML");
}
